use clap::Parser;
use futures::StreamExt;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::Layer;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use warp_protocol::codec::Message;

mod interface;
mod tunnel;

#[derive(Parser)]
#[command(name = "warp")]
#[command(about = "Warp data across any network")]
struct Args {
    #[arg()]
    warp_config_path: PathBuf,
}

struct WarpCore {
    warp_config: warp_config::WarpConfig,
}

impl WarpCore {
    fn new(warp_config: warp_config::WarpConfig) -> Self {
        WarpCore { warp_config }
    }

    async fn run(&mut self) {
        let mut futures = futures::stream::FuturesUnordered::new();
        let (interfaces_tx, interfaces_rx) = tokio::sync::watch::channel(interface::InterfaceMap::new());
        let (peer_addresses_tx, peer_addresses_rx) = tokio::sync::watch::channel(Vec::<std::net::SocketAddr>::new());
        let interface_filter = self.warp_config.interfaces.exclusion_patterns.clone();

        let warp_map_cipher = warp_protocol::crypto::cipher_from_shared_secret(
            &self.warp_config.private_key,
            &self.warp_config.warp_map.public_key,
        );
        let peer_cipher = warp_protocol::crypto::cipher_from_shared_secret(
            &self.warp_config.private_key,
            &self.warp_config.far_gate.public_key,
        );

        // Using an unbounded queue as we have no way to communicate backpressure to the remote sender?
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<interface::RxPayload>();

        let interface_scan_task = tokio::task::Builder::new()
            .name("interface scan task")
            .spawn({
                let warp_config = self.warp_config.clone();
                let mut interfaces = interface::InterfaceMap::new();
                let interfaces_tx = interfaces_tx.clone();
                async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                        warp_config.interfaces.interface_scan_interval,
                    ));

                    loop {
                        interval.tick().await;

                        // TODO: Extract this into a method so we can handle errors properly
                        {
                            // TODO: Only querying for IPv4 interfaces; IPv6 should also just work but we haven't tested them
                            let ipv4_interfacse: Vec<_> = pnet::datalink::interfaces()
                                .iter()
                                .filter(|iface| !interface_filter.is_match(&iface.name))
                                .filter_map(|iface| {
                                    iface
                                        .ips
                                        .iter()
                                        .find(|ip| matches!(ip.ip(), std::net::IpAddr::V4(_)))
                                        .map(|ip| crate::interface::NetworkInterfaceIdentifier {
                                            name: iface.name.clone(),
                                            ip: ip.ip(),
                                        })
                                })
                                .collect();

                            interfaces.retain(|existing_id, existing_interface| {
                                let retain = ipv4_interfacse.iter().any(|current_id| existing_id == current_id);
                                if !retain {
                                    info!("Interface {existing_id}) no longer detected; removing");
                                }
                                retain
                            });

                            for interface_id in ipv4_interfacse {
                                interfaces.entry(interface_id.clone()).or_insert_with(|| {
                                    info!("Adding new interface {}", interface_id);
                                    crate::interface::NetworkInterface::new(&interface_id, &warp_config, tx.clone())
                                        .unwrap()
                                });
                            }
                        }
                        interfaces_tx.send_replace(interfaces.clone());
                    }
                }
            })
            .unwrap();
        futures.push(interface_scan_task);

        let mut tunnel_gates: std::collections::HashMap<u64, std::sync::Arc<tunnel::Gate>> =
            std::collections::HashMap::new();
        for (warp_tunnel_name, warp_tunnel_config) in &self.warp_config.tunnels {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            warp_tunnel_name.hash(&mut hasher);
            let tunnel_id = hasher.finish();

            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

            let gate = tunnel::Gate::new(warp_tunnel_name, warp_tunnel_config.gate.clone(), tx.clone()).unwrap();
            tunnel_gates.insert(tunnel_id, gate);

            let warp_gate_task = tokio::task::Builder::new()
                .name(&format!("warp-gate {}: data accelerator", warp_tunnel_name))
                .spawn({
                    let tunnel_id: [u8; 8] = tunnel_id.to_le_bytes();
                    let interfaces_rx = interfaces_rx.clone();
                    let peer_addresses_rx = peer_addresses_rx.clone();
                    let peer_cipher = peer_cipher.clone();
                    async move {
                        while let Some(data) = rx.recv().await {
                            let msg = warp_protocol::messages::TunnelPayload { tunnel_id, data };
                            let interfaces = interfaces_rx.borrow().clone();
                            let peer_addresses = peer_addresses_rx.borrow().clone();
                            for (interface_id, interface) in interfaces.iter() {
                                for peer_address in peer_addresses.iter() {
                                    interface
                                        .send_to(msg.clone(), peer_address, &peer_cipher)
                                        .await
                                        .unwrap();
                                }
                            }
                        }
                    }
                })
                .unwrap();

            futures.push(warp_gate_task);
        }
        let tunnel_gates = std::sync::Arc::new(tunnel_gates);

        let rx_processing_task = tokio::task::Builder::new()
            .name("rx processing task")
            .spawn({
                let peer_addresses_tx = peer_addresses_tx.clone();
                let warp_config = self.warp_config.clone();
                let warp_map_cipher = warp_map_cipher.clone();
                let peer_cipher = peer_cipher.clone();
                let tunnel_gates = tunnel_gates.clone();
                async move {
                    while let Some(payload) = rx.recv().await {
                        let mut remaining_buf = payload.data.as_slice();
                        loop {
                            let (msg, buf) = warp_protocol::codec::WireMessage::from_slice(remaining_buf).unwrap();

                            match payload.from {
                                from if from == warp_config.warp_map.address => {
                                    let decrypted_wire_msg = msg.decrypt(&warp_map_cipher).unwrap();
                                    match decrypted_wire_msg.message_id {
                                        warp_protocol::messages::RegisterResponse::MESSAGE_ID => {
                                            let register_response =
                                                warp_protocol::messages::RegisterResponse::decode(decrypted_wire_msg)
                                                    .unwrap();
                                            info!(
                                                "{} is visible publicly at {}",
                                                payload.receiver, register_response.address
                                            );
                                            tracing::event!(
                                                tracing::Level::INFO,
                                                interface = payload.receiver_name,
                                                round_trip_latency_warp_map = std::time::SystemTime::now()
                                                    .duration_since(register_response.request_timestamp)
                                                    .unwrap()
                                                    .as_secs_f32(),
                                            );
                                        }
                                        warp_protocol::messages::MappingResponse::MESSAGE_ID => {
                                            let mapping =
                                                warp_protocol::messages::MappingResponse::decode(decrypted_wire_msg)
                                                    .unwrap();
                                            tracing::event!(
                                                tracing::Level::INFO,
                                                peer_addresses = mapping.endpoints.len()
                                            );
                                            peer_addresses_tx.send_replace(mapping.endpoints);
                                        }
                                        _ => {
                                            info!(
                                                "Received unexpected message from warp-map: {:?}",
                                                decrypted_wire_msg
                                            );
                                        }
                                    }
                                }
                                from => {
                                    // Assume everything else is from our peer
                                    let decrypted_wire_msg = msg.decrypt(&peer_cipher);
                                    if let Ok(decrypted_wire_msg) = decrypted_wire_msg {
                                        match decrypted_wire_msg.message_id {
                                            warp_protocol::messages::TunnelPayload::MESSAGE_ID => {
                                                let tunnel_payload =
                                                    warp_protocol::messages::TunnelPayload::decode(decrypted_wire_msg)
                                                        .unwrap();
                                                let tunnel_id = u64::from_le_bytes(tunnel_payload.tunnel_id);
                                                match tunnel_gates.get(&tunnel_id) {
                                                    None => {
                                                        tracing::warn!(
                                                            "Received data at {} for unknown tunnel {} from {}",
                                                            &payload.receiver,
                                                            from,
                                                            tunnel_id
                                                        );
                                                    }
                                                    Some(gate) => {
                                                        gate.send_to_application(&tunnel_payload.data).await.unwrap();
                                                    }
                                                }
                                            }
                                            _ => {
                                                tracing::warn!(
                                                    "Received unexpected message at {} from {}; {:?}",
                                                    &payload.receiver,
                                                    from,
                                                    decrypted_wire_msg
                                                );
                                            }
                                        }
                                    } else {
                                        info!(
                                            "Received invalid message at {} from {}; ignoring",
                                            &payload.receiver, from
                                        );
                                    }
                                }
                            }

                            remaining_buf = buf;
                            if remaining_buf.is_empty() {
                                break;
                            }
                        }
                    }
                }
            })
            .unwrap();
        futures.push(rx_processing_task);

        while futures.next().await.is_some() {
            println!(".");
        }
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

    let warp_config: warp_config::WarpConfig =
        toml::from_str(std::fs::read_to_string(args.warp_config_path)?.as_str())?;

    info!(
        "Public key: {}",
        warp_protocol::crypto::pubkey_to_string(&warp_config.private_key.public_key())
    );

    WarpCore::new(warp_config).run().await;

    Ok(())
}
