use aead::{Aead, Nonce};

// WARP in ASCII
const PROTOCOL_MAGIC: [u8; 4] = [0x77, 0x61, 0x72, 0x70];

// We can pack multiple of these into a single UDP datagram as they self-describe their size
#[derive(Debug, Clone, bincode::Encode, bincode::Decode)]
pub struct WireMessage {
    pub protocol_magic: [u8; 4],
    // TODO: We waste a byte specifying the length of the nonce; make this a fixed array instead generic on the cipher
    //       This will probably need to wait until rust-crypto replaces all the hybrid-array stuff with const generics
    //       (Or try some trait nonsense to extract the aead::Aead::NonceSize into something that fits in [u8; N])
    pub nonce: Vec<u8>,
    pub encrypted_message: Vec<u8>,
    pub associated_data: Vec<u8>,
}

impl WireMessage {
    pub fn from_slice(slice: &[u8]) -> Result<(Self, &[u8]), crate::DecodeError> {
        let (msg, consumed) = bincode::decode_from_slice(slice, bincode::config::standard())?;
        Ok((msg, &slice[consumed..]))
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, crate::EncodeError> {
        Ok(bincode::encode_to_vec(self, bincode::config::standard())?)
    }

    pub fn decrypt<C: aead::Aead>(self, cipher: &C) -> Result<UnencryptedWireMessage, crate::DecodeError> {
        if self.protocol_magic != PROTOCOL_MAGIC {
            return Err(crate::DecodeError::InvalidMagic);
        }
        let nonce =
            Nonce::<C>::try_from(self.nonce.as_slice()).map_err(|e| crate::DecodeError::InvalidMessageFormat)?;
        let mut plaintext = cipher
            .decrypt(
                &nonce,
                aead::Payload {
                    aad: &self.associated_data,
                    msg: &self.encrypted_message,
                },
            )
            .map_err(|_| crate::DecodeError::Decryption)?;

        let message_id = plaintext.pop().ok_or(crate::DecodeError::InvalidMessageFormat)?; // We stuffed the message id at the end

        Ok(UnencryptedWireMessage {
            message_id,
            private: plaintext,
            public: self.associated_data,
        })
    }
}

#[derive(Debug, Clone)]
pub struct UnencryptedWireMessage {
    pub message_id: u8,
    private: Vec<u8>,
    public: Vec<u8>,
}

impl UnencryptedWireMessage {
    pub fn encrypt<C: aead::Aead>(self, cipher: &C) -> Result<WireMessage, crate::EncodeError> {
        let nonce = C::generate_nonce().map_err(|_| crate::EncodeError::Encryption)?;

        let mut to_be_encrypted = self.private;
        to_be_encrypted.push(self.message_id);

        let encrypted_data = cipher
            .encrypt(
                &nonce,
                aead::Payload {
                    msg: &to_be_encrypted,
                    aad: &self.public,
                },
            )
            .map_err(|_| crate::EncodeError::Encryption)?;

        Ok(WireMessage {
            protocol_magic: PROTOCOL_MAGIC,
            nonce: nonce.to_vec(),
            encrypted_message: encrypted_data,
            associated_data: self.public,
        })
    }
}

#[derive(Debug)]
pub enum MessageParts {
    PublicOnly(Vec<u8>),
    PrivateOnly(Vec<u8>),
    Both { public: Vec<u8>, private: Vec<u8> },
}

pub trait Message: Sized {
    const MESSAGE_ID: u8;

    type AssociatedData;

    fn encode(self) -> Result<UnencryptedWireMessage, crate::EncodeError> {
        let parts = self.split()?;

        let message = match parts {
            MessageParts::PublicOnly(public) => UnencryptedWireMessage {
                message_id: Self::MESSAGE_ID,
                private: Vec::new(),
                public,
            },
            MessageParts::PrivateOnly(private) => UnencryptedWireMessage {
                message_id: Self::MESSAGE_ID,
                private,
                public: Vec::new(),
            },
            MessageParts::Both { public, private } => UnencryptedWireMessage {
                message_id: Self::MESSAGE_ID,
                private,
                public,
            },
        };

        Ok(message)
    }

    fn decode(message: UnencryptedWireMessage) -> Result<Self, crate::DecodeError> {
        if message.message_id != Self::MESSAGE_ID {
            return Err(crate::DecodeError::UnexpectedMessageId(Self::MESSAGE_ID));
        }

        use crate::codec::MessageParts::*;
        let parts = match (&message.public.len(), &message.private.len()) {
            (0, 0) => Err(crate::DecodeError::InvalidMessageFormat),
            (0, _) => Ok(PrivateOnly(message.private)),
            (_, 0) => Ok(PublicOnly(message.public)),
            (_, _) => Ok(Both {
                public: message.public,
                private: message.private,
            }),
        }?;

        Self::from_parts(parts)
    }

    // split() will be implemented by the warp-protocol-derive::AeadMessage based
    // on the #[public] and #[private] fields of the message.
    fn split(self) -> Result<MessageParts, crate::EncodeError>;

