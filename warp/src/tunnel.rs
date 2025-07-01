use futures::SinkExt;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::OnceCell;

// The warp tunnel "Gate" is the interface between warp and external applications
pub(crate) struct Gate {
    socket: GateSocket,
    rx_task: OnceCell<tokio::task::JoinHandle<()>>,
}

enum GateSocket {
    Loopback {
        socket: tokio::net::UdpSocket,
        destination: std::net::SocketAddr,
    },
    UnixDomainSocket {
        socket: tokio::net::UnixDatagram,
    },
}

impl Gate {
    pub(crate) fn new(
        tunnel_name: &str,
        gate_config: warp_config::WarpGateConfig,
        rx_channel: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    ) -> anyhow::Result<Arc<Self>> {
        let gate = Arc::new(match gate_config {
            warp_config::WarpGateConfig::Loopback(config) => {
                let bind_address = SocketAddr::new(
                    match config.ipv4 {
                        true => std::net::Ipv4Addr::LOCALHOST.into(),
                        false => std::net::Ipv6Addr::LOCALHOST.into(),
                    },
                    config.application_to_gate,
                );
                let destination_address = SocketAddr::new(
                    match config.ipv4 {
                        true => std::net::Ipv4Addr::LOCALHOST.into(),
                        false => std::net::Ipv6Addr::LOCALHOST.into(),
                    },
                    config.gate_to_application,
                );
                let raw_socket = std::net::UdpSocket::bind(bind_address)?;
                raw_socket.set_nonblocking(true)?;
                let tokio_socket = tokio::net::UdpSocket::from_std(raw_socket)?;
                tracing::info!(
                    "warp-gate {}: waiting for application data at {}, forwarding tunnel data to {}",
                    tunnel_name,
                    bind_address,
                    destination_address
                );
                Self {
                    socket: GateSocket::Loopback {
                        socket: tokio_socket,
                        destination: destination_address,
                    },
                    rx_task: OnceCell::new(),
                }
            }
            warp_config::WarpGateConfig::UnixDomainSocket(path) => {
                // TODO: Is there a more robust way than just deleting whatever's at the path?
                match std::fs::remove_file(&path.path) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(e) => Err(e),
                }?;
                let socket = tokio::net::UnixDatagram::bind(&path.path)?;
                tracing::info!(
                    "warp-gate {}: reading and writing from {}",
                    tunnel_name,
                    path.path.display()
                );
                Self {
                    socket: GateSocket::UnixDomainSocket { socket },
                    rx_task: OnceCell::new(),
                }
            }
        });

        let weak_interface = Arc::downgrade(&gate);
        let rx_task = tokio::task::Builder::new()
            .name(&format!("warp-gate {}: application listener", tunnel_name))
            .spawn({
                let tunnel_name = tunnel_name.to_string();
                async move {
                    let mut buf = vec![0u8; 1500];
                    let gate = weak_interface.upgrade().unwrap();
                    loop {
                        match gate.recv_from_application(&mut buf).await {
                            Err(err) => {
                                tracing::error!("Error receiving message by gate {}: {}", tunnel_name, err);
                            }
                            Ok(recv_size) => {
                                if let Err(err) = rx_channel.send(buf[..recv_size].to_vec()) {
                                    tracing::error!("Error proxying message to processing queue: {}", err)
                                }
                            }
                        }
                    }
                }
            })?;

        gate.rx_task
            .set(rx_task)
            .map_err(|_| anyhow::anyhow!("Failed to set rx task"))?;

        Ok(gate)
    }

    async fn recv_from_application(&self, buf: &mut [u8]) -> anyhow::Result<usize> {
        match &self.socket {
            GateSocket::Loopback { socket, destination } => Ok(socket.recv(buf).await?),
            GateSocket::UnixDomainSocket { socket } => Ok(socket.recv(buf).await?),
        }
    }

    pub(crate) async fn send_to_application(&self, data: &[u8]) -> anyhow::Result<()> {
        match &self.socket {
            GateSocket::Loopback { socket, destination } => {
                let sent_bytes = socket.send(data).await?;
                match sent_bytes {
                    bytes if bytes == data.len() => Ok(()),
                    _ => Err(anyhow::anyhow!("send call did not send the complete buffer")),
                }
            }
            GateSocket::UnixDomainSocket { socket } => {
                let sent_bytes = socket.send(data).await?;
                match sent_bytes {
                    bytes if bytes == data.len() => Ok(()),
                    _ => Err(anyhow::anyhow!("send call did not send the complete buffer")),
                }
            }
        }
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        if let Some(task) = self.rx_task.get() {
            task.abort()
        }
    }
}
