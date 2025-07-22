use regex::RegexSet;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::ToSocketAddrs;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WarpConfig {
    #[serde(
        serialize_with = "serialize_private_key",
        deserialize_with = "deserialize_private_key"
    )]
    pub private_key: warp_protocol::PrivateKey, // TODO: Deserialise into the right type with serde
    pub interfaces: InterfacesConfig,
    pub warp_map: WarpMapConfig,
    pub far_gate: WarpFarGateConfig,
    pub tunnels: BTreeMap<String, WarpTunnelConfig>,
}

fn serialize_private_key<S>(private_key: &warp_protocol::PrivateKey, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let string = warp_protocol::crypto::privkey_to_string(private_key);
    string.serialize(serializer)
}

fn deserialize_private_key<'de, D>(deserializer: D) -> Result<warp_protocol::PrivateKey, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let string = String::deserialize(deserializer)?;
    warp_protocol::crypto::privkey_from_string(&string).map_err(serde::de::Error::custom)
}

fn serialize_public_key<S>(private_key: &warp_protocol::PublicKey, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let string = warp_protocol::crypto::pubkey_to_string(private_key);
    string.serialize(serializer)
}

fn deserialize_public_key<'de, D>(deserializer: D) -> Result<warp_protocol::PublicKey, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let string = String::deserialize(deserializer)?;
    warp_protocol::crypto::pubkey_from_string(&string).map_err(serde::de::Error::custom)
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct InterfacesConfig {
    pub interface_scan_interval: u64,
    pub bind_to_device: Option<bool>,
    #[serde(
        serialize_with = "serialize_interface_exclusions",
        deserialize_with = "deserialize_interface_exclusions"
    )]
    pub exclusion_patterns: regex::RegexSet,
    pub max_consecutive_failures: usize,
}

fn serialize_interface_exclusions<S>(regex_set: &RegexSet, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    // Get the patterns as strings from the RegexSet
    let patterns: Vec<&str> = regex_set.patterns().iter().map(|s| s.as_str()).collect();
    patterns.serialize(serializer)
}

fn deserialize_interface_exclusions<'de, D>(deserializer: D) -> Result<RegexSet, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let patterns: Vec<String> = Vec::deserialize(deserializer)?;

    for (i, pattern) in patterns.iter().enumerate() {
        if let Err(e) = regex::Regex::new(pattern) {
            return Err(serde::de::Error::custom(format!(
                "Invalid regex pattern at index {}: '{}' - {}",
                i, pattern, e
            )));
        }
    }

    RegexSet::new(&patterns).map_err(serde::de::Error::custom)
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WarpMapConfig {
    #[serde(deserialize_with = "deserialize_address")]
    pub address: std::net::SocketAddr,
    #[serde(serialize_with = "serialize_public_key", deserialize_with = "deserialize_public_key")]
    pub public_key: warp_protocol::PublicKey,
}

fn deserialize_address<'de, D>(deserializer: D) -> Result<std::net::SocketAddr, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let string = String::deserialize(deserializer)?;
    if let Ok(adresses) = string.to_socket_addrs() {
        adresses
            .filter(|s| s.ip().is_ipv4())
            .next()
            .ok_or_else(|| serde::de::Error::custom(format!("invalid address: {}", string)))
    } else {
        Err(serde::de::Error::custom(format!("invalid address: {}", string)))
    }
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
    #[serde(serialize_with = "serialize_public_key", deserialize_with = "deserialize_public_key")]
    pub public_key: warp_protocol::PublicKey,
}

fn serialize_duration<S>(duration: &std::time::Duration, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    duration.as_secs_f64().serialize(serializer)
}

fn deserialize_duration<'de, D>(deserializer: D) -> Result<std::time::Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    f64::deserialize(deserializer).map(std::time::Duration::from_secs_f64)
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WarpTransportConfig {
    pub redundancy: RedundancyConfig,
    pub mtu: u16,
    pub ordered: bool,

    // TODO: Make this support values like "100us"/"100ns"/"100ms" etc.
    #[serde(serialize_with = "serialize_duration", deserialize_with = "deserialize_duration")]
    pub send_deadline: std::time::Duration,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RedundancyConfig {
    pub num_shards: u8,
    pub required_shards: u8,
}
