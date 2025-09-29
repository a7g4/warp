use aead::AeadCore;

pub const NONCE_SIZE: usize = <<crate::Cipher as AeadCore>::NonceSize as aead::array::typenum::Unsigned>::USIZE;

/// Trait for types that can be converted to nonce bytes without allocation
pub trait Nonceable {
    type Output<'a>: AsRef<[u8]>
    where
        Self: 'a;
    fn as_nonce_bytes<'a>(&'a self) -> Self::Output<'a>;
    fn from_nonce_bytes<'a>(nonce: Self::Output<'a>) -> Self;
}

impl Nonceable for u64 {
    type Output<'a> = [u8; 8];
    fn as_nonce_bytes<'a>(&'a self) -> Self::Output<'a> {
        self.to_ne_bytes()
    }

    fn from_nonce_bytes<'a>(nonce: Self::Output<'a>) -> Self {
        u64::from_ne_bytes(nonce)
    }
}

impl Nonceable for u32 {
    type Output<'a> = [u8; 4];
    fn as_nonce_bytes<'a>(&'a self) -> Self::Output<'a> {
        self.to_ne_bytes()
    }

    fn from_nonce_bytes<'a>(nonce: Self::Output<'a>) -> Self {
        u32::from_ne_bytes(nonce)
    }
}

// We can pack multiple of these into a single UDP datagram as they self-describe their size
#[derive(Debug, Clone, bincode::Encode, bincode::Decode)]
pub struct WireMessage {
    pub nonce: [u8; NONCE_SIZE],
    pub encrypted_message: Vec<u8>,
    pub associated_data: Vec<u8>,
}

impl WireMessage {
    pub fn from_slice(slice: &[u8]) -> Result<(Self, &[u8]), crate::DecodeError> {
        let (msg, consumed) = bincode::decode_from_slice(slice, crate::BINCODE_CONFIG)?;
        Ok((msg, &slice[consumed..]))
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, crate::EncodeError> {
        Ok(bincode::encode_to_vec(self, crate::BINCODE_CONFIG)?)
    }

    // Warning! This has not been authenticated! Make sure to decrypt the message before trusting it's contents
    pub fn decode_public<M: Message>(self) -> Result<M::AssociatedData, crate::DecodeError>
    where
        <M as Message>::AssociatedData: bincode::Decode<()>,
    {
        let (associated_data, read_size) = bincode::decode_from_slice(&self.associated_data, crate::BINCODE_CONFIG)?;
        if read_size != self.associated_data.len() {
            // The associated_data bytes should only contain the associated data; nothing else
            Err(crate::DecodeError::InvalidMessageFormat)
        } else {
            Ok(associated_data)
        }
    }

    pub fn decrypt(self, cipher: &crate::Cipher) -> Result<UnencryptedWireMessage, crate::DecodeError> {
        use aead::Aead;
        let nonce = aead::Nonce::<crate::Cipher>::from(self.nonce);
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
            nonce: nonce.into(),
            public: self.associated_data,
            secret: plaintext,
        })
    }
}

#[derive(Debug, Clone)]
pub struct UnencryptedWireMessage {
    pub message_id: u8,
    pub nonce: [u8; NONCE_SIZE],
    public: Vec<u8>,
    secret: Vec<u8>,
}

impl UnencryptedWireMessage {
    pub fn encrypt(self, cipher: &crate::Cipher) -> Result<WireMessage, crate::EncodeError> {
        use aead::Aead;
        let mut to_be_encrypted = self.secret;
        to_be_encrypted.push(self.message_id);

        let encrypted_data = cipher
            .encrypt(
                &self.nonce.into(),
                aead::Payload {
                    msg: &to_be_encrypted,
                    aad: &self.public,
                },
            )
            .map_err(|_| crate::EncodeError::Encryption)?;

        Ok(WireMessage {
            nonce: self.nonce,
            encrypted_message: encrypted_data,
            associated_data: self.public,
        })
    }

