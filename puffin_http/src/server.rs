use anyhow::Context as _;
use async_std::{
    io::WriteExt,
    net::{TcpListener, TcpStream},
    task,
};
use puffin::GlobalProfiler;
use std::{
    net::SocketAddr,
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
    join_handle: Option<std::thread::JoinHandle<()>>,
    num_clients: Arc<AtomicUsize>,
}

impl Server {
    /// Start listening for connections on this addr (e.g. "0.0.0.0:8585")
    pub fn new(bind_addr: &str) -> anyhow::Result<Self> {
        let tcp_listener = task::block_on(async {
            TcpListener::bind(bind_addr)
                .await
                .context("binding server TCP socket")
        })?;

        // We use crossbeam_channel instead of `mpsc`,
        // because on shutdown we want all frames to be sent.
        // `mpsc::Receiver` stops receiving as soon as the `Sender` is dropped,
        // but `crossbeam_channel` will continue until the channel is empty.
        let (tx, rx): (crossbeam_channel::Sender<Arc<puffin::FrameData>>, _) =
            crossbeam_channel::unbounded();

        let clients = Arc::new(RwLock::new(Vec::new()));
        let num_clients = Arc::new(AtomicUsize::default());
        let num_clients_cloned = num_clients.clone();

        let join_handle = std::thread::Builder::new()
            .name("puffin-server".to_owned())
            .spawn(move || {
                let mut ps_connection = PuffinServerConnection {
                    tcp_listener,
                    clients: clients.clone(),
                    num_clients: num_clients_cloned.clone(),
                };
                let mut ps_send = PuffinServerSend {
                    clients,
                    num_clients: num_clients_cloned,
                };

                while let Ok(frame) = rx.recv() {
                    if let Err(err) = task::block_on(ps_connection.accept_new_clients()) {
                        log::warn!("puffin server failure: {}", err);
                    }
                    if let Err(err) = task::block_on(ps_send.send(&frame)) {
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
            join_handle: Some(join_handle),
            num_clients,
        })
    }

    /// Number of clients currently connected.
    pub fn num_clients(&self) -> usize {
        self.num_clients.load(Ordering::SeqCst)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        GlobalProfiler::lock().remove_sink(self.sink_id);

        // Take care to send everything before we shut down:
        if let Some(join_handle) = self.join_handle.take() {
            join_handle.join().ok();
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

/// Listens for incoming connections
struct PuffinServerConnection {
    tcp_listener: TcpListener,
    clients: Arc<RwLock<Vec<Client>>>,
    num_clients: Arc<AtomicUsize>,
}
impl PuffinServerConnection {
    async fn accept_new_clients(&mut self) -> anyhow::Result<()> {
        loop {
            match self.tcp_listener.accept().await {
                Ok((tcp_stream, client_addr)) => {
                    log::info!("{} connected", client_addr);

                    let (packet_tx, packet_rx) = crossbeam_channel::bounded(MAX_FRAMES_IN_QUEUE);

                    let join_handle = std::thread::Builder::new()
                        .name("puffin-server-client".to_owned())
                        .spawn(move || {
                            task::block_on(client_loop(packet_rx, client_addr, tcp_stream));
                        })
                        .context("Couldn't spawn thread")?;

                    self.clients.write().unwrap().push(Client {
                        client_addr,
                        packet_tx: Some(packet_tx),
                        join_handle: Some(join_handle),
                    });
                    self.num_clients
                        .store(self.clients.read().unwrap().len(), Ordering::SeqCst);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    break; // Nothing to do for now.
                }
                Err(e) => {
                    anyhow::bail!("puffin server TCP error: {:?}", e);
                }
            }
        }
        Ok(())
    }
}

/// streams to client puffin profiler data.
struct PuffinServerSend {
    clients: Arc<RwLock<Vec<Client>>>,
    num_clients: Arc<AtomicUsize>,
}

impl PuffinServerSend {
    pub async fn send(&mut self, frame: &puffin::FrameData) -> anyhow::Result<()> {
        if self.clients.read().unwrap().is_empty() {
            return Ok(());
        }
        puffin::profile_function!();

        let mut packet = vec![];
        packet
            .write_all(&crate::PROTOCOL_VERSION.to_le_bytes())
            .await
            .unwrap();
        frame
            .write_into(&mut packet)
            .context("Encode puffin frame")?;

        let packet: Packet = packet.into();

        let mut clients = self.clients.write().unwrap();
        clients.retain(|client| match &client.packet_tx {
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

async fn client_loop(
    packet_rx: crossbeam_channel::Receiver<Packet>,
    client_addr: SocketAddr,
    mut tcp_stream: TcpStream,
) {
    while let Ok(packet) = packet_rx.recv() {
        if let Err(err) = tcp_stream.write_all(&packet).await {
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
