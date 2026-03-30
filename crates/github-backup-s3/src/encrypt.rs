// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! AES-256-GCM at-rest encryption for S3 backup artefacts.
//!
//! When an encryption key is configured via `--encrypt-key`, each file is
//! encrypted before upload and the resulting ciphertext is stored in S3.
//!
//! # Wire format
//!
//! ```text
//! [ 12-byte random nonce ][ ciphertext + 16-byte GCM tag ]
//! ```
//!
//! The nonce is generated fresh for every file so that encrypting the same
//! plaintext twice produces different ciphertext.  The nonce is stored
//! prepended to the ciphertext so that decryption only needs the key and the
//! stored blob.
//!
//! # Key format
//!
//! Pass a 32-byte key as a 64-character hex string via `--encrypt-key` or the
//! `BACKUP_ENCRYPT_KEY` environment variable.  Generate a random key with:
//!
//! ```bash
//! openssl rand -hex 32
//! ```
//!
//! # Decryption
//!
//! ```bash
//! # Split nonce (first 12 bytes) and ciphertext+tag, then decrypt:
//! dd if=file.json.enc bs=12 count=1 of=nonce.bin
//! dd if=file.json.enc bs=12 skip=1 of=ct.bin
//! openssl enc -d -aes-256-gcm -K <hex_key> -iv <hex_nonce> \
//!   -in ct.bin -out file.json
//! ```

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};

use crate::error::S3Error;

/// Nonce length for AES-256-GCM (96 bits / 12 bytes).
const NONCE_LEN: usize = 12;

/// Encrypts `plaintext` with AES-256-GCM using the provided 32-byte key.
///
/// Returns `[ 12-byte nonce | ciphertext | 16-byte tag ]`.
///
/// # Errors
///
/// Returns [`S3Error::Encrypt`] if the AEAD operation fails (should not happen
/// with valid key material).
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, S3Error> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));

    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| S3Error::Encrypt(e.to_string()))?;

    // Prepend the nonce so the blob is self-contained.
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypts a blob previously produced by [`encrypt`].
///
/// Expects the format `[ 12-byte nonce | ciphertext | 16-byte tag ]`.
///
/// # Errors
///
/// Returns [`S3Error::Encrypt`] if the blob is too short or the AEAD
/// authentication fails (wrong key or corrupted data).
pub fn decrypt(key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>, S3Error> {
    if blob.len() < NONCE_LEN {
        return Err(S3Error::Encrypt(
            "encrypted blob is too short to contain a nonce".to_string(),
        ));
    }
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let nonce = Nonce::from_slice(nonce_bytes);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));

    cipher.decrypt(nonce, ciphertext).map_err(|e| {
        S3Error::Encrypt(format!(
            "decryption failed (wrong key or corrupt data): {e}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn test_key() -> [u8; 32] {
        [0x42u8; 32]
    }

    proptest! {
        /// Verifies that decrypt(encrypt(pt)) == pt for arbitrary plaintext.
        #[test]
        fn encrypt_decrypt_roundtrip_arbitrary(pt in proptest::collection::vec(any::<u8>(), 0..4096)) {
            let key = test_key();
            let blob = encrypt(&key, &pt).expect("encrypt");
            let recovered = decrypt(&key, &blob).expect("decrypt");
            prop_assert_eq!(recovered, pt);
        }

        /// Verifies that a 1-byte bit-flip in the ciphertext causes decryption
        /// to fail (AEAD tag integrity check).
        #[test]
        fn tampered_ciphertext_fails_decryption(
            pt in proptest::collection::vec(any::<u8>(), 1..256),
            flip_idx in any::<usize>(),
        ) {
            let key = test_key();
            let mut blob = encrypt(&key, &pt).expect("encrypt");
            // Flip a byte in the ciphertext region (after the nonce).
            if blob.len() > NONCE_LEN {
                let idx = NONCE_LEN + (flip_idx % (blob.len() - NONCE_LEN));
                blob[idx] ^= 0xFF;
                prop_assert!(
                    decrypt(&key, &blob).is_err(),
                    "tampered ciphertext should fail AEAD authentication"
                );
            }
        }
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = test_key();
        let plaintext = b"hello, encrypted world!";
        let blob = encrypt(&key, plaintext).expect("encrypt");
        let recovered = decrypt(&key, &blob).expect("decrypt");
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn encrypt_produces_different_nonce_each_call() {
        let key = test_key();
        let pt = b"same plaintext";
        let blob1 = encrypt(&key, pt).expect("first encrypt");
        let blob2 = encrypt(&key, pt).expect("second encrypt");
        // The first 12 bytes (nonce) should differ with overwhelming probability.
        assert_ne!(
            &blob1[..NONCE_LEN],
            &blob2[..NONCE_LEN],
            "nonces must be random and unique"
        );
    }

    #[test]
    fn encrypt_output_length() {
        let key = test_key();
        let plaintext = b"test data";
        let blob = encrypt(&key, plaintext).expect("encrypt");
        // nonce (12) + ciphertext (plaintext.len()) + GCM tag (16)
        assert_eq!(blob.len(), NONCE_LEN + plaintext.len() + 16);
    }

    #[test]
    fn decrypt_wrong_key_fails() {
        let key = test_key();
        let bad_key = [0xFFu8; 32];
        let blob = encrypt(&key, b"secret").expect("encrypt");
        assert!(
            decrypt(&bad_key, &blob).is_err(),
            "wrong key should fail AEAD authentication"
        );
    }

    #[test]
    fn decrypt_truncated_blob_fails() {
        let key = test_key();
        assert!(
            decrypt(&key, &[0u8; 8]).is_err(),
            "blob shorter than nonce should fail"
        );
    }

    #[test]
    fn decrypt_empty_blob_fails() {
        let key = test_key();
        assert!(decrypt(&key, &[]).is_err());
    }

    #[test]
    fn encrypt_empty_plaintext() {
        let key = test_key();
        let blob = encrypt(&key, b"").expect("encrypt empty");
        let recovered = decrypt(&key, &blob).expect("decrypt empty");
        assert_eq!(recovered, b"");
    }
}