    // split() will be implemented by the warp-protocol-derive::AeadMessage based
    // on the #[public] and #[private] fields of the message.
    fn from_parts(parts: MessageParts) -> Result<Self, crate::DecodeError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bincode::Decode;
    use chacha20poly1305::{Key, KeyInit};
    use warp_protocol_derive::AeadMessage;

    #[derive(Debug, Clone, PartialEq, AeadMessage)]
    #[message_id = 1]
    struct PublicOnly {
        #[Aead(associated_data)]
        string: String,
        #[Aead(associated_data)]
        number: u32,
    }

    #[derive(Debug, Clone, PartialEq, AeadMessage)]
    #[message_id = 2]
    struct PrivateOnly {
        #[Aead(encrypted)]
        string: String,
        #[Aead(encrypted)]
        number: u32,
    }

    #[derive(Debug, Clone, PartialEq, AeadMessage)]
    #[message_id = 2]
    struct Mixed {
        #[Aead(encrypted)]
        string: String,
        #[Aead(associated_data)]
        number: u32,
    }

    const TEST_KEY: [u8; 32] = [42; 32]; // I rolled a dice

    #[test]
    fn test_private_only_roundtrip() {
        let cipher = chacha20poly1305::ChaCha20Poly1305::new(Key::from_slice(&TEST_KEY));
        let msg = PrivateOnly {
            string: "The undertakings of pride".to_string(),
            number: 99,
        };

        let unencrypted_msg = msg.clone().encode().unwrap();
        assert_eq!(unencrypted_msg.public.len(), 0);
        assert_ne!(unencrypted_msg.private.len(), 0);

        let encrypted_msg = unencrypted_msg.encrypt(&cipher).unwrap();
        let bytes = encrypted_msg.to_bytes().unwrap();
        let rx_encrypted_msg = WireMessage::from_slice(&bytes).unwrap().0;

        assert_eq!(rx_encrypted_msg.protocol_magic, PROTOCOL_MAGIC);
        assert_ne!(rx_encrypted_msg.encrypted_message.len(), 0);
        assert_eq!(rx_encrypted_msg.associated_data.len(), 0);

        let decrypted_msg = rx_encrypted_msg.decrypt(&cipher).unwrap();
        let reconstructed_msg = PrivateOnly::decode(decrypted_msg).unwrap();
        assert_eq!(reconstructed_msg, msg);
    }

    #[test]
    fn test_public_only_roundtrip() {
        let cipher = chacha20poly1305::ChaCha20Poly1305::new(Key::from_slice(&TEST_KEY));
        let msg = PublicOnly {
            string: "The undertakings of pride".to_string(),
            number: 99,
        };

        let unencrypted_msg = msg.clone().encode().unwrap();
        assert_ne!(unencrypted_msg.public.len(), 0);
        assert_eq!(unencrypted_msg.private.len(), 0);

        let encrypted_msg = unencrypted_msg.encrypt(&cipher).unwrap();
        let bytes = encrypted_msg.to_bytes().unwrap();
        let rx_encrypted_msg = WireMessage::from_slice(&bytes).unwrap().0;

        assert_eq!(rx_encrypted_msg.protocol_magic, PROTOCOL_MAGIC);
        assert_ne!(rx_encrypted_msg.encrypted_message.len(), 0); // The message ID gets written to the private section
        assert_ne!(rx_encrypted_msg.associated_data.len(), 0);

        let decrypted_msg = rx_encrypted_msg.decrypt(&cipher).unwrap();
        let reconstructed_msg = PublicOnly::decode(decrypted_msg).unwrap();
        assert_eq!(reconstructed_msg, msg);
    }

    #[test]
    fn test_mixed_roundtrip() {
        let cipher = chacha20poly1305::ChaCha20Poly1305::new(Key::from_slice(&TEST_KEY));
        let msg = Mixed {
            string: "The undertakings of pride".to_string(),
            number: 99,
        };

        let unencrypted_msg = msg.clone().encode().unwrap();
        assert_ne!(unencrypted_msg.public.len(), 0);
        assert_ne!(unencrypted_msg.private.len(), 0);

        let encrypted_msg = unencrypted_msg.encrypt(&cipher).unwrap();
        let bytes = encrypted_msg.to_bytes().unwrap();
        let rx_encrypted_msg = WireMessage::from_slice(&bytes).unwrap().0;

        assert_eq!(rx_encrypted_msg.protocol_magic, PROTOCOL_MAGIC);
        assert_ne!(rx_encrypted_msg.encrypted_message.len(), 0);
        assert_ne!(rx_encrypted_msg.associated_data.len(), 0);

        let decrypted_msg = rx_encrypted_msg.decrypt(&cipher).unwrap();
        let reconstructed_msg = Mixed::decode(decrypted_msg).unwrap();
        assert_eq!(reconstructed_msg, msg);
    }
}
