pub fn pubkey_to_string(pubkey: &crate::PublicKey) -> String {
    base32::encode(base32::Alphabet::Crockford, &pubkey.to_sec1_bytes())
}
pub fn pubkey_from_string(pubkey: &str) -> Result<crate::PublicKey, crate::DecodeError> {
    let bytes = &base32::decode(base32::Alphabet::Crockford, pubkey)
        .ok_or(crate::DecodeError::Base32DecodeError(pubkey.to_string()))?;
    Ok(crate::PublicKey::from_sec1_bytes(bytes)?)
}

pub fn privkey_to_string(key: &crate::PrivateKey) -> String {
    base32::encode(base32::Alphabet::Crockford, &key.to_bytes())
}

pub fn privkey_from_string(key: &str) -> Result<crate::PrivateKey, crate::DecodeError> {
    let bytes = base32::decode(base32::Alphabet::Crockford, key)
        .ok_or(crate::DecodeError::Base32DecodeError(key.to_string()))?;
    Ok(crate::PrivateKey::from_slice(&bytes)?)
}

pub fn cipher_from_shared_secret(private_key: &crate::PrivateKey, peer_pubkey: &crate::PublicKey) -> crate::Cipher {
    use aead::KeyInit;
    use sha3::Digest;
    let shared_secret =
        k256::elliptic_curve::ecdh::diffie_hellman(private_key.to_nonzero_scalar(), peer_pubkey.as_affine());
    let mut hasher = sha3::Sha3_256::new();
    hasher.update(shared_secret.raw_secret_bytes().as_slice());
    let key = hasher.finalize();

    crate::Cipher::new(&aead::Key::<crate::Cipher>::from(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    use aead::{Aead, AeadCore, Payload};

    #[test]
    fn test_shared_secret() {
        let key_1 = k256::SecretKey::random(&mut rand::rng());
        let key_2 = k256::SecretKey::random(&mut rand::rng());

        assert_ne!(key_1, key_2);

        let cipher_1 = cipher_from_shared_secret(&key_1, &key_2.public_key());
        let cipher_2 = cipher_from_shared_secret(&key_2, &key_1.public_key());

        let nonce = crate::Cipher::generate_nonce()
            .map_err(|_| crate::EncodeError::Encryption)
            .unwrap();

        let original_bytes = &[1; 256];
        let aad = &[2; 256];

        let bytes = cipher_1
            .encrypt(
                &nonce,
                Payload {
                    msg: original_bytes,
                    aad,
                },
            )
            .unwrap();

        let decrypted_bytes = cipher_2.decrypt(&nonce, Payload { msg: &bytes, aad }).unwrap();

        assert_eq!(original_bytes, decrypted_bytes.as_slice());
    }
}
