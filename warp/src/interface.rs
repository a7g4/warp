use std::fmt::Display;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

const BUFFER_SIZE: usize = 65536;

#[derive(Debug)]
pub struct RxPayload {
    pub from: SocketAddr,
    pub receiver: SocketAddr,
    pub receiver_name: String,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub struct TxPayload {
    pub to: SocketAddr,
    pub deadline: Option<std::time::Instant>,
    // TODO: Change this to a warp-protocol::codec::Message so the interface can trace the nonce/tracer
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
    max_consecutive_failures: usize,

    consecutive_failures: std::sync::atomic::AtomicUsize,
    registration_task: tokio::sync::OnceCell<JoinHandle<()>>,
    receiver_task: tokio::sync::OnceCell<JoinHandle<()>>,

    sender_queue_tx: tokio::sync::mpsc::UnboundedSender<TxPayload>,
    sender_task: tokio::sync::OnceCell<JoinHandle<()>>,

    // External address as seen by warp-map (for PeerAddressOverride)
    // TODO: Is this the right way to do this? I just want a C++ like Atomic<Option<SocketAddr>>
    external_address_notifier: tokio::sync::watch::Sender<Option<SocketAddr>>,
    external_address_watch: tokio::sync::watch::Receiver<Option<SocketAddr>>,
}

impl NetworkInterface {
    pub fn new(
        id: NetworkInterfaceId,
        config: &warp_config::WarpConfig,
        rx_channel: tokio::sync::mpsc::UnboundedSender<RxPayload>,
    ) -> anyhow::Result<Arc<Self>> {
        let bind_to_device = config.interfaces.bind_to_device.unwrap_or(false);
        let socket = Self::create_socket(&id, bind_to_device)?;
        let receiver_addr = socket.local_addr()?;

        let (outbound_sender, outbound_receiver) = tokio::sync::mpsc::unbounded_channel::<TxPayload>();
        let (external_address_notifier, external_address_watch) = tokio::sync::watch::channel(None);

        let interface = Arc::new(Self {
            id: id.clone(),
            socket,
            receiver_addr,
            max_consecutive_failures: config.interfaces.max_consecutive_failures,
            consecutive_failures: std::sync::atomic::AtomicUsize::new(0),
            registration_task: tokio::sync::OnceCell::new(),
            receiver_task: tokio::sync::OnceCell::new(),
            sender_queue_tx: outbound_sender,
            sender_task: tokio::sync::OnceCell::new(),
            external_address_notifier,
            external_address_watch,
        });

        interface
            .registration_task
            .set(Self::spawn_registration_task(interface.clone(), config)?)?;

        interface
            .receiver_task
            .set(Self::spawn_receiver_task(interface.clone(), rx_channel)?)?;

        interface
            .sender_task
            .set(Self::spawn_sender_task(interface.clone(), outbound_receiver)?)?;

        Ok(interface)
    }

