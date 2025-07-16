use std::fmt::Display;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OnceCell, mpsc};
use tokio::task::JoinHandle;
use anyhow::Context;

const BUFFER_SIZE: usize = 65536;
// TODO: Get this from config
const DEFAULT_MTU: usize = 1300;
// TODO: Get this from config
const SEND_TIMEOUT_MS: u64 = 100;

// TODO: Get this from config
const MAX_CONSECUTIVE_FAILURES: usize = 10;

#[derive(Debug)]
pub struct RxPayload {
    pub from: SocketAddr,
    pub receiver: SocketAddr,
    pub receiver_name: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct NetworkInterfaceId {
    pub name: String,
    pub ip: IpAddr,
}

impl Display for NetworkInterfaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name, self.ip)
    }
}

pub struct NetworkInterface {
    pub id: NetworkInterfaceId,
    socket: tokio::net::UdpSocket,
    receiver_addr: SocketAddr,
    mtu: usize,

    consecutive_failures: std::sync::atomic::AtomicUsize,
    registration_task: OnceCell<JoinHandle<()>>,
    receiver_task: OnceCell<JoinHandle<()>>,
}

impl NetworkInterface {
    pub fn new(
        id: NetworkInterfaceId,
        config: &warp_config::WarpConfig,
        rx_channel: mpsc::UnboundedSender<RxPayload>,
    ) -> anyhow::Result<Arc<Self>> {
        let bind_to_device = config.interfaces.use_bind_to_device.unwrap_or(false);
        let socket = Self::create_socket(&id, bind_to_device)?;
        let receiver_addr = socket.local_addr()?;

        let interface = Arc::new(Self {
            id: id.clone(),
            socket,
            receiver_addr,
            mtu: DEFAULT_MTU,
            consecutive_failures: std::sync::atomic::AtomicUsize::new(0),
            registration_task: OnceCell::new(),
            receiver_task: OnceCell::new(),
        });

        interface
            .registration_task
            .set(Self::spawn_registration_task(interface.clone(), config)?)?;

        interface
            .receiver_task
            .set(Self::spawn_receiver_task(interface.clone(), rx_channel)?)?;

        Ok(interface)
    }

    fn create_socket(interface: &NetworkInterfaceId, bind_to_device: bool) -> anyhow::Result<tokio::net::UdpSocket> {
        let std_socket = std::net::UdpSocket::bind(SocketAddr::new(interface.ip, 0))?;

        // TODO: This is an ugly hack to work around linux routing shenanigans and needs root
        #[cfg(target_os = "linux")]
        if bind_to_device {
            let interface_name_cstr = std::ffi::CString::new(interface.name.clone())?;
            unsafe {
                let ret = libc::setsockopt(
                    std_socket.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_BINDTODEVICE,
                    interface_name_cstr.as_ptr() as *const libc::c_void,
                    interface_name_cstr.as_bytes_with_nul().len() as libc::socklen_t,
                );
                if ret != 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
            }
        }

        std_socket.set_nonblocking(true)?;
        std_socket.set_write_timeout(Some(Duration::from_millis(SEND_TIMEOUT_MS)))?;
        Ok(tokio::net::UdpSocket::from_std(std_socket)?)
    }

    fn spawn_registration_task(
        interface: Arc<Self>,
        config: &warp_config::WarpConfig,
    ) -> anyhow::Result<JoinHandle<()>> {
        let task = tokio::spawn({
            let public_key = config.private_key.public_key();
            let peer_pubkey = config.far_gate.public_key;
            let warp_map_addr = config.warp_map.address;
            let cipher =
                warp_protocol::crypto::cipher_from_shared_secret(&config.private_key, &config.warp_map.public_key);
            let mut interval = tokio::time::interval(Duration::from_secs(config.interfaces.interface_scan_interval));

            async move {
                loop {
                    interval.tick().await;

                    tracing::info!("Registering interface {}", interface.id);

                    if let Err(e) =
                        Self::register_interface(&interface, &public_key, &peer_pubkey, warp_map_addr, &cipher).await
                    {
                        tracing::error!("Registration failed for {}: {}", interface.id, e);
                    }
                }
            }
        });

        Ok(task)
    }

    fn spawn_receiver_task(
        interface: Arc<Self>,
        rx_channel: mpsc::UnboundedSender<RxPayload>,
    ) -> anyhow::Result<JoinHandle<()>> {
        let task = tokio::spawn({
            let receiver_addr = interface.receiver_addr;

            async move {
                let mut buf = vec![0u8; BUFFER_SIZE];

                loop {
                    match interface.socket.recv_from(&mut buf).await {
                        Ok((size, from)) => {
                            let payload = RxPayload {
                                from,
                                receiver: receiver_addr,
                                receiver_name: interface.id.name.clone(),
                                data: buf[..size].to_vec(),
                            };

                            if rx_channel.send(payload).is_err() {
                                tracing::debug!("Receiver channel closed, exiting");
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::error!("Receive error on {}: {}", interface.id, e);
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
            }
        });

        Ok(task)
    }

    async fn register_interface<C>(
        interface: &NetworkInterface,
        public_key: &warp_protocol::PublicKey,
        peer_pubkey: &warp_protocol::PublicKey,
        warp_map_addr: SocketAddr,
        cipher: &C,
    ) -> anyhow::Result<()>
    where
        C: warp_protocol::Aead,
    {
        let timestamp = std::time::SystemTime::now();

        // Send registration
        let registration = warp_protocol::messages::RegisterRequest {
            pubkey: *public_key,
            timestamp,
        };
        interface.send_to(registration, &warp_map_addr, cipher).await?;

        // Query peer address
        let query = warp_protocol::messages::MappingRequest {
            peer_pubkey: *peer_pubkey,
            timestamp,
        };
        interface.send_to(query, &warp_map_addr, cipher).await?;

        Ok(())
    }

    pub async fn send_to<M, C>(&self, message: M, address: &SocketAddr, cipher: &C) -> anyhow::Result<()>
    where
        M: warp_protocol::codec::Message,
        C: warp_protocol::Aead,
    {
        let bytes = message.encode()?.encrypt(cipher)?.to_bytes()?;

        if bytes.len() > self.mtu {
            return Err(anyhow::anyhow!("Message size {} exceeds MTU {}", bytes.len(), self.mtu));
        }

        match self.socket.send_to(&bytes, address).await {
            Ok(sent_bytes) if sent_bytes == bytes.len() => {
                self.consecutive_failures.store(0, std::sync::atomic::Ordering::Release);
                Ok(())
            },
            Ok(sent_bytes) => {
                self.consecutive_failures.fetch_add(1, std::sync::atomic::Ordering::Release);
                Err(anyhow::anyhow!(
                "Incomplete send; attempted to send {} bytes but only sent {}",
                bytes.len(),
                sent_bytes
                ))
            },
            Err(e) => {
                self.consecutive_failures.fetch_add(1, std::sync::atomic::Ordering::Release);
                Err(e).context(format!("Failed to send {} bytes", bytes.len()))
            },
        }
    }

    pub fn is_alive(&self) -> bool {
        println!("{}: {} consecutive failures", self.id, self.consecutive_failures.load(std::sync::atomic::Ordering::Relaxed));
        self.consecutive_failures.load(std::sync::atomic::Ordering::Relaxed) < MAX_CONSECUTIVE_FAILURES
    }

    fn stop(&mut self) {
        if let Some(task) = self.registration_task.get() {
            task.abort();
        }
        if let Some(task) = self.receiver_task.get() {
            task.abort();
        }
    }
}

impl Drop for NetworkInterface {
    fn drop(&mut self) {
        self.stop();
    }
}
