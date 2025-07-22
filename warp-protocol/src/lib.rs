pub mod codec;
pub mod crypto;
pub mod messages;

pub use aead::Aead;

pub type PrivateKey = k256::SecretKey;
pub type PublicKey = k256::PublicKey;
pub type Cipher = chacha20poly1305::ChaCha20Poly1305;

pub const BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard();

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Encoding error: {0}")]
    Encoding(#[from] EncodeError),
    #[error("Decoding error: {0}")]
    Decoding(#[from] DecodeError),
}

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("Bincode encoding error: {0}")]
    Bincode(#[from] bincode::error::EncodeError),
    #[error("Encryption error")]
    Encryption,
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("Bincode decoding error: {0}")]
    Bincode(#[from] bincode::error::DecodeError),
    #[error("Decryption error")]
    Decryption,
    #[error("Key error")]
    KeyError(#[from] k256::elliptic_curve::Error),
    #[error("Invalid message format")]
    InvalidMessageFormat,
    #[error("Unable to decode Base32 string: '{0}'")]
    Base32DecodeError(String),
    #[error("Unexpected message id: expected {0}")]
    UnexpectedMessageId(u8),
    #[error("Unknown message ID: {0}")]
    UnknownMessageId(u8),
}