    pub fn decode<M: Message>(&self) -> Result<M, crate::DecodeError> {
        if self.message_id != M::MESSAGE_ID {
            return Err(crate::DecodeError::UnexpectedMessageId(self.message_id));
        }
        Ok(M::from_parts(&self.nonce, &self.public, &self.secret))
    }
}

pub trait Message: Sized {
    const MESSAGE_ID: u8;

    type AssociatedData;

    fn encode(self) -> Result<UnencryptedWireMessage, crate::EncodeError> {
        let mut final_nonce = [0u8; NONCE_SIZE];
        let has_custom_nonce = self.with_nonce_bytes(|nonce_bytes| {
            if nonce_bytes.len() >= final_nonce.len() {
                final_nonce.copy_from_slice(&nonce_bytes[..NONCE_SIZE]);
            } else {
                final_nonce[..nonce_bytes.len()].copy_from_slice(nonce_bytes);
                let random_nonce = crate::Cipher::generate_nonce().map_err(|_| crate::EncodeError::Encryption)?;
                let remaining_len = final_nonce.len() - nonce_bytes.len();
                final_nonce[nonce_bytes.len()..].copy_from_slice(&random_nonce.as_slice()[..remaining_len]);
            }
            Ok(())
        })?;

        let nonce = if has_custom_nonce {
            aead::Nonce::<crate::Cipher>::from(final_nonce)
        } else {
            // No custom nonce provided, generate a random one
            crate::Cipher::generate_nonce().map_err(|_| crate::EncodeError::Encryption)?
        };

        Ok(UnencryptedWireMessage {
            message_id: Self::MESSAGE_ID,
            nonce: nonce.into(),
            public: self.public_bytes()?,
            secret: self.secret_bytes()?,
        })
    }

    // with_nonce_bytes() will be implemented by the warp-protocol-derive::AeadMessage to extract nonce bytes from a field marked with #[Aead(Nonce)], if present.
    // The function approach avoids allocations by passing the bytes directly to the closure
    // Returns true if the function was called (i.e., there's a custom nonce), false otherwise
    fn with_nonce_bytes<F, R>(&self, f: F) -> Result<bool, crate::EncodeError>
    where
        F: FnOnce(&[u8]) -> Result<R, crate::EncodeError>;

    // This will be implemented by the warp-protocol-derive::AeadMessage based on the #[public] fields of the message
    fn public_bytes(&self) -> Result<Vec<u8>, crate::EncodeError>;

    // This will be implemented by the warp-protocol-derive::AeadMessage based on the #[private] fields of the message
    fn secret_bytes(&self) -> Result<Vec<u8>, crate::EncodeError>;

    // This will be implemented by the warp-protocol-derive::AeadMessage as the "inverse" of public_bytes() and private_bytes()
    fn from_parts(nonce: &[u8; NONCE_SIZE], public_bytes: &[u8], secret_bytes: &[u8]) -> Self;
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[derive(Debug, Clone, PartialEq, AeadMessage)]
    #[message_id = 3]
    struct WithCustomNonce {
        #[Aead(encrypted)]
        data: String,
        #[Aead(Nonce)]
        custom_nonce: u64,
    }

    const TEST_KEY: [u8; 32] = [42; 32]; // I rolled a dice

    #[test]
    fn test_private_only_roundtrip() {
        use aead::KeyInit;
        let cipher = crate::Cipher::new(&aead::Key::<crate::Cipher>::from(TEST_KEY));
        let msg = PrivateOnly {
            string: "The undertakings of pride".to_string(),
            number: 99,
        };

        let encrypted_msg = msg.clone().encode().unwrap().encrypt(&cipher).unwrap();
        let bytes = encrypted_msg.to_bytes().unwrap();
        let rx_encrypted_msg = WireMessage::from_slice(&bytes).unwrap().0;

        assert_ne!(rx_encrypted_msg.encrypted_message.len(), 0);
        assert_eq!(rx_encrypted_msg.associated_data.len(), 0);

        let decrypted_msg = rx_encrypted_msg.decrypt(&cipher).unwrap();
        let reconstructed_msg: PrivateOnly = decrypted_msg.decode().unwrap();
        assert_eq!(reconstructed_msg, msg);
    }

