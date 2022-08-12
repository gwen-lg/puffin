use anyhow::Context as _;
use puffin::GlobalProfiler;
use std::{
    io::Write,
    net::{SocketAddr, TcpListener, TcpStream},
    ops::DerefMut,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, RwLock,
    },
};

/// Maximum size of the backlog of packets to send to a client if they aren't reading fast enough.
const MAX_FRAMES_IN_QUEUE: usize = 30;

/// Listens for incoming connections
/// and streams them puffin profiler data.
///
/// Drop to stop transmitting and listening for new connections.
pub struct Server {
    sink_id: puffin::FrameSinkId,
    send_handle: Option<std::thread::JoinHandle<()>>,
    num_clients: Arc<AtomicUsize>,
    recv_client_connect: crossbeam_channel::Receiver<()>,
}

impl Server {
    /// Start listening for connections on this addr (e.g. "0.0.0.0:8585")
    pub fn new(bind_addr: &str) -> anyhow::Result<Self> {
        let tcp_listener = TcpListener::bind(bind_addr).context("binding server TCP socket")?;

        let clients: Arc<RwLock<Vec<Client>>> = Default::default();
        let num_clients = Arc::new(AtomicUsize::default());
        let num_clients_cloned = num_clients.clone();

        let (sender_client_connect, recv_client_connect) = crossbeam_channel::bounded(1);
        let clients_cloned = clients.clone();
        std::thread::Builder::new()
            .name("ps-client-watcher".to_owned())
            .spawn(move || {
                if let Err(err) = accept_new_clients(
                    tcp_listener,
                    clients_cloned,
                    num_clients_cloned,
                    sender_client_connect,
                ) {
                    log::warn!("puffin server failure: {}", err);
                }
            })
            .context("Couldn't spawn puffin client watcher thread")?;

        // We use crossbeam_channel instead of `mpsc`,
        // because on shutdown we want all frames to be sent.
        // `mpsc::Receiver` stops receiving as soon as the `Sender` is dropped,
        // but `crossbeam_channel` will continue until the channel is empty.
        let (tx, rx): (crossbeam_channel::Sender<Arc<puffin::FrameData>>, _) =
            crossbeam_channel::unbounded();

        let num_clients_cloned = num_clients.clone();
        let send_handle = std::thread::Builder::new()
            .name("ps-send".to_owned())
            .spawn(move || {
                let mut server_impl = PuffinServerSend {
                    clients: clients,
                    num_clients: num_clients_cloned,
                };

                while let Ok(frame) = rx.recv() {
                    if let Err(err) = server_impl.send(&*frame) {
                        log::warn!("puffin server failure: {}", err);
                    }
                }
            })
            .context("Couldn't spawn thread")?;

        let sink_id = GlobalProfiler::lock().add_sink(Box::new(move |frame| {
            tx.send(frame).ok();
        }));

        Ok(Server {
            sink_id,
            send_handle: Some(send_handle),
            num_clients,
            recv_client_connect,
        })
    }

    /// Number of clients currently connected.
    pub fn num_clients(&self) -> usize {
        self.num_clients.load(Ordering::SeqCst)
    }

    /// Block thread to wait at least a puffin client.
    pub fn wait_client(&self) {
        while self.num_clients() == 0 {
            match self.recv_client_connect.recv() {
                Ok(()) => {}
                Err(err) => {
                    log::warn!("wait_client: {}", err);
                }
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        GlobalProfiler::lock().remove_sink(self.sink_id);

        // Take care to send everything before we shut down:
        if let Some(send_handle) = self.send_handle.take() {
            send_handle.join().ok();
        }
    }
}

type Packet = Arc<[u8]>;

struct Client {
    client_addr: SocketAddr,
    packet_tx: Option<crossbeam_channel::Sender<Packet>>,
    join_handle: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Client {
    fn drop(&mut self) {
        // Take care to send everything before we shut down!

        // Drop the sender to signal to shut down:
        self.packet_tx = None;

        // Wait for the shutdown:
        if let Some(join_handle) = self.join_handle.take() {
            join_handle.join().ok();
        }
    }
}

fn accept_new_clients(
    tcp_listener: TcpListener,
    clients: Arc<RwLock<Vec<Client>>>,
    num_clients: Arc<AtomicUsize>,
    recv_client_connect: crossbeam_channel::Sender<()>,
) -> anyhow::Result<()> {
    loop {
        //let clients = Arc::clone(&clients);
        match tcp_listener.accept() {
            Ok((tcp_stream, client_addr)) => {
                tcp_stream
                    .set_nonblocking(false)
                    .context("stream.set_nonblocking")?;

                log::info!("{} connected", client_addr);

                let (packet_tx, packet_rx) = crossbeam_channel::bounded(MAX_FRAMES_IN_QUEUE);

                let join_handle = std::thread::Builder::new()
                    .name("ps-client".to_owned())
                    .spawn(move || client_loop(packet_rx, client_addr, tcp_stream))
                    .context("Couldn't spawn thread")?;

                clients.write().unwrap().push(Client {
                    client_addr,
                    packet_tx: Some(packet_tx),
                    join_handle: Some(join_handle),
                });
                num_clients.store(clients.read().unwrap().len(), Ordering::SeqCst);
                match recv_client_connect.send(()) {
                    Ok(()) => {}
                    Err(err) => {
                        log::info!("failed to send msg : {}", err);
                    }
                }
            }
            Err(e) => {
                anyhow::bail!("puffin server TCP error: {:?}", e);
            }
        }
    }
}

/// Listens for incoming connections
/// and streams them puffin profiler data.
struct PuffinServerSend {
    clients: Arc<RwLock<Vec<Client>>>,
    num_clients: Arc<AtomicUsize>,
}

impl PuffinServerSend {
    pub fn send(&mut self, frame: &puffin::FrameData) -> anyhow::Result<()> {
        if self.num_clients.load(Ordering::SeqCst) == 0 {
            return Ok(());
        }
        puffin::profile_function!();

        let mut packet = vec![];
        packet
            .write_all(&crate::PROTOCOL_VERSION.to_le_bytes())
            .unwrap();
        frame
            .write_into(&mut packet)
            .context("Encode puffin frame")?;

        let packet: Packet = packet.into();

        let mut clients = self.clients.write().unwrap();
        clients
            .deref_mut()
            .retain(|client| match &client.packet_tx {
                None => false,
                Some(packet_tx) => match packet_tx.try_send(packet.clone()) {
                    Ok(()) => true,
                    Err(crossbeam_channel::TrySendError::Disconnected(_)) => false,
                    Err(crossbeam_channel::TrySendError::Full(_)) => {
                        log::info!(
                            "puffin client {} is not accepting data fast enough; dropping a frame",
                            client.client_addr
                        );
                        true
                    }
                },
            });
        self.num_clients.store(clients.len(), Ordering::SeqCst);

        Ok(())
    }
}

fn client_loop(
    packet_rx: crossbeam_channel::Receiver<Packet>,
    client_addr: SocketAddr,
    mut tcp_stream: TcpStream,
) {
    while let Ok(packet) = packet_rx.recv() {
        if let Err(err) = tcp_stream.write_all(&packet) {
            log::info!(
                "puffin server failed sending to {}: {} (kind: {:?})",
                client_addr,
                err,
                err.kind()
            );
            break;
        }
    }
}
