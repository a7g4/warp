use std::sync::Arc;
use tokio::sync::{OnceCell, mpsc, watch};
use tokio::task::JoinHandle;
use warp_config::WarpGateConfig;

const BUFFER_SIZE: usize = 65536;

enum ApplicationSocket {
    Loopback {
        socket: tokio::net::UdpSocket,
        fixed_destination: Option<std::net::SocketAddr>,
        current_destination: watch::Sender<Option<std::net::SocketAddr>>,
    },
    UnixDomainSocket(tokio::net::UnixDatagram),
}

impl ApplicationSocket {
    async fn recv_from_application<'a>(&self, buf: &'a mut [u8]) -> anyhow::Result<&'a [u8]> {
        let size = match self {
            Self::Loopback {
                socket,
                fixed_destination,
                current_destination,
            } => {
                let (size, addr) = socket.recv_from(buf).await?;

                // Update destination if not fixed
                if fixed_destination.is_none() {
                    current_destination.send_replace(Some(addr));
                }

                size
            }
            Self::UnixDomainSocket(socket) => {
                
                socket.recv(buf).await?
            }
        };
        Ok(&buf[..size])
    }

    async fn send_to_application(
        &self,
        data: &[u8],
        fallback_addr: Option<std::net::SocketAddr>,
    ) -> anyhow::Result<usize> {
        match self {
            Self::Loopback {
                socket,
                fixed_destination,
                ..
            } => match (fixed_destination, fallback_addr) {
                (Some(fixed_destination), _) => Ok(socket.send_to(data, fixed_destination).await?),
                (None, Some(fallback_addr)) => Ok(socket.send_to(data, fallback_addr).await?),
                (None, None) => Err(anyhow::anyhow!("no destination address provided"))?,
            },
            Self::UnixDomainSocket(socket) => Ok(socket.send(data).await?),
        }
    }
}

pub struct OutboundTunnelPayload {
    pub tunnel_payload: warp_protocol::messages::TunnelPayload,
    pub deadline: std::time::Instant,
}

pub struct Gate {
    application_inbound_channel: mpsc::UnboundedSender<warp_protocol::messages::TunnelPayload>,
    application_listener_task: OnceCell<JoinHandle<()>>,
    application_sender_task: OnceCell<JoinHandle<()>>,
}

