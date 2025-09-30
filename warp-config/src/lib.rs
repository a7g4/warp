use std::collections::BTreeMap;

mod serdes;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WarpConfig {
    #[serde(
        serialize_with = "serdes::serialize_private_key",
        deserialize_with = "serdes::deserialize_private_key"
    )]
    pub private_key: warp_protocol::PrivateKey,
    pub interfaces: InterfacesConfig,
    pub warp_map: WarpMapConfig,
    pub far_gate: WarpFarGateConfig,
    pub tunnels: BTreeMap<String, WarpTunnelConfig>,
}

// When a new interface is detected, warp will use it if and only if:
// - it matches at least one inclusion pattern
// - it matches no exclusion pattern
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct InterfacesConfig {
    #[serde(
        serialize_with = "serdes::serialize_duration",
        deserialize_with = "serdes::deserialize_duration"
    )]
    pub interface_scan_interval: std::time::Duration,
    #[serde(
        serialize_with = "serdes::serialize_duration",
        deserialize_with = "serdes::deserialize_duration"
    )]
    pub holepunch_keep_alive_interval: std::time::Duration,
    pub bind_to_device: Option<bool>,
    #[serde(
        serialize_with = "serdes::serialize_regex_set",
        deserialize_with = "serdes::deserialize_regex_set"
    )]
    pub exclusion_patterns: regex::RegexSet,
    #[serde(
        serialize_with = "serdes::serialize_regex_set",
        deserialize_with = "serdes::deserialize_regex_set"
    )]
    pub inclusion_patterns: regex::RegexSet,
    pub max_consecutive_failures: usize,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WarpMapConfig {
    #[serde(deserialize_with = "serdes::deserialize_address")]
    pub address: std::net::SocketAddr,
    #[serde(
        serialize_with = "serdes::serialize_public_key",
        deserialize_with = "serdes::deserialize_public_key"
    )]
    pub public_key: warp_protocol::PublicKey,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WarpTunnelConfig {
    pub gate: WarpGateConfig,
    pub transport: WarpTransportConfig,
    // If tunnel_id is not set, it's string name will be used instead in the transport protocol
    pub tunnel_id: Option<u64>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum WarpGateConfig {
    Loopback(LoopbackConfig),
    UnixDomainSocket(UnixDomainSocketConfig),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct UnixDomainSocketConfig {
    pub path: std::path::PathBuf,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct LoopbackConfig {
    pub ipv4: bool,
    pub application_to_gate: u16,
    // If gate_to_application is None, application data will be sent to the last socket address that
    // sent data to the application_to_gate port
    pub gate_to_application: Option<u16>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WarpFarGateConfig {
    #[serde(
        serialize_with = "serdes::serialize_public_key",
        deserialize_with = "serdes::deserialize_public_key"
    )]
    pub public_key: warp_protocol::PublicKey,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WarpTransportConfig {
    pub redundancy: RedundancyConfig,
    pub mtu: u16,
    pub ordered: bool,

    #[serde(
        serialize_with = "serdes::serialize_duration",
        deserialize_with = "serdes::deserialize_duration"
    )]
    pub send_deadline: std::time::Duration,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RedundancyConfig {
    pub num_shards: u8,
    pub required_shards: u8,
}
