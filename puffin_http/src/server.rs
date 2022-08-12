use anyhow::Context as _;
use async_std::{
    channel,
    io::WriteExt,
    net::{TcpListener, TcpStream},
    sync::{Arc, RwLock},
    task,
};
use puffin::GlobalProfiler;
use std::{
    net::SocketAddr,
    sync::atomic::{AtomicUsize, Ordering},
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
        let bind_addr = String::from(bind_addr);
        // We use crossbeam_channel instead of `mpsc`,
        // because on shutdown we want all frames to be sent.
        // `mpsc::Receiver` stops receiving as soon as the `Sender` is dropped,
        // but `crossbeam_channel` will continue until the channel is empty.
        let (tx, rx): (channel::Sender<Arc<puffin::FrameData>>, _) = channel::unbounded();

        let num_clients = Arc::new(AtomicUsize::default());
        let num_clients_cloned = num_clients.clone();
        let join_handle = std::thread::Builder::new()
            .name("puffin-server".to_owned())
            .spawn(move || {
                Server::run(bind_addr, rx, num_clients_cloned).unwrap();
            })
            .context("Can't start puffin-server thread.")?;

        let sink_id = GlobalProfiler::lock().add_sink(Box::new(move |frame| {
            tx.try_send(frame).ok();
        }));

        Ok(Server {
            sink_id,
            join_handle: Some(join_handle),
            num_clients,
        })
    }

    /// start and run puffin server service
    pub fn run(
        bind_addr: String,
        rx: channel::Receiver<Arc<puffin::FrameData>>,
        num_clients: Arc<AtomicUsize>,
    ) -> anyhow::Result<()> {
        let tcp_listener = task::block_on(async {
            TcpListener::bind(bind_addr)
                .await
                .context("binding server TCP socket")
        })?;

        let clients = Arc::new(RwLock::new(Vec::new()));
        let clients_cloned = clients.clone();
        let num_clients_cloned = num_clients.clone();

        let _psconnect_handle = task::Builder::new()
            .name("ps-connect".to_owned())
            .spawn(async move {
                let mut ps_connection = PuffinServerConnection {
                    tcp_listener,
                    clients: clients_cloned,
                    num_clients: num_clients_cloned,
                };
                if let Err(err) = ps_connection.accept_new_clients().await {
                    log::warn!("puffin server failure: {}", err);
                }
            })
            .context("Couldn't spawn ps-connect task")?;

        let pssend_handle = task::Builder::new()
            .name("ps-send".to_owned())
            .spawn(async move {
                let mut ps_send = PuffinServerSend {
                    clients,
                    num_clients,
                };

                while let Ok(frame) = rx.recv().await {
                    if let Err(err) = ps_send.send(&*frame).await {
                        log::warn!("puffin server failure: {}", err);
                    }
                }
            })
            .context("Couldn't spawn ps-send task")?;

        task::block_on(pssend_handle);
        Ok(())
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
    packet_tx: Option<channel::Sender<Packet>>,
    join_handle: Option<task::JoinHandle<()>>,
}

impl Drop for Client {
    fn drop(&mut self) {
        // Take care to send everything before we shut down!

        // Drop the sender to signal to shut down:
        self.packet_tx = None;

        // Wait for the shutdown:
        if let Some(join_handle) = self.join_handle.take() {
            task::block_on(join_handle); // .ok()
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

                    let (packet_tx, packet_rx) = channel::bounded(MAX_FRAMES_IN_QUEUE);

                    let join_handle = task::Builder::new()
                        .name("ps-client".to_owned())
                        .spawn(async move {
                            client_loop(packet_rx, client_addr, tcp_stream).await;
                        })
                        .context("Couldn't spawn ps-client task")?;

                    self.clients.write().await.push(Client {
                        client_addr,
                        packet_tx: Some(packet_tx),
                        join_handle: Some(join_handle),
                    });
                    self.num_clients
                        .store(self.clients.read().await.len(), Ordering::SeqCst);
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
        if self.clients.read().await.is_empty() {
            return Ok(());
        }
        //puffin::profile_function!(); //TODO: enable again later

        let mut packet = vec![];
        packet
            .write_all(&crate::PROTOCOL_VERSION.to_le_bytes())
            .await
            .unwrap();
        frame
            .write_into(&mut packet)
            .context("Encode puffin frame")?;

        let packet: Packet = packet.into();

        let mut clients = self.clients.write().await;
        clients.retain(|client| {
            task::block_on(async { Self::send_to_client(client, packet.clone()).await })
        });
        self.num_clients.store(clients.len(), Ordering::SeqCst);

        Ok(())
    }

    async fn send_to_client(client: &Client, packet: Packet) -> bool {
        match &client.packet_tx {
            None => false,
            Some(packet_tx) => match packet_tx.send(packet).await {
                Ok(()) => true,
                Err(err) => {
                    log::info!("puffin send error: {} for '{}'", err, client.client_addr);
                    true
                }
            },
        }
    }
}

async fn client_loop(
    packet_rx: channel::Receiver<Packet>,
    client_addr: SocketAddr,
    mut tcp_stream: TcpStream,
) {
    while let Ok(packet) = packet_rx.recv().await {
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
