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
#[message_id = 0x14]
pub struct DeregisterRequest {
    #[AeadSerialisation(bincode(with_serde))]
    #[Aead(associated_data)]
    pub pubkey: crate::PublicKey,
    #[Aead(encrypted)]
    pub timestamp: std::time::SystemTime,
}

#[derive(Debug, Clone, PartialEq, AeadMessage)]
#[message_id = 0x15]
pub struct DeregisterResponse {
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, bincode::Encode, bincode::Decode)]
pub enum TunnelId {
    Name(String),
    Id(u64),
}

#[derive(Debug, Clone, PartialEq, bincode::Encode, bincode::Decode)]
pub struct MultipartIdentifier {
    parent_tracer: u64,
    num_parts: u64,
    part_id: u64,
}

#[derive(Debug, Clone, PartialEq, bincode::Encode, bincode::Decode, Default)]
pub enum ReconstructionTag {
    #[default]
    Plain,
    Xor(u64, u64), // This payload is the XOR of two other payloads with these tracer ids
    Multipart(MultipartIdentifier),
}

#[derive(Debug, Clone, PartialEq, AeadMessage)]
#[message_id = 0xF1] // Warp at faster than F1 speeds!
pub struct TunnelPayload {
    #[Aead(encrypted)]
    pub tunnel_id: TunnelId,
    #[Aead(Nonce)]
    pub tracer: u64,
    #[Aead(encrypted)]
    pub reconstruction_tag: ReconstructionTag,
    #[Aead(encrypted)]
    pub data: Vec<u8>,
}

impl TunnelPayload {
    pub fn new(tunnel_id: TunnelId, tracer: u64, data: Vec<u8>) -> Self {
        TunnelPayload {
            tunnel_id,
            tracer,
            data,
            reconstruction_tag: ReconstructionTag::Plain,
        }
    }
}

// This message is sent to inform a peer to send to the origin of this message instead of the specified address.
#[derive(Debug, Clone, PartialEq, AeadMessage)]
#[message_id = 0xF2]
pub struct PeerAddressOverride {
    #[Aead(encrypted)]
    pub replace: std::net::SocketAddr,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Message;
    use aead::KeyInit;

    const TEST_KEY: [u8; 32] = [42; 32];

    // This is the lower bound of the overhead for the tunnel payload:
    // - 12 bytes: nonce (encrytion)
    // - 16 bytes: aead tag (MAC-ish thing)
    // - 01 bytes: message id
    // - 01 bytes: tunnel id
    // - 01 bytes: reconstruction tag
    // ----------------------------------------
    // Total: 31 bytes

    #[test]
    fn tunnel_payload_overhead_1024_bytes() {
        let cipher = crate::Cipher::new(&aead::Key::<crate::Cipher>::from(TEST_KEY));
        let data = [1; 1024];
        let message = TunnelPayload::new(TunnelId::Id(0), 0, data.to_vec());
        let wire_bytes = message.encode().unwrap().encrypt(&cipher).unwrap().to_bytes().unwrap();

        assert_eq!(wire_bytes.len(), data.len() + 39);
    }

    #[test]
    fn tunnel_payload_overhead_8_bytes() {
        let cipher = crate::Cipher::new(&aead::Key::<crate::Cipher>::from(TEST_KEY));

        let data = [1; 8];
        let message = TunnelPayload::new(TunnelId::Id(0), 0, data.to_vec());

        let wire_bytes = message.encode().unwrap().encrypt(&cipher).unwrap().to_bytes().unwrap();

        assert_eq!(wire_bytes.len(), data.len() + 35);
    }

    #[test]
    fn test_tunnel_payload_uses_tracer_as_nonce() {
        use crate::codec::Message;
        use aead::KeyInit;

        const NONCE: u64 = 42;

        let cipher = crate::Cipher::new(&aead::Key::<crate::Cipher>::from(TEST_KEY));
        let data = vec![1, 2, 3, 4, 5];
        let message = TunnelPayload::new(TunnelId::Id(42), NONCE, data.clone());

        // Test that extract_nonce returns the tracer bytes
        let mut extracted_nonce = None;
        let has_nonce = message
            .with_nonce_bytes(|bytes| {
                extracted_nonce = Some(bytes.to_vec());
                Ok(())
            })
            .unwrap();
        assert!(has_nonce);
        assert!(extracted_nonce.is_some());
        let nonce_bytes = extracted_nonce.unwrap();
        assert_eq!(nonce_bytes.as_slice(), &message.tracer.to_le_bytes());

        // Test encryption with custom nonce from tracer (now handled automatically)
        let encrypted_msg = message.clone().encode().unwrap().encrypt(&cipher).unwrap();
        let bytes = encrypted_msg.to_bytes().unwrap();
        let rx_encrypted_msg = crate::codec::WireMessage::from_slice(&bytes).unwrap().0;

        // The nonce should start with our tracer bytes (first 8 bytes)
        assert_eq!(&rx_encrypted_msg.nonce[..8], &message.tracer.to_le_bytes());

        // Verify the message can be decrypted and reconstructed
        let decrypted_msg = rx_encrypted_msg.decrypt(&cipher).unwrap();
        let reconstructed_msg: TunnelPayload = decrypted_msg.decode().unwrap();

        // The reconstructed message should have the original data
        assert_eq!(reconstructed_msg.tunnel_id, message.tunnel_id);
        assert_eq!(reconstructed_msg.reconstruction_tag, message.reconstruction_tag);
        assert_eq!(reconstructed_msg.data, message.data);
        // The tracer field retains its original value during reconstruction since it's a nonce field
        assert_eq!(reconstructed_msg.tracer, NONCE);
    }
}
