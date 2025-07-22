use std::str::FromStr;

fn main() {
    let mut config = warp_config::WarpConfig {
        private_key: warp_protocol::crypto::privkey_from_string("2ZHQBY729J6XEQNT8HFH3P61401VYZXG8AX3ZP4CJA3ZY9XHJZ10")
            .unwrap(),
        interfaces: warp_config::InterfacesConfig {
            interface_scan_interval: 10,
            bind_to_device: Some(false),
            exclusion_patterns: regex::RegexSet::new(vec!["eth.*"]).unwrap(),
            max_consecutive_failures: 10,
        },
        warp_map: warp_config::WarpMapConfig {
            address: std::net::SocketAddr::from_str("1.2.3.4:13116").unwrap(),
            public_key: warp_protocol::crypto::pubkey_from_string(
                "0B2XTQXPMCXTKYFPYR5DY8T61W2186HD569YQWMPTV56E1VH7ZS82",
            )
            .unwrap(),
        },
        far_gate: warp_config::WarpFarGateConfig {
            public_key: warp_protocol::crypto::pubkey_from_string(
                "0AZHJ33TNX8V7BK77W78224TZSM028Q6CARFTR2VRWK2ECBCP6T1Y",
            )
            .unwrap(),
        },
        tunnels: std::collections::BTreeMap::new(),
    };

    config.tunnels.insert(
        "video_streams".to_string(),
        warp_config::WarpTunnelConfig {
            tunnel_id: None,
            gate: warp_config::WarpGateConfig::UnixDomainSocket(warp_config::UnixDomainSocketConfig {
                path: "/tmp/socket".into(),
            }),
            transport: warp_config::WarpTransportConfig {
                redundancy: warp_config::RedundancyConfig {
                    num_shards: 5,
                    required_shards: 3,
                },
                mtu: 1400,
                send_deadline: std::time::Duration::from_millis(10),
                ordered: false,
            },
        },
    );

    config.tunnels.insert(
        "wireguard".to_string(),
        warp_config::WarpTunnelConfig {
            tunnel_id: Some(5),
            gate: warp_config::WarpGateConfig::Loopback(warp_config::LoopbackConfig {
                ipv4: true,
                application_to_gate: 9000,
                gate_to_application: None,
            }),
            transport: warp_config::WarpTransportConfig {
                redundancy: warp_config::RedundancyConfig {
                    num_shards: 5,
                    required_shards: 3,
                },
                mtu: 1400,
                send_deadline: std::time::Duration::from_micros(10),
                ordered: false,
            },
        },
    );

    config.tunnels.insert(
        "control_messages".to_string(),
        warp_config::WarpTunnelConfig {
            tunnel_id: Some(42),
            gate: warp_config::WarpGateConfig::Loopback(warp_config::LoopbackConfig {
                ipv4: true,
                application_to_gate: 9010,
                gate_to_application: Some(9011),
            }),
            transport: warp_config::WarpTransportConfig {
                redundancy: warp_config::RedundancyConfig {
                    num_shards: 5,
                    required_shards: 3,
                },
                mtu: 1400,
                send_deadline: std::time::Duration::from_nanos(10),
                ordered: false,
            },
        },
    );

    println!("{}", toml::to_string(&config).unwrap());
}
