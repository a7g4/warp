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
    async fn recv_and_forward(
        &self,
        buf: &mut [u8],
        tx_channel: &mpsc::UnboundedSender<Vec<u8>>,
    ) -> anyhow::Result<()> {
        let size: anyhow::Result<usize> = match self {
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

                Ok(size)
            }
            Self::UnixDomainSocket(socket) => {
                let size = socket.recv(buf).await?;
                Ok(size)
            }
        };

        Ok(tx_channel.send(buf[..size?].to_vec())?)
    }

    async fn send(&self, data: &[u8], fallback_addr: Option<std::net::SocketAddr>) -> anyhow::Result<usize> {
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

pub struct Gate {
    pub(crate) tunnel_name: String,
    socket: Arc<ApplicationSocket>,
    current_destination: watch::Receiver<Option<std::net::SocketAddr>>,
    _task: OnceCell<JoinHandle<()>>,
}

impl Gate {
    pub fn new(
        tunnel_name: &str,
        config: WarpGateConfig,
        tx_channel: mpsc::UnboundedSender<Vec<u8>>,
    ) -> anyhow::Result<Arc<Self>> {
        let (dest_tx, dest_rx) = watch::channel(None);

        let socket = Self::create_socket(&config, tunnel_name, dest_tx)?;
        let socket = Arc::new(socket);

        let gate = Arc::new(Self {
            tunnel_name: tunnel_name.to_string(),
            socket: socket.clone(),
            current_destination: dest_rx,
            _task: OnceCell::new(),
        });

        let task = tokio::task::Builder::new()
            .name(&format!("warp-gate {}: application listener", tunnel_name))
            .spawn({
                let tunnel_name = tunnel_name.to_string();
                let socket = socket.clone();
                async move {
                    let mut buf = vec![0u8; BUFFER_SIZE];
                    loop {
                        if let Err(e) = socket.recv_and_forward(&mut buf, &tx_channel).await {
                            tracing::error!("warp-gate {}: recv error: {}", tunnel_name, e);
                        }
                    }
                }
            })?;

        gate._task.set(task).map_err(|_| anyhow::anyhow!("Task already set"))?;
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

                tracing::info!("warp-gate {}: listening on {}", tunnel_name, bind_addr);

                let fixed_destination = if let Some(port) = config.gate_to_application {
                    let dest_addr = std::net::SocketAddr::new(ip, port);
                    dest_tx.send_replace(Some(dest_addr));
                    tracing::info!("warp-gate {}: fixed destination {}", tunnel_name, dest_addr);
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

                tracing::info!("warp-gate {}: using socket {}", tunnel_name, config.path.display());

                Ok(ApplicationSocket::UnixDomainSocket(socket))
            }
        }
    }

    pub async fn send_to_application(&self, data: &[u8]) -> anyhow::Result<()> {
        let fallback_destination = *self.current_destination.borrow();

        let sent = self.socket.send(data, fallback_destination).await?;

        if sent != data.len() {
            return Err(anyhow::anyhow!("Partial send: {} of {} bytes", sent, data.len()));
        }

        Ok(())
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        if let Some(task) = self._task.get() {
            task.abort();
        }
    }
}