impl Gate {
    pub fn new(
        tunnel_name: &str,
        tunnel_id: warp_protocol::messages::TunnelId,
        config: WarpGateConfig,
        send_deadline: std::time::Duration,
        application_outbound_channel: mpsc::UnboundedSender<OutboundTunnelPayload>,
    ) -> anyhow::Result<Arc<Self>> {
        let (destination_announce, destination_watch) = watch::channel(None);

        let socket = Self::create_socket(&config, tunnel_name, destination_announce)?;
        let socket = Arc::new(socket);

        let (application_inbound_channel, mut application_inbound_channel_rx) = tokio::sync::mpsc::unbounded_channel();

        let gate = Arc::new(Self {
            application_inbound_channel,
            application_listener_task: OnceCell::new(),
            application_sender_task: OnceCell::new(),
        });

        let application_listener_task = tokio::task::Builder::new()
            .name(&format!("warp-gate {}: application to gate listener", tunnel_name))
            .spawn({
                let tracer_generator = std::sync::atomic::AtomicU64::new(0);
                let tunnel_name = tunnel_name.to_string();
                let socket = socket.clone();
                async move {
                    let mut buf = vec![0u8; BUFFER_SIZE];
                    loop {
                        match socket.recv_from_application(&mut buf).await {
                            Ok(data) => {
                                let tunnel_payload = warp_protocol::messages::TunnelPayload::new(
                                    tunnel_id.clone(),
                                    tracer_generator.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                                    data.to_vec(),
                                );
                                let outbound = OutboundTunnelPayload {
                                    tunnel_payload,
                                    deadline: std::time::Instant::now() + send_deadline,
                                };
                                tracing::event!(
                                    tracing::Level::DEBUG,
                                    tunnel_name = tunnel_name,
                                    tracer = outbound.tunnel_payload.tracer,
                                    payload_size = outbound.tunnel_payload.data.len(),
                                    "APPLICATION_TO_GATE_DATA_RX"
                                );
                                application_outbound_channel
                                    .send(outbound)
                                    .expect("Channel should be open");
                            }
                            Err(e) => {
                                tracing::event!(
                                    tracing::Level::WARN,
                                    tunnel_name = tunnel_name,
                                    error = %e,
                                    "APPLICATION_TO_GATE_DATA_RX_ERROR"
                                );
                            }
                        }

                        #[cfg(feature = "manual_yields")]
                        tokio::task::yield_now().await;
                    }
                }
            })?;
        gate.application_listener_task
            .set(application_listener_task)
            .expect("application_listener_task should not have been set");

        let application_sender_task = tokio::task::Builder::new()
            .name(&format!("warp-gate {}: gate to application tx", tunnel_name))
            .spawn({
                let tunnel_name = tunnel_name.to_string();
                let socket = socket.clone();
                let destination_watch = destination_watch.clone();
                async move {
                    while let Some(tunnel_payload) = application_inbound_channel_rx.recv().await {
                        let fallback_destination = *destination_watch.borrow();
                        let queue_length = application_inbound_channel_rx.len();

                        match socket
                            .send_to_application(&tunnel_payload.data, fallback_destination)
                            .await
                        {
                            Ok(sent) if sent == tunnel_payload.data.len() => {
                                tracing::event!(
                                    tracing::Level::DEBUG,
                                    tunnel_name = tunnel_name,
                                    tracer = tunnel_payload.tracer,
                                    payload_size = tunnel_payload.data.len(),
                                    queue_length = queue_length,
                                    "GATE_TO_APPLICATION_DATA_SUCCESS"
                                );
                            }
                            Ok(sent) => {
                                tracing::event!(
                                    tracing::Level::WARN,
                                    tunnel_name = tunnel_name,
                                    tracer = tunnel_payload.tracer,
                                    payload_size = tunnel_payload.data.len(),
                                    sent_bytes = sent,
                                    queue_length = queue_length,
                                    "GATE_TO_APPLICATION_DATA_INCOMPLETE"
                                );
                            }
                            Err(e) => {
                                tracing::event!(
                                    tracing::Level::WARN,
                                    tunnel_name = tunnel_name,
                                    tracer = tunnel_payload.tracer,
                                    payload_size = tunnel_payload.data.len(),
                                    queue_length = queue_length,
                                    error = %e,
                                    "GATE_TO_APPLICATION_DATA_FAILED"
                                );
                            }
                        }
                        #[cfg(feature = "manual_yields")]
                        tokio::task::yield_now().await;
                    }
                }
            })?;
        gate.application_sender_task
            .set(application_sender_task)
            .expect("application_sender_task should not have been set");

        Ok(gate)
    }

    fn create_socket(
        config: &WarpGateConfig,
        tunnel_name: &str,
        dest_tx: watch::Sender<Option<std::net::SocketAddr>>,
    ) -> anyhow::Result<ApplicationSocket> {
        match config {
            WarpGateConfig::Loopback(config) => {
                let ip = if config.ipv4 {
                    std::net::Ipv4Addr::LOCALHOST.into()
                } else {
                    std::net::Ipv6Addr::LOCALHOST.into()
                };

                let bind_addr = std::net::SocketAddr::new(ip, config.application_to_gate);
                let std_socket = std::net::UdpSocket::bind(bind_addr)?;
                std_socket.set_nonblocking(true)?;
                let socket = tokio::net::UdpSocket::from_std(std_socket)?;

                tracing::info!(
                    "warp-gate {}: listening for application data at {}",
                    tunnel_name,
                    bind_addr
                );

                let fixed_destination = if let Some(port) = config.gate_to_application {
                    let dest_addr = std::net::SocketAddr::new(ip, port);
                    dest_tx.send_replace(Some(dest_addr));
                    tracing::info!("warp-gate {}: sending application data to {}", tunnel_name, dest_addr);
                    Some(dest_addr)
                } else {
                    None
                };

                Ok(ApplicationSocket::Loopback {
                    socket,
                    fixed_destination,
                    current_destination: dest_tx,
                })
            }
            WarpGateConfig::UnixDomainSocket(config) => {
                let _ = std::fs::remove_file(&config.path);
                let socket = tokio::net::UnixDatagram::bind(&config.path)?;

                tracing::info!(
                    "warp-gate {}: communicating with application over socket {}",
                    tunnel_name,
                    config.path.display()
                );

                Ok(ApplicationSocket::UnixDomainSocket(socket))
            }
        }
    }

    pub async fn send_to_application(&self, tunnel_payload: warp_protocol::messages::TunnelPayload) {
        self.application_inbound_channel.send(tunnel_payload).unwrap();
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        if let Some(task) = self.application_listener_task.get() {
            task.abort();
        }
        if let Some(task) = self.application_sender_task.get() {
            task.abort();
        }
    }
}