    fn create_socket(interface: &NetworkInterfaceId, bind_to_device: bool) -> anyhow::Result<tokio::net::UdpSocket> {
        let std_socket = std::net::UdpSocket::bind(SocketAddr::new(interface.ip, 0))?;

        let interface_name_cstr = std::ffi::CString::new(interface.name.clone())?;

        // TODO: This is an ugly hack to work around routing shenanigans and may need root
        if bind_to_device {
            #[cfg(target_os = "linux")]
            unsafe {
                use std::os::fd::AsRawFd;
                tracing::info!("Using SO_BINDTODEVICE for {}", interface);
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
            #[cfg(target_os = "macos")]
            unsafe {
                tracing::info!("Using IP_BOUND_IF for {}", interface);
                use std::os::fd::AsRawFd;

                let interface_index = libc::if_nametoindex(interface_name_cstr.as_ptr());
                if interface_index == 0 {
                    return Err(std::io::Error::last_os_error().into());
                }

                let ret = libc::setsockopt(
                    std_socket.as_raw_fd(),
                    libc::IPPROTO_IP,
                    libc::IP_BOUND_IF,
                    &interface_index as *const u32 as *const libc::c_void,
                    std::mem::size_of::<u32>() as libc::socklen_t,
                );
                if ret != 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
            }
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            return Err("bind_to_device is not supported on {}", std::env::consts::OS);
        }

        std_socket.set_nonblocking(true)?;
        Ok(tokio::net::UdpSocket::from_std(std_socket)?)
    }

    fn spawn_registration_task(
        interface: Arc<Self>,
        config: &warp_config::WarpConfig,
    ) -> anyhow::Result<JoinHandle<()>> {
        let task = tokio::task::Builder::new()
            .name(&format!("interface {} registration task", interface.id))
            .spawn({
                let public_key = config.private_key.public_key();
                let peer_pubkey = config.far_gate.public_key;
                let warp_map_addr = config.warp_map.address;
                let cipher =
                    warp_protocol::crypto::cipher_from_shared_secret(&config.private_key, &config.warp_map.public_key);
                let mut interval =
                    tokio::time::interval(Duration::from_secs(config.interfaces.interface_scan_interval));

                async move {
                    loop {
                        interval.tick().await;

                        tracing::info!("Registering interface {} with warp-map", interface.id);

                        if let Err(e) =
                            Self::register_interface(&interface, &public_key, &peer_pubkey, warp_map_addr, &cipher)
                                .await
                        {
                            tracing::error!("Registration failed for {}: {}", interface.id, e);
                        }
                    }
                }
            })
            .expect("task initialised");

        Ok(task)
    }

    fn spawn_receiver_task(
        interface: Arc<Self>,
        rx_channel: tokio::sync::mpsc::UnboundedSender<RxPayload>,
    ) -> anyhow::Result<JoinHandle<()>> {
        let task = tokio::task::Builder::new()
            .name(&format!("interface {} receiver", interface.id))
            .spawn({
                let receiver_addr = interface.receiver_addr;

                async move {
                    let mut buf = vec![0u8; BUFFER_SIZE];

                    loop {
                        match interface.socket.recv_from(&mut buf).await {
                            Ok((size, from)) => {
                                tracing::event!(
                                    tracing::Level::DEBUG,
                                    interface = %interface.id,
                                    from_addr = %from,
                                    payload_size = size,
                                    "INTERFACE_RX"
                                );
                                let payload = RxPayload {
                                    from,
                                    receiver: receiver_addr,
                                    receiver_name: interface.id.name.clone(),
                                    data: buf[..size].to_vec(),
                                };
                                rx_channel.send(payload).expect("Channel should be open");
                            }
                            Err(e) => {
                                tracing::event!(
                                    tracing::Level::WARN,
                                    interface = %interface.id,
                                    error = %e,
                                    "INTERFACE_RX_FAILED"
                                );
                            }
                        }
                    }
                }
            })?;

        Ok(task)
    }

    fn spawn_sender_task(
        interface: Arc<Self>,
        mut outbound_rx: tokio::sync::mpsc::UnboundedReceiver<TxPayload>,
    ) -> anyhow::Result<JoinHandle<()>> {
        let task = tokio::task::Builder::new()
            .name(&format!("interface {} sender", interface.id))
            .spawn({
                async move {
                    while let Some(tx_payload) = outbound_rx.recv().await {
                        let queue_length = outbound_rx.len();
                        if let Some(deadline) = tx_payload.deadline
                            && deadline < std::time::Instant::now()
                        {
                            tracing::event!(
                                tracing::Level::WARN,
                                interface = interface.id.name,
                                destination = %tx_payload.to,
                                payload_size = tx_payload.data.len(),
                                queue_length = queue_length,
                                "INTERFACE_SEND_DEADLINE_MISSED"
                            );
                            continue;
                        }
                        let send_start_time = std::time::Instant::now();
                        let send_result = if let Some(deadline) = tx_payload.deadline {
                            tokio::time::timeout_at(
                                deadline.into(),
                                interface.socket.send_to(&tx_payload.data, tx_payload.to),
                            )
                        } else {
                            // TODO: What should this default to? Configurable?
                            tokio::time::timeout(
                                std::time::Duration::from_millis(100),
                                interface.socket.send_to(&tx_payload.data, tx_payload.to),
                            )
                        }
                        .await;
                        let send_duration = send_start_time.elapsed();
                        match send_result {
                            Ok(Ok(sent_bytes)) if sent_bytes == tx_payload.data.len() => {
                                interface
                                    .consecutive_failures
                                    .store(0, std::sync::atomic::Ordering::Release);
                                tracing::event!(
                                    tracing::Level::DEBUG,
                                    interface = interface.id.name,
                                    destination = %tx_payload.to,
                                    send_duration_us = send_duration.as_micros(),
                                    payload_size = tx_payload.data.len(),
                                    queue_length = queue_length,
                                    "INTERFACE_SEND"
                                );
                            }
                            Ok(Ok(sent_bytes)) => {
                                interface
                                    .consecutive_failures
                                    .fetch_add(1, std::sync::atomic::Ordering::Release);
                                tracing::event!(
                                    tracing::Level::WARN,
                                    interface = interface.id.name,
                                    destination = %tx_payload.to,
                                    send_duration_us = send_duration.as_micros(),
                                    payload_size = tx_payload.data.len(),
                                    sent_bytes = sent_bytes,
                                    queue_length = queue_length,
                                    "INTERFACE_SEND_INCOMPLETE"
                                );
                            }
                            Ok(Err(e)) => {
                                interface
                                    .consecutive_failures
                                    .fetch_add(1, std::sync::atomic::Ordering::Release);
                                tracing::event!(
                                    tracing::Level::WARN,
                                    interface = interface.id.name,
                                    destination = %tx_payload.to,
                                    send_duration_us = send_duration.as_micros(),
                                    payload_size = tx_payload.data.len(),
                                    queue_length = queue_length,
                                    error = %e,
                                    "INTERFACE_SEND_FAILED"
                                );
                            }
                            Err(_timeout_err) => {
                                interface
                                    .consecutive_failures
                                    .fetch_add(1, std::sync::atomic::Ordering::Release);
                                tracing::event!(
                                    tracing::Level::WARN,
                                    interface = interface.id.name,
                                    destination = %tx_payload.to,
                                    send_duration_us = send_duration.as_micros(),
                                    payload_size = tx_payload.data.len(),
                                    queue_length = queue_length,
                                    "INTERFACE_SEND_TIMEOUT"
                                );
                            }
                        }
                    }
                }
            })?;

        Ok(task)
    }
    async fn register_interface(
        interface: &NetworkInterface,
        public_key: &warp_protocol::PublicKey,
        peer_pubkey: &warp_protocol::PublicKey,
        warp_map_addr: SocketAddr,
        cipher: &warp_protocol::Cipher,
    ) -> anyhow::Result<()> {
        use warp_protocol::codec::Message;
        let timestamp = std::time::SystemTime::now();

        // Send registration
        let registration = warp_protocol::messages::RegisterRequest {
            pubkey: *public_key,
            timestamp,
        };
        let mut payload = registration.encode()?.encrypt(cipher)?.to_bytes()?;

        // Query peer address
        let query = warp_protocol::messages::MappingRequest {
            peer_pubkey: *peer_pubkey,
            timestamp,
        };

        payload.append(&mut query.encode()?.encrypt(cipher)?.to_bytes()?);

        interface.queue_send(payload, &warp_map_addr, None)?;

        Ok(())
    }

    pub fn queue_send(
        &self,
        data: Vec<u8>,
        address: &SocketAddr,
        deadline: Option<std::time::Instant>,
    ) -> anyhow::Result<()> {
        self.sender_queue_tx.send(TxPayload {
            data,
            deadline,
            to: *address,
        })?;
        Ok(())
    }

    pub fn is_alive(&self) -> bool {
        self.consecutive_failures.load(std::sync::atomic::Ordering::Relaxed) < self.max_consecutive_failures
    }

    pub fn get_external_address(&self) -> Option<SocketAddr> {
        *self.external_address_watch.borrow()
    }

    pub fn set_external_address(&self, address: SocketAddr) {
        self.external_address_notifier.send_replace(Some(address));
    }

    fn stop(&mut self) {
        if let Some(task) = self.registration_task.get() {
            task.abort();
        }
        if let Some(task) = self.receiver_task.get() {
            task.abort();
        }
        if let Some(task) = self.sender_task.get() {
            task.abort();
        }
    }
}

impl Drop for NetworkInterface {
    fn drop(&mut self) {
        self.stop();
    }
}
