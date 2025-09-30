pub(crate) fn serialize_regex_set<S>(regex_set: &regex::RegexSet, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::Serialize;
    // Get the patterns as strings from the RegexSet
    let patterns: Vec<&str> = regex_set.patterns().iter().map(|s| s.as_str()).collect();
    patterns.serialize(serializer)
}

pub(crate) fn deserialize_regex_set<'de, D>(deserializer: D) -> Result<regex::RegexSet, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let patterns: Vec<String> = Vec::deserialize(deserializer)?;

    for (i, pattern) in patterns.iter().enumerate() {
        if let Err(e) = regex::Regex::new(pattern) {
            return Err(serde::de::Error::custom(format!(
                "Invalid regex pattern at index {i}: '{pattern}' - {e}"
            )));
        }
    }

    regex::RegexSet::new(&patterns).map_err(serde::de::Error::custom)
}

pub(crate) fn deserialize_address<'de, D>(deserializer: D) -> Result<std::net::SocketAddr, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    use std::net::ToSocketAddrs;

    let string = String::deserialize(deserializer)?;
    if let Ok(adresses) = string.to_socket_addrs() {
        adresses
            .filter(|s| s.ip().is_ipv4())
            .next()
            .ok_or_else(|| serde::de::Error::custom(format!("invalid address: {string}")))
    } else {
        Err(serde::de::Error::custom(format!("invalid address: {string}")))
    }
}

pub(crate) fn serialize_private_key<S>(
    private_key: &warp_protocol::PrivateKey,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::Serialize;
    let string = warp_protocol::crypto::privkey_to_string(private_key);
    string.serialize(serializer)
}

pub(crate) fn deserialize_private_key<'de, D>(deserializer: D) -> Result<warp_protocol::PrivateKey, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let string = String::deserialize(deserializer)?;
    warp_protocol::crypto::privkey_from_string(&string).map_err(serde::de::Error::custom)
}

pub(crate) fn serialize_public_key<S>(private_key: &warp_protocol::PublicKey, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::Serialize;
    let string = warp_protocol::crypto::pubkey_to_string(private_key);
    string.serialize(serializer)
}

pub(crate) fn deserialize_public_key<'de, D>(deserializer: D) -> Result<warp_protocol::PublicKey, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let string = String::deserialize(deserializer)?;
    warp_protocol::crypto::pubkey_from_string(&string).map_err(serde::de::Error::custom)
}

// TODO: Make this support values like "100us"/"100ns"/"100ms" etc.
pub(crate) fn serialize_duration<S>(duration: &std::time::Duration, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::Serialize;
    duration.as_secs_f64().serialize(serializer)
}

// TODO: Make this support values like "100us"/"100ns"/"100ms" etc.
pub(crate) fn deserialize_duration<'de, D>(deserializer: D) -> Result<std::time::Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    f64::deserialize(deserializer).map(std::time::Duration::from_secs_f64)
}
