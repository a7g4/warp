use std::collections::HashMap;
use std::fmt::Display;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OnceCell, mpsc};

pub type InterfaceMap = HashMap<NetworkInterfaceIdentifier, Arc<NetworkInterface>>;

#[derive(Debug)]
pub struct RxPayload {
    pub from: SocketAddr,
    pub receiver: SocketAddr,
    pub receiver_name: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub(crate) struct NetworkInterfaceIdentifier {
    pub name: String,
    pub ip: IpAddr,
}

impl Display for NetworkInterfaceIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "{} ({})", self.name, self.ip)
    }
}

pub(crate) struct NetworkInterface {
    socket: tokio::net::UdpSocket,
    registration_task: OnceCell<tokio::task::JoinHandle<()>>,
    rx_task: OnceCell<tokio::task::JoinHandle<()>>,
    mtu: usize,
}

impl NetworkInterface {
    pub fn new(
        interface_id: &NetworkInterfaceIdentifier,
        warp_config: &warp_config::WarpConfig,
        rx_channel: tokio::sync::mpsc::UnboundedSender<RxPayload>,
    ) -> anyhow::Result<Arc<Self>> {
        let raw_socket = std::net::UdpSocket::bind(SocketAddr::new(interface_id.ip, 0))?;
        raw_socket.set_nonblocking(true)?;
        raw_socket.set_write_timeout(Some(Duration::from_millis(100)))?;
        let tokio_socket = tokio::net::UdpSocket::from_std(raw_socket)?;
        let receiver_address = tokio_socket.local_addr()?;

        let interface = Arc::new(Self {
            socket: tokio_socket,
            registration_task: OnceCell::new(),
            rx_task: OnceCell::new(),
            mtu: 1300, // TODO: Get from configuration
        });

        // Pre-allocate shared strings
        //let interface_name: Arc<str> = interface_id.name.clone().into();

        // Registration task with proper shutdown
        let weak_interface = Arc::downgrade(&interface);
        let registration_task = tokio::task::Builder::new()
            .name(&format!("warp-map registration task for {}", interface_id))
            .spawn({
                let interface_id = interface_id.clone();
                let public_key = warp_config.private_key.public_key();
                let warp_map_cipher = warp_protocol::crypto::cipher_from_shared_secret(
                    &warp_config.private_key.clone(),
                    &warp_config.warp_map.public_key,
                );
                let mut interface_scan_interval =
                    tokio::time::interval(Duration::from_secs(warp_config.interfaces.interface_scan_interval));
                let warp_map_addr = warp_config.warp_map.address;
                let peer_pubkey = warp_config.far_gate.public_key;

                async move {
                    loop {
                        interface_scan_interval.tick().await;
                        let Some(interface) = weak_interface.upgrade() else {
                            tracing::debug!("Interface dropped, exiting registration task");
                            break;
                        };

                        tracing::info!("Registering interface {}", interface_id);

                        // Handle errors properly instead of continue
                        if let Err(err) = Self::send_registration_messages(
                            &interface,
                            &public_key,
                            &peer_pubkey,
                            warp_map_addr,
                            &warp_map_cipher,
                        )
                        .await
                        {
                            tracing::error!("Registration failed for {}: {}", interface_id, err);
                        }
                    }
                }
            })?;

        // Set task (this should never fail on first set)
        interface
            .registration_task
            .set(registration_task)
            .map_err(|_| anyhow::anyhow!("Failed to set registration task"))?;

        // RX task with proper shutdown and buffer reuse
        let weak_interface = Arc::downgrade(&interface);
        let rx_task = tokio::task::Builder::new()
            .name(&format!("rx task for {}", interface_id))
            .spawn({
                let interface_id = interface_id.clone();

                async move {
                    let mut buf = vec![0u8; 1500];

                    loop {
                        // Check if interface still exists at start of loop
                        let Some(interface) = weak_interface.upgrade() else {
                            tracing::debug!("Interface dropped, exiting rx task");
                            break;
                        };

                        match interface.socket.recv_from(&mut buf).await {
                            Err(err) => {
                                tracing::error!("Error receiving message on {}: {}", interface_id, err);
                            }
                            Ok((recv_size, from)) => {
                                let payload = RxPayload {
                                    from,
                                    receiver: receiver_address,
                                    receiver_name: interface_id.name.clone(),
                                    data: buf[..recv_size].to_vec(),
                                };

                                if let Err(err) = rx_channel.send(payload) {
                                    tracing::error!("Error proxying message to processing queue: {}", err)
                                }
                            }
                        }
                    }
                }
            })?;

        interface
            .rx_task
            .set(rx_task)
            .map_err(|_| anyhow::anyhow!("Failed to set rx task"))?;

        Ok(interface)
    }

    // Extract registration logic for better error handling
    async fn send_registration_messages<C>(
        interface: &NetworkInterface,
        public_key: &warp_protocol::PublicKey,
        peer_pubkey: &warp_protocol::PublicKey,
        warp_map_addr: SocketAddr,
        cipher: &C,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        C: warp_protocol::Aead,
    {
        let registration_msg = warp_protocol::messages::RegisterRequest {
            pubkey: *public_key,
            timestamp: std::time::SystemTime::now(),
        };

        interface.send_to(registration_msg, &warp_map_addr, cipher).await?;

        let peer_address_query = warp_protocol::messages::MappingRequest {
            peer_pubkey: *peer_pubkey,
            timestamp: std::time::SystemTime::now(),
        };

        interface.send_to(peer_address_query, &warp_map_addr, cipher).await?;
        Ok(())
    }

    pub async fn send_to<M, C>(&self, message: M, address: &SocketAddr, cipher: &C) -> tokio::io::Result<usize>
    where
        M: warp_protocol::codec::Message,
        C: warp_protocol::Aead,
    {
        // Better error handling chain
        let encoded = message
            .encode()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let encrypted = encoded
            .encrypt(cipher)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let bytes = encrypted
            .to_bytes()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        if bytes.len() > self.mtu {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Message size {} exceeds MTU {}", bytes.len(), self.mtu),
            ));
        }

        self.socket.send_to(&bytes, address).await
    }
}

impl Drop for NetworkInterface {
    fn drop(&mut self) {
        if let Some(task) = self.registration_task.get() {
            task.abort();
        }
        if let Some(task) = self.rx_task.get() {
            task.abort();
        }
    }
}
