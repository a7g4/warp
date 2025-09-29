use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use warp_protocol::codec::Message;

mod interface;
mod tunnel;
mod routing;

#[derive(Parser)]
#[command(name = "warp")]
#[command(about = "Warp data across any network")]
struct Args {
    #[arg()]
    warp_config_path: PathBuf,

    #[arg(short, long, default_value_t = tracing_subscriber::filter::LevelFilter::INFO)]
    verbosity: tracing_subscriber::filter::LevelFilter,
}

struct WarpCore {
    warp_config: warp_config::WarpConfig,
    shutdown: tokio::sync::oneshot::Receiver<()>,
}

impl WarpCore {
    fn new(warp_config: warp_config::WarpConfig) -> (Self, tokio::sync::oneshot::Sender<()>) {
        let (shutdown_notifier, shutdown) = tokio::sync::oneshot::channel();
        let warp_core = WarpCore { warp_config, shutdown };
        (warp_core, shutdown_notifier)
    }

    async fn run(&mut self) {
        let mut futures = futures::stream::FuturesUnordered::new();
        
        // Create consolidated packet routing state
        let routing_state = std::sync::Arc::new(routing::RoutingState::new());
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
                let mut interfaces = Vec::new();
                let routing_state = routing_state.clone();
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
                                        .map(|ip| crate::interface::NetworkInterfaceId {
                                            name: iface.name.clone(),
                                            ip: ip.ip(),
                                        })
                                })
                                .collect();

                            interfaces.retain(|existing_interface: &std::sync::Arc<interface::NetworkInterface>| {
                                let alive = existing_interface.is_alive();
                                if !alive {
                                    tracing::warn!("{} is no longer alive", existing_interface.id);
                                }
                                alive
                            });
                            interfaces.retain(|existing_interface: &std::sync::Arc<interface::NetworkInterface>| {
                                let retain = ipv4_interfacse
                                    .iter()
                                    .any(|current_id| &existing_interface.id == current_id);
                                if !retain {
                                    tracing::info!("Interface {} no longer detected; removing", existing_interface.id);
                                }
                                retain
                            });

                            let new_interface_ids: Vec<_> = ipv4_interfacse
                                .iter()
                                .filter(|new_interface| {
                                    !interfaces
                                        .iter()
                                        .any(|existing_interface| &existing_interface.id == *new_interface)
                                })
                                .collect();

                            for new_interface_id in new_interface_ids {
                                match interface::NetworkInterface::new(
                                    new_interface_id.clone(),
                                    &warp_config,
                                    tx.clone(),
                                ) {
                                    Ok(new_interface) => interfaces.push(new_interface),
                                    Err(e) => {
                                        tracing::warn!("Failed to create new interface {}: {}", new_interface_id, e)
                                    }
                                }
                            }
                        }
                        routing_state.interfaces_sender().send_replace(interfaces.clone());
                    }
                }
            })
            .unwrap();
        futures.push(interface_scan_task);

        let (outbound_tunnel_payload_publisher, mut outbound_tunnel_payloads) =
            tokio::sync::mpsc::unbounded_channel::<crate::tunnel::OutboundTunnelPayload>();

        let mut tunnel_gates: std::collections::HashMap<
            warp_protocol::messages::TunnelId,
            std::sync::Arc<tunnel::Gate>,
        > = std::collections::HashMap::new();

        for (warp_tunnel_name, warp_tunnel_config) in &self.warp_config.tunnels {
            let tunnel_id = match warp_tunnel_config.tunnel_id {
                Some(id) => warp_protocol::messages::TunnelId::Id(id),
                None => warp_protocol::messages::TunnelId::Name(warp_tunnel_name.to_owned()),
            };

            let gate = tunnel::Gate::new(
                warp_tunnel_name,
                tunnel_id.clone(),
                warp_tunnel_config.gate.clone(),
                warp_tunnel_config.transport.send_deadline,
                outbound_tunnel_payload_publisher.clone(),
            )
            .unwrap();
            tunnel_gates.insert(tunnel_id, gate);
        }
        let tunnel_gates = std::sync::Arc::new(tunnel_gates);

        let override_sender_task = tokio::task::Builder::new()
            .name("Holepunching: peer address override sender")
            .spawn({
                let routing_state = routing_state.clone();
                let peer_cipher = peer_cipher.clone();
                let warp_config = self.warp_config.clone();

                async move {
                    let mut interval = tokio::time::interval(
                        warp_config.interfaces.holepunch_keep_alive_interval
                    );

                    loop {
                        interval.tick().await;

                        let interfaces = routing_state.interfaces();

                        for interface in interfaces.iter() {
                            if !interface.is_alive() {
                                continue;
                            }

                            // Send override message if we know our external address
                            if let Some(external_addr) = interface.get_external_address() {
                                let override_msg =
                                    warp_protocol::messages::PeerAddressOverride { replace: external_addr };

                                if let Ok(data) = override_msg
                                    .encode()
                                    .and_then(|encoded| encoded.encrypt(&peer_cipher))
                                    .and_then(|encrypted| encrypted.to_bytes())
                                {
                                    for peer_addr in routing_state.resolve_peer_addresses(&interface.id.name) {
                                        if let Err(e) = interface.queue_send(data.clone(), &peer_addr, None) {
                                            tracing::event!(
                                                tracing::Level::WARN,
                                                interface = %interface.id,
                                                peer_addr = %peer_addr,
                                                error = %e,
                                                "OVERRIDE_SEND_FAILED"
                                            );
                                        } else {
                                            tracing::event!(
                                                tracing::Level::DEBUG,
                                                interface = %interface.id,
                                                peer_addr = %peer_addr,
                                                replace_addr = %external_addr,
                                                "OVERRIDE_SENT_PERIODIC"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            })
            .unwrap();
        futures.push(override_sender_task);

        let warp_accelerator_task = tokio::task::Builder::new()
            .name("warp-accelerator")
            .spawn({
                let routing_state = routing_state.clone();
                let peer_cipher = peer_cipher.clone();

                async move {
                    while let Some(outbound) = outbound_tunnel_payloads.recv().await {

                        let tracer = outbound.tunnel_payload.tracer;

                        // TODO: Error handle this better
                        let data = outbound
                            .tunnel_payload
                            .encode()
                            .unwrap()
                            .encrypt(&peer_cipher)
                            .unwrap()
                            .to_bytes()
                            .unwrap();

                        // TODO: Here is where we can pick the routes from the cross product of interfaces and peer addresses
                        // TODO: Here is where we can query each interface's send queue size/failure rate etc.
                        for interface in routing_state.interfaces().iter().filter(|interface| interface.is_alive()) {
                            let resolved_addresses = routing_state.resolve_peer_addresses(&interface.id.name);
                            
                            for resolved_address in &resolved_addresses {
                                match interface.queue_send(data.clone(), resolved_address, Some(outbound.deadline)) {
                                    Ok(()) => {
                                        tracing::event!(
                                            tracing::Level::DEBUG,
                                            tracer = tracer,
                                            interface = %interface.id,
                                            resolved_addr = %resolved_address,
                                            "TUNNEL_PAYLOAD_SEND_QUEUED"
                                        );
                                    }
                                    Err(e) => {
                                        tracing::event!(
                                            tracing::Level::WARN,
                                            tracer = tracer,
                                            interface = %interface.id,
                                            resolved_addr = %resolved_address,
                                            error = %e,
                                            "TUNNEL_PAYLOAD_SEND_QUEUE_ERROR"
                                        );
                                    }
                                }
                            }
                        }
                        outbound
                            .completion_notifier
                            .send(())
                            .expect("Tunnel completion listener is not listening");
                    }
                }
            })
            .unwrap();

        futures.push(warp_accelerator_task);

        let rx_processing_task = tokio::task::Builder::new()
            .name("global rx processor")
            .spawn({
                let routing_state = routing_state.clone();
                let warp_config = self.warp_config.clone();
                let warp_map_cipher = warp_map_cipher.clone();
                let tunnel_gates = tunnel_gates.clone();
                async move {
                    while let Some(payload) = rx.recv().await {
                        let rx_start_time = std::time::Instant::now();
                        let queue_length = rx.len();

                        let mut message_index = 0;
                        let mut remaining_buf = payload.data.as_slice();
                        loop {
                            let (msg, buf) = warp_protocol::codec::WireMessage::from_slice(remaining_buf).unwrap();
                            tracing::event!(
                                tracing::Level::DEBUG,
                                interface = payload.receiver_name,
                                from_addr = %payload.from,
                                message_index = message_index,
                                payload_size = payload.data.len(),
                                queue_length = queue_length,
                                "RX_MESSAGE"
                            );

                            match payload.from {
                                from if from == warp_config.warp_map.address => {
                                    let decrypted_wire_msg = msg.decrypt(&warp_map_cipher).unwrap();
                                    match decrypted_wire_msg.message_id {
                                        warp_protocol::messages::RegisterResponse::MESSAGE_ID => {
                                            let register_response: warp_protocol::messages::RegisterResponse =
                                                decrypted_wire_msg.decode().unwrap();

                                            // Update external address for the receiving interface
                                            let interfaces = routing_state.interfaces();
                                            for interface in interfaces.iter() {
                                                if interface.id.name == payload.receiver_name {
                                                    interface.set_external_address(register_response.address);
                                                    break;
                                                }
                                            }

                                            tracing::event!(
                                                tracing::Level::INFO,
                                                interface = payload.receiver_name,
                                                public_address = %register_response.address,
                                                one_way_latency_warp_map = std::time::SystemTime::now()
                                                            .duration_since(register_response.timestamp)
                                                            .map(|duration| duration.as_secs_f32())
                                                            .unwrap_or_else(|e| -e.duration().as_secs_f32()),
                                                round_trip_latency_warp_map = std::time::SystemTime::now()
                                                            .duration_since(register_response.request_timestamp)
                                                            .map(|duration| duration.as_secs_f32())
                                                            .unwrap_or_else(|e| -e.duration().as_secs_f32()),
                                                "MESSAGE_PROCESSED[RegisterResponse]"
                                            );
                                        }
                                        warp_protocol::messages::MappingResponse::MESSAGE_ID => {
                                            let mapping: warp_protocol::messages::MappingResponse =
                                                decrypted_wire_msg.decode().unwrap();
                                            routing_state.handle_mapping_response(&mapping);

                                            tracing::event!(
                                                tracing::Level::INFO,
                                                interface = payload.receiver_name,
                                                peer_addresses = format!("{:?}", mapping.endpoints),
                                                active_overrides = routing_state.active_overrides_count(),
                                                one_way_latency_warp_map = std::time::SystemTime::now()
                                                    .duration_since(mapping.timestamp)
                                                    .map(|duration| duration.as_secs_f32())
                                                    .unwrap_or_else(|e| -e.duration().as_secs_f32()),
                                                "MESSAGE_PROCESSED[MappingResponse]"
                                            );
                                        }
                                        _ => {
                                            tracing::event!(
                                                tracing::Level::WARN,
                                                interface = payload.receiver_name,
                                                "UNKNOWN_MESSAGE_FROM_WARP_MAP"
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
                                                let tunnel_payload: warp_protocol::messages::TunnelPayload =
                                                    decrypted_wire_msg.decode().unwrap();
                                                match tunnel_gates.get(&tunnel_payload.tunnel_id) {
                                                    None => {
                                                        tracing::warn!(
                                                            "Received data at {} for unknown tunnel {:?} from {}",
                                                            &payload.receiver,
                                                            &tunnel_payload.tunnel_id,
                                                            from
                                                        );
                                                    }
                                                    Some(gate) => gate.send_to_application(tunnel_payload).await,
                                                }
                                            }
                                            warp_protocol::messages::PeerAddressOverride::MESSAGE_ID => {
                                                let override_msg: warp_protocol::messages::PeerAddressOverride =
                                                    decrypted_wire_msg.decode().unwrap();

                                                // Update address override for the specific interface that received this message
                                                routing_state.handle_peer_address_override(&override_msg, from, &payload.receiver_name);
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
                                        tracing::info!(
                                            "Received invalid message at {} from {}; ignoring",
                                            &payload.receiver,
                                            from
                                        );
                                    }
                                }
                            }

                            remaining_buf = buf;
                            if remaining_buf.is_empty() {
                                break;
                            }
                            message_index += 1;
                        }

                        // Log total RX processing time for this payload
                        let rx_processing_duration = rx_start_time.elapsed();
                        tracing::event!(
                            tracing::Level::DEBUG,
                            interface = payload.receiver_name,
                            rx_processing_latency_us = rx_processing_duration.as_micros(),
                            "Completed payload processing"
                        );
                    }
                }
            })
            .unwrap();
        futures.push(rx_processing_task);

        // Wait for either tasks to complete or shutdown signal
        use futures::StreamExt;

        tokio::select! {
            _ = futures.next() => {
                panic!("warp terminated unexpectedly")
            }
            _ = &mut self.shutdown => {
                tracing::info!("Graceful shutdown initiated");

                let interfaces = routing_state.interfaces();
                for interface in interfaces.iter() {
                    let deregister_request = warp_protocol::messages::DeregisterRequest {
                        pubkey: self.warp_config.private_key.public_key(),
                        timestamp: std::time::SystemTime::now(),
                    };

                    if let Ok(data) = deregister_request.encode()
                        .and_then(|encoded| encoded.encrypt(&warp_map_cipher))
                        .and_then(|encrypted| encrypted.to_bytes()) {

                        if let Err(e) = interface.queue_send(data, &self.warp_config.warp_map.address, None) {
                            tracing::warn!(
                                interface = %interface.id,
                                error = %e,
                                "INTERFACE_DEREGISTRATION_FAILED"
                            );
                        } else {
                            tracing::info!(
                                interface = %interface.id,
                                "INTERFACE_DEREGISTRATION_SENT"
                            );
                        }
                    }
                }

                // Give a brief moment for deregister messages to be sent
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                tracing::info!("Graceful shutdown complete");
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;

    let stdout_layer = tracing_subscriber::fmt::layer().with_filter(args.verbosity);
    let tokio_console_layer = console_subscriber::spawn();

    tracing_subscriber::registry()
        .with(tokio_console_layer)
        .with(stdout_layer)
        .init();

    rt.block_on(async_main(args))
}

async fn async_main(args: Args) -> anyhow::Result<()> {
    let warp_config: warp_config::WarpConfig =
        toml::from_str(std::fs::read_to_string(args.warp_config_path)?.as_str())?;

    tracing::info!(
        "Public key: {}",
        warp_protocol::crypto::pubkey_to_string(&warp_config.private_key.public_key())
    );

    let (mut warp_core, shutdown) = WarpCore::new(warp_config);

    tokio::spawn(async move {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to register SIGTERM handler");
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
            .expect("Failed to register SIGINT handler");

        tokio::select! {
            _ = sigterm.recv() => {
                tracing::info!("Received SIGTERM, initiating graceful shutdown");
            }
            _ = sigint.recv() => {
                tracing::info!("Received SIGINT, initiating graceful shutdown");
            }
        }

        let _ = shutdown.send(());
    });

    warp_core.run().await;

    Ok(())
}