    #[test]
    fn test_public_only_roundtrip() {
        use aead::KeyInit;
        let cipher = crate::Cipher::new(&aead::Key::<crate::Cipher>::from(TEST_KEY));
        let msg = PublicOnly {
            string: "The undertakings of pride".to_string(),
            number: 99,
        };

        let encrypted_msg = msg.clone().encode().unwrap().encrypt(&cipher).unwrap();
        let bytes = encrypted_msg.to_bytes().unwrap();
        let rx_encrypted_msg = WireMessage::from_slice(&bytes).unwrap().0;

        assert_ne!(rx_encrypted_msg.encrypted_message.len(), 0); // The message ID gets written to the private section
        assert_ne!(rx_encrypted_msg.associated_data.len(), 0);

        let decrypted_msg = rx_encrypted_msg.decrypt(&cipher).unwrap();
        let reconstructed_msg: PublicOnly = decrypted_msg.decode().unwrap();
        assert_eq!(reconstructed_msg, msg);
    }

    #[test]
    fn test_mixed_roundtrip() {
        use aead::KeyInit;
        let cipher = crate::Cipher::new(&aead::Key::<crate::Cipher>::from(TEST_KEY));
        let msg = Mixed {
            string: "The undertakings of pride".to_string(),
            number: 99,
        };

        let encrypted_msg = msg.clone().encode().unwrap().encrypt(&cipher).unwrap();
        let bytes = encrypted_msg.to_bytes().unwrap();
        let rx_encrypted_msg = WireMessage::from_slice(&bytes).unwrap().0;

        assert_ne!(rx_encrypted_msg.encrypted_message.len(), 0);
        assert_ne!(rx_encrypted_msg.associated_data.len(), 0);

        let decrypted_msg = rx_encrypted_msg.decrypt(&cipher).unwrap();
        let reconstructed_msg: Mixed = decrypted_msg.decode().unwrap();
        assert_eq!(reconstructed_msg, msg);
    }

    #[test]
    fn test_custom_nonce_roundtrip() {
        use aead::KeyInit;
        let cipher = crate::Cipher::new(&aead::Key::<crate::Cipher>::from(TEST_KEY));
        let msg = WithCustomNonce {
            data: "Test data with custom nonce".to_string(),
            custom_nonce: 0x1234567890ABCDEF,
        };

        let mut extracted_nonce = None;
        let has_nonce = msg
            .with_nonce_bytes(|bytes| {
                extracted_nonce = Some(bytes.to_vec());
                Ok(())
            })
            .unwrap();
        assert!(has_nonce);
        assert!(extracted_nonce.is_some());
        let nonce_bytes = extracted_nonce.unwrap();
        assert_eq!(nonce_bytes.as_slice(), &0x1234567890ABCDEFu64.to_le_bytes());

        let encrypted_msg = msg.clone().encode().unwrap().encrypt(&cipher).unwrap();
        let bytes = encrypted_msg.to_bytes().unwrap();
        let (rx_encrypted_msg, remaining_bytes) = WireMessage::from_slice(&bytes).unwrap();
        assert!(remaining_bytes.is_empty());

        assert_eq!(&rx_encrypted_msg.nonce[..8], &0x1234567890ABCDEFu64.to_le_bytes());

        let decrypted_msg = rx_encrypted_msg.decrypt(&cipher).unwrap();
        let reconstructed_msg: WithCustomNonce = decrypted_msg.decode().unwrap();

        // The reconstructed message should have the original data and retain nonce field
        assert_eq!(reconstructed_msg.data, msg.data);
        // The nonce field retains its original value during reconstruction
        assert_eq!(reconstructed_msg.custom_nonce, 0x1234567890ABCDEFu64);
    }
}
