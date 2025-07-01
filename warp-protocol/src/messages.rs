// What is the right way to define a protocol like this in Rust?
// Bincode is space-efficient but makes it difficult to ensure forward/backward compatibility.
use warp_protocol_derive::AeadMessage;

#[derive(Debug, Clone, PartialEq, AeadMessage)]
#[message_id = 0x10]
pub struct RegisterRequest {
    #[AeadSerialisation(bincode(with_serde))]
    #[Aead(associated_data)]
    pub pubkey: crate::PublicKey,
    #[Aead(encrypted)]
    pub timestamp: std::time::SystemTime,
}

#[derive(Debug, Clone, PartialEq, AeadMessage)]
#[message_id = 0x11]
pub struct RegisterResponse {
    #[Aead(encrypted)]
    pub address: std::net::SocketAddr,
    #[Aead(encrypted)]
    pub timestamp: std::time::SystemTime,
    #[Aead(encrypted)]
    pub request_timestamp: std::time::SystemTime,
}

#[derive(Debug, Clone, PartialEq, AeadMessage)]
#[message_id = 0x12]
pub struct MappingRequest {
    #[Aead(encrypted)]
    #[AeadSerialisation(bincode(with_serde))]
    pub peer_pubkey: crate::PublicKey,
    #[Aead(encrypted)]
    pub timestamp: std::time::SystemTime,
}

#[derive(Debug, Clone, PartialEq, AeadMessage)]
#[message_id = 0x13]
pub struct MappingResponse {
    #[Aead(encrypted)]
    #[AeadSerialisation(bincode(with_serde))]
    pub peer_pubkey: crate::PublicKey,
    #[Aead(encrypted)]
    pub endpoints: Vec<std::net::SocketAddr>,
    #[Aead(encrypted)]
    pub timestamp: std::time::SystemTime,
}

// TODO: Implement codec::Message on this manually to reduce the size overhead
#[derive(Debug, Clone, PartialEq, AeadMessage)]
#[message_id = 0xF1]
pub struct TunnelPayload {
    // FIXME: Figure out what this represents! Currently its a hash of the tunnel name
    #[Aead(encrypted)]
    pub tunnel_id: [u8; 8],
    #[Aead(encrypted)]
    pub data: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Message;
    use aead::Aead;
    use chacha20poly1305::ChaCha20Poly1305;
    use chacha20poly1305::Key;
    use chacha20poly1305::KeyInit;

    const TEST_KEY: [u8; 32] = [42; 32];

    #[test]
    fn tunnel_payload_overhead_1024_bytes() {
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&TEST_KEY));

        let data = [1; 1024];
        let message = TunnelPayload {
            tunnel_id: [1, 2, 3, 4, 5, 6, 7, 8],
            data: data.into(),
        };
        let wire_bytes = message.encode().unwrap().encrypt(&cipher).unwrap().to_bytes().unwrap();

        assert_eq!(wire_bytes.len(), data.len() + 49); // 49 byte overhead... not great, not terrible?
    }

    #[test]
    fn tunnel_payload_overhead_8_bytes() {
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&TEST_KEY));

        let data = [1; 0];
        let message = TunnelPayload {
            tunnel_id: [1, 2, 3, 4, 5, 6, 7, 8],
            data: data.into(),
        };
        let wire_bytes = message.encode().unwrap().encrypt(&cipher).unwrap().to_bytes().unwrap();

        assert_eq!(wire_bytes.len(), data.len() + 45);
    }
}
