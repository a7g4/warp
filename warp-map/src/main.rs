mod map;

use clap::Parser;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{error, info};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;
use warp_protocol::codec::Message;

#[derive(Parser)]
#[command(name = "warp-map")]
#[command(about = "UDP hole-punching mapping server")]
struct Args {
    #[arg(short, long, default_value = "0.0.0.0:13116")]
    bind: SocketAddr,

    #[arg(short, long, default_value = "A2FP3SPBZ7RDXQPADFDYC9MZ0WQAW7S8RNW6J01C7FENTXY93WSG")]
    private_key: String,

    #[arg(short, long, default_value = "60")]
    client_expiry_seconds: u64,
}

struct WarpMapServer {
    private_key: warp_protocol::PrivateKey,
    bind_addr: SocketAddr,
    client_store: Arc<RwLock<map::ClientStore>>,
}
//
// #[derive(bincode::Decode)]
// struct RegistrationAad {
//     #[bincode(with_serde)]
//     public_key: warp_protocol::PublicKey,
// }

impl WarpMapServer {
    fn new(private_key: warp_protocol::PrivateKey, bind_addr: SocketAddr, client_expiry: std::time::Duration) -> Self {
        Self {
            private_key,
            bind_addr,
            client_store: Arc::new(RwLock::new(map::ClientStore::new(client_expiry))),
        }
    }

    async fn run(&self) {
        let socket = Arc::new(tokio::net::UdpSocket::bind(self.bind_addr).await.unwrap());
        info!("Listening on: {}", socket.local_addr().unwrap());

        // Spawn garbage collection task
        let gc_store = self.client_store.clone();
        tokio::task::Builder::new()
            .name("client store garbage collector")
            .spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                loop {
                    interval.tick().await;
                    gc_store.write().await.garbage_collect(Instant::now());
                }
            })
            .unwrap();

        loop {
            let mut buf = [0; 2 << 9];
            match socket.recv_from(&mut buf).await {
                Ok((len, address)) => {
                    let socket_clone = socket.clone();
                    let private_key = self.private_key.clone();
                    let client_store = self.client_store.clone();

                    let task_name = format!("Handle data from {}", address);

                    // TODO: I think spawning a new task for each message is overkill; do something better
                    let spawn_result = tokio::task::Builder::new().name(&task_name).spawn(async move {
                        match Self::process_rx_buffer(&private_key, &client_store, &buf[..len], &address).await {
                            Ok(response) => {
                                if let Err(e) = socket_clone.send_to(&response, address).await {
                                    error!("Failed to send response to {}: {}", address, e);
                                }
                            }
                            Err(e) => {
                                error!("Error processing message from {}: {}", address, e);
                            }
                        }
                    });
                    match spawn_result {
                        Ok(_) => {}
                        Err(e) => {
                            error!("Error spawning task for message from {}: {}", address, e);
                        }
                    }
                }
                Err(e) => {
                    error!("Error receiving from socket: {}", e);
                }
            }
        }
    }

    async fn process_rx_buffer(
        private_key: &warp_protocol::PrivateKey,
        client_store: &Arc<RwLock<map::ClientStore>>,
        buf: &[u8],
        from: &SocketAddr,
    ) -> anyhow::Result<Vec<u8>> {
        let mut response_bytes: Vec<u8> = Vec::new();
        let mut remaining_buf = buf;

        loop {
            let (msg, buf) = warp_protocol::codec::WireMessage::from_slice(remaining_buf)?;

            let client_key = {
                let store = client_store.read().await;
                match store.get_pubkey(from) {
                    None => {
                        let (aad, _): (warp_protocol::messages::RegisterRequestAssociatedData, usize) =
                            bincode::decode_from_slice(&msg.associated_data, bincode::config::standard())?;
                        aad.pubkey
                    }
                    Some(client_key) => client_key,
                }
            };

            let cipher = warp_protocol::crypto::cipher_from_shared_secret(private_key, &client_key);
            let decrypted = msg.decrypt(&cipher)?;
            let client_key_string = warp_protocol::crypto::pubkey_to_string(&client_key);

            match decrypted.message_id {
                warp_protocol::messages::RegisterRequest::MESSAGE_ID => {
                    let registration_msg: warp_protocol::messages::RegisterRequest = decrypted.decode()?;

                    {
                        let mut store = client_store.write().await;
                        store.register_client(client_key, *from, Instant::now());
                    }

                    let response = warp_protocol::messages::RegisterResponse {
                        address: *from,
                        timestamp: std::time::SystemTime::now(),
                        request_timestamp: registration_msg.timestamp,
                    };
                    let dt = response.timestamp.duration_since(registration_msg.timestamp)?;
                    tracing::event!(
                        name: "RegistrationRequest",
                        tracing::Level::INFO,
                        public_key = client_key_string,
                        address = from.to_string().as_str(),
                        clock_network_skew = dt.as_secs_f32());

                    let bytes = response.encode()?.encrypt(&cipher)?.to_bytes()?;
                    response_bytes.extend_from_slice(bytes.as_slice());
                }
                warp_protocol::messages::MappingRequest::MESSAGE_ID => {
                    println!("MappingRequest");
                    let mapping_msg: warp_protocol::messages::MappingRequest = decrypted.decode()?;

                    let addresses = {
                        let store = client_store.read().await;
                        store.get_addresses(&mapping_msg.peer_pubkey, Instant::now())
                    };

                    let n_addresses = addresses.len();
                    let response = warp_protocol::messages::MappingResponse {
                        peer_pubkey: mapping_msg.peer_pubkey,
                        endpoints: addresses,
                        timestamp: std::time::SystemTime::now(),
                    };
                    let dt = response.timestamp.duration_since(mapping_msg.timestamp)?;
                    info!(
                        "Mapping request received from {}, returned {} addresses, transit time + clock skew = {}",
                        client_key_string,
                        n_addresses,
                        dt.as_secs()
                    );

                    let bytes = response.encode()?.encrypt(&cipher)?.to_bytes()?;
                    response_bytes.extend_from_slice(bytes.as_slice());
                }
                id => return Err(warp_protocol::DecodeError::UnexpectedMessageId(id).into()),
            }

            remaining_buf = buf;
            if remaining_buf.is_empty() {
                break;
            }

            // Yield to allow other tasks to run
            tokio::task::yield_now().await;
        }
        Ok(response_bytes)
    }
}

fn main() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;

    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_span_events(FmtSpan::CLOSE)
        .with_filter(tracing_subscriber::filter::LevelFilter::INFO);
    let tokio_console_layer = console_subscriber::spawn();

    tracing_subscriber::registry()
        .with(tokio_console_layer)
        .with(stdout_layer)
        .init();

    rt.block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
    let args = Args::parse();
    let private_key = warp_protocol::crypto::privkey_from_string(&args.private_key)?;

    info!(
        "Public key: {}",
        warp_protocol::crypto::pubkey_to_string(&private_key.public_key())
    );

    WarpMapServer::new(
        private_key,
        args.bind,
        std::time::Duration::from_secs(args.client_expiry_seconds),
    )
    .run()
    .await;
    Ok(())
}
