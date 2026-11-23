// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! AES-256-GCM at-rest encryption for S3 backup artefacts.
//!
//! When an encryption key is configured via `--encrypt-key`, each file is
//! encrypted before upload and the resulting ciphertext is stored in S3.
//!
//! # Wire format (version 1, written since streaming uploads)
//!
//! ```text
//! [ "GHBKENC" | 0x01 ][ 32-byte random salt ][ segment 0 ] … [ segment n ]
//! ```
//!
//! The payload is the STREAM construction of Hoang, Reyhanitabar, Rogaway and
//! Vizár ("Online Authenticated-Encryption and its Nonce-Reuse
//! Misuse-Resistance", CRYPTO 2015), as implemented by RustCrypto's
//! `aead_stream::EncryptorBE32`, over AES-256-GCM:
//!
//! * The plaintext is cut into 64 KiB segments; each is sealed separately and
//!   carries its own 16-byte tag.  The last segment may be shorter (and is
//!   empty only for an empty file).
//! * The 96-bit GCM nonce of segment *i* is seven zero bytes, the 32-bit
//!   big-endian counter *i*, and a byte that is `1` for the last segment and
//!   `0` otherwise.  Reordering, dropping or truncating segments therefore
//!   fails authentication.
//! * The segment key is **per object**: HKDF-SHA256 (RFC 5869) with the random
//!   salt from the header, the user's key as input keying material and the
//!   `info` string `github-backup-rust/s3/payload/v1`.  A fresh key per object
//!   is what makes the fixed nonce prefix safe, as in the `age` file format's
//!   payload (which uses the same construction with ChaCha20-Poly1305).  The
//!   version byte is part of the `info` string by way of its `v1`, so a blob
//!   relabelled to another version cannot authenticate.
//!
//! Segment-wise sealing is what allows [`EncryptingReader`] to encrypt a file
//! of any size while holding one segment in memory.
//!
//! # Legacy format (version 0, read only)
//!
//! Objects written by earlier versions are a single AES-256-GCM message under
//! the user's key: `[ 12-byte random nonce ][ ciphertext + 16-byte tag ]`.
//! They carry no header, so [`decrypt`] tells the formats apart like this: a
//! blob that does not start with the 8-byte magic is legacy; a blob that does
//! is decrypted as version 1 and, **only if that fails authentication**, as
//! legacy.  A legacy blob starts with a random nonce, so it begins with the
//! magic with probability 2^-64, and even then the fallback reads it
//! correctly.  No output is ever returned that did not authenticate under the
//! key, so the choice cannot produce wrong plaintext.
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
//! Use the tool itself (`github-backup --decrypt …`), or follow the format
//! above with any AES-GCM and HKDF implementation (the book page
//! *Storage / Encryption* has a Python example).  The `openssl enc` command
//! cannot decrypt GCM: it rejects AEAD ciphers.

use std::io::{self, Read};

use aead_stream::{DecryptorBE32, EncryptorBE32, StreamBE32};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::digest::derive_subkey;
use crate::error::S3Error;

/// First bytes of every version-1 object: `GHBKENC` and the version.
pub const MAGIC: [u8; 8] = *b"GHBKENC\x01";

/// Length of the per-object HKDF salt.
const SALT_LEN: usize = 32;

/// Length of the version-1 header (magic and salt).
const HEADER_LEN: usize = MAGIC.len() + SALT_LEN;

/// Plaintext bytes per STREAM segment (64 KiB, as in `age`).
const SEGMENT_LEN: usize = 64 * 1024;

/// GCM authentication tag length.
const TAG_LEN: usize = 16;

/// Nonce length of the legacy single-message format (96 bits).
const LEGACY_NONCE_LEN: usize = 12;

/// HKDF `info` for the per-object payload key.
const PAYLOAD_KEY_INFO: &[u8] = b"github-backup-rust/s3/payload/v1";

/// Size of the object that [`encrypt`] (or an [`EncryptingReader`]) produces
/// for `plaintext_len` bytes of input: header, plaintext, and one tag per
/// segment.
#[must_use]
pub fn encrypted_len(plaintext_len: u64) -> u64 {
    let segments = plaintext_len.div_ceil(SEGMENT_LEN as u64).max(1);
    HEADER_LEN as u64 + plaintext_len + segments * TAG_LEN as u64
}

/// The per-object AES-256-GCM key: HKDF-SHA256(salt, key, [`PAYLOAD_KEY_INFO`]).
///
/// HKDF-Extract is `PRK = HMAC-SHA256(salt, key)` (RFC 5869, section 2.2);
/// HKDF-Expand of one block is [`derive_subkey`].
fn payload_cipher(key: &[u8; 32], salt: &[u8]) -> Aes256Gcm {
    let mut extract =
        <Hmac<Sha256> as KeyInit>::new_from_slice(salt).expect("HMAC accepts any key length");
    extract.update(key);
    let mut prk = Zeroizing::new([0u8; 32]);
    prk.copy_from_slice(&extract.finalize().into_bytes());
    let okm = derive_subkey(&prk, PAYLOAD_KEY_INFO);
    Aes256Gcm::new(&Key::<Aes256Gcm>::from(*okm))
}

/// The STREAM nonce prefix: zero, because every object has its own key.
fn stream_nonce() -> aead_stream::Nonce<Aes256Gcm, StreamBE32<Aes256Gcm>> {
    Default::default()
}

/// A [`Read`] adapter that yields the version-1 encryption of everything its
/// inner reader yields, one 64 KiB segment at a time.
pub struct EncryptingReader<R> {
    inner: R,
    /// `None` once the last segment has been sealed.
    encryptor: Option<EncryptorBE32<Aes256Gcm>>,
    /// Plaintext read ahead: up to one segment plus one byte, which tells a
    /// full segment that is the last from one that is not.
    pending: Vec<u8>,
    /// Ciphertext not yet handed out, starting with the header.
    out: Vec<u8>,
    /// How much of `out` has been handed out.
    pos: usize,
}

impl<R> std::fmt::Debug for EncryptingReader<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptingReader")
            .field("finished", &self.encryptor.is_none())
            .finish_non_exhaustive()
    }
}

impl<R: Read> EncryptingReader<R> {
    /// Encrypts what `inner` yields under `key`, with a fresh random salt.
    ///
    /// # Errors
    ///
    /// Fails only if the operating-system random number generator does.
    pub fn new(key: &[u8; 32], inner: R) -> io::Result<Self> {
        let mut salt = [0u8; SALT_LEN];
        getrandom::fill(&mut salt)
            .map_err(|e| io::Error::other(format!("operating-system RNG failed: {e}")))?;
        let encryptor = EncryptorBE32::from_aead(payload_cipher(key, &salt), &stream_nonce());
        let mut out = Vec::with_capacity(SEGMENT_LEN + TAG_LEN);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&salt);
        Ok(Self {
            inner,
            encryptor: Some(encryptor),
            pending: Vec::with_capacity(SEGMENT_LEN + 1),
            out,
            pos: 0,
        })
    }

    /// The inner reader (for example to finish a digest computed over the
    /// plaintext).
    pub fn into_inner(self) -> R {
        self.inner
    }

    /// Seals the next segment into `out`.  Returns `false` after the last.
    fn seal_next(&mut self) -> io::Result<bool> {
        let Some(encryptor) = self.encryptor.take() else {
            return Ok(false);
        };
        while self.pending.len() <= SEGMENT_LEN {
            let start = self.pending.len();
            self.pending.resize(SEGMENT_LEN + 1, 0);
            match self.inner.read(&mut self.pending[start..]) {
                Ok(0) => {
                    self.pending.truncate(start);
                    break;
                }
                Ok(n) => self.pending.truncate(start + n),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => self.pending.truncate(start),
                Err(e) => {
                    self.pending.truncate(start);
                    self.encryptor = Some(encryptor);
                    return Err(e);
                }
            }
        }
        let failed = |_| io::Error::other("AES-GCM encryption failed");
        self.out.clear();
        self.pos = 0;
        if self.pending.len() > SEGMENT_LEN {
            let mut encryptor = encryptor;
            self.out.extend_from_slice(&self.pending[..SEGMENT_LEN]);
            self.pending.drain(..SEGMENT_LEN);
            encryptor
                .encrypt_next_in_place(&[], &mut self.out)
                .map_err(failed)?;
            self.encryptor = Some(encryptor);
        } else {
            self.out.append(&mut self.pending);
            encryptor
                .encrypt_last_in_place(&[], &mut self.out)
                .map_err(failed)?;
        }
        Ok(true)
    }
}

impl<R: Read> Read for EncryptingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.pos == self.out.len() && !self.seal_next()? {
            return Ok(0);
        }
        let n = buf.len().min(self.out.len() - self.pos);
        buf[..n].copy_from_slice(&self.out[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Encrypts `plaintext` with the version-1 format under `key`.
///
/// # Errors
///
/// Returns [`S3Error::Encrypt`] if the AEAD operation fails (should not happen
/// with valid key material).
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, S3Error> {
    let mut out =
        Vec::with_capacity(usize::try_from(encrypted_len(plaintext.len() as u64)).unwrap_or(0));
    EncryptingReader::new(key, plaintext)
        .and_then(|mut reader| reader.read_to_end(&mut out))
        .map_err(|e| S3Error::Encrypt(e.to_string()))?;
    Ok(out)
}

/// Decrypts an object written by this tool, in either format (see the module
/// documentation for how they are told apart).
///
/// # Errors
///
/// Returns [`S3Error::Encrypt`] if the blob is too short or does not
/// authenticate under `key` (wrong key or corrupted data).
pub fn decrypt(key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>, S3Error> {
    let failed = || S3Error::Encrypt("decryption failed (wrong key or corrupt data)".to_string());
    if blob.starts_with(&MAGIC) {
        if let Some(plaintext) = decrypt_v1(key, blob) {
            return Ok(plaintext);
        }
    }
    if blob.len() < LEGACY_NONCE_LEN + TAG_LEN {
        return Err(S3Error::Encrypt(
            "encrypted blob is too short to be an encrypted object".to_string(),
        ));
    }
    decrypt_legacy(key, blob).ok_or_else(failed)
}

/// Decrypts the version-1 format; `None` unless every segment authenticates.
fn decrypt_v1(key: &[u8; 32], blob: &[u8]) -> Option<Vec<u8>> {
    let body = blob.get(HEADER_LEN..)?;
    let salt = &blob[MAGIC.len()..HEADER_LEN];
    let mut decryptor = DecryptorBE32::from_aead(payload_cipher(key, salt), &stream_nonce());
    let sealed_segment = SEGMENT_LEN + TAG_LEN;
    let mut plaintext = Vec::with_capacity(body.len());
    let mut rest = body;
    while rest.len() > sealed_segment {
        let (segment, tail) = rest.split_at(sealed_segment);
        plaintext.extend_from_slice(&decryptor.decrypt_next(segment).ok()?);
        rest = tail;
    }
    plaintext.extend_from_slice(&decryptor.decrypt_last(rest).ok()?);
    Some(plaintext)
}

/// Decrypts the legacy single-message format.
fn decrypt_legacy(key: &[u8; 32], blob: &[u8]) -> Option<Vec<u8>> {
    let (nonce, ciphertext) = blob.split_at(LEGACY_NONCE_LEN);
    let nonce = Nonce::try_from(nonce).expect("split_at(LEGACY_NONCE_LEN)");
    Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key))
        .decrypt(&nonce, ciphertext)
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const KEY: [u8; 32] = [0x42; 32];

    /// The legacy writer, kept for tests only: blobs it produced must stay
    /// readable.
    fn encrypt_legacy(key: &[u8; 32], nonce: [u8; 12], plaintext: &[u8]) -> Vec<u8> {
        let mut out = nonce.to_vec();
        out.extend(
            Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key))
                .encrypt(&Nonce::from(nonce), plaintext)
                .unwrap(),
        );
        out
    }

    proptest! {
        #[test]
        fn encrypt_decrypt_roundtrip_arbitrary(pt in proptest::collection::vec(any::<u8>(), 0..4096)) {
            let blob = encrypt(&KEY, &pt).expect("encrypt");
            prop_assert_eq!(blob.len() as u64, encrypted_len(pt.len() as u64));
            prop_assert_eq!(decrypt(&KEY, &blob).expect("decrypt"), pt);
        }

        /// Any single-byte change anywhere (header included) is rejected.
        #[test]
        fn tampered_blob_fails_decryption(
            pt in proptest::collection::vec(any::<u8>(), 1..256),
            flip_idx in any::<usize>(),
        ) {
            let mut blob = encrypt(&KEY, &pt).expect("encrypt");
            let idx = flip_idx % blob.len();
            blob[idx] ^= 0x01;
            prop_assert!(decrypt(&KEY, &blob).is_err());
        }
    }

    #[test]
    fn segment_boundaries_round_trip_and_match_encrypted_len() {
        for len in [
            0,
            1,
            SEGMENT_LEN - 1,
            SEGMENT_LEN,
            SEGMENT_LEN + 1,
            3 * SEGMENT_LEN,
            3 * SEGMENT_LEN + 7,
        ] {
            let pt: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let blob = encrypt(&KEY, &pt).unwrap();
            assert_eq!(blob.len() as u64, encrypted_len(len as u64), "len {len}");
            assert_eq!(decrypt(&KEY, &blob).unwrap(), pt, "len {len}");
        }
    }

    #[test]
    fn the_reader_is_independent_of_how_it_is_read() {
        // A reader that yields 3 bytes at a time, read through a 5-byte buffer.
        struct Trickle<'a>(&'a [u8]);
        impl Read for Trickle<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let n = buf.len().min(self.0.len()).min(3);
                buf[..n].copy_from_slice(&self.0[..n]);
                self.0 = &self.0[n..];
                Ok(n)
            }
        }
        let pt: Vec<u8> = (0..(2 * SEGMENT_LEN + 11)).map(|i| (i % 7) as u8).collect();
        let mut reader = EncryptingReader::new(&KEY, Trickle(&pt)).unwrap();
        let mut blob = Vec::new();
        let mut buf = [0u8; 5];
        loop {
            let n = reader.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            blob.extend_from_slice(&buf[..n]);
        }
        assert_eq!(decrypt(&KEY, &blob).unwrap(), pt);
    }

    #[test]
    fn truncation_and_reordering_are_detected() {
        let pt: Vec<u8> = vec![9; 3 * SEGMENT_LEN];
        let blob = encrypt(&KEY, &pt).unwrap();
        let sealed = SEGMENT_LEN + TAG_LEN;
        // Drop the last segment: the new last one was not sealed as last.
        assert!(decrypt(&KEY, &blob[..HEADER_LEN + 2 * sealed]).is_err());
        // Swap segments 0 and 1.
        let mut swapped = blob[..HEADER_LEN].to_vec();
        swapped.extend_from_slice(&blob[HEADER_LEN + sealed..HEADER_LEN + 2 * sealed]);
        swapped.extend_from_slice(&blob[HEADER_LEN..HEADER_LEN + sealed]);
        swapped.extend_from_slice(&blob[HEADER_LEN + 2 * sealed..]);
        assert!(decrypt(&KEY, &swapped).is_err());
    }

    #[test]
    fn every_object_gets_a_fresh_salt() {
        let a = encrypt(&KEY, b"same plaintext").unwrap();
        let b = encrypt(&KEY, b"same plaintext").unwrap();
        assert_eq!(&a[..MAGIC.len()], &MAGIC);
        assert_ne!(&a[MAGIC.len()..HEADER_LEN], &b[MAGIC.len()..HEADER_LEN]);
        assert_ne!(a, b);
    }

    #[test]
    fn legacy_blobs_still_decrypt() {
        let blob = encrypt_legacy(&KEY, [7; 12], b"written by an earlier version");
        assert_eq!(
            decrypt(&KEY, &blob).unwrap(),
            b"written by an earlier version"
        );
    }

    /// A legacy blob whose random nonce happens to begin with the magic is
    /// still read correctly (the version-1 attempt fails authentication).
    #[test]
    fn a_legacy_blob_that_starts_with_the_magic_still_decrypts() {
        let mut nonce = [0u8; 12];
        nonce[..8].copy_from_slice(&MAGIC);
        nonce[8..].copy_from_slice(&[1, 2, 3, 4]);
        let blob = encrypt_legacy(&KEY, nonce, &[5u8; 100]);
        assert!(blob.starts_with(&MAGIC));
        assert_eq!(decrypt(&KEY, &blob).unwrap(), vec![5u8; 100]);
    }

    /// Fixed vector, cross-checked with an independent implementation
    /// (Python `cryptography`: HKDF-SHA256 then AES-GCM per segment).
    #[test]
    fn version_1_matches_an_independent_implementation() {
        let mut blob = MAGIC.to_vec();
        blob.extend_from_slice(&[0x11; SALT_LEN]);
        blob.extend_from_slice(&hex(
            "a80e435d58072a966b04887673fe257df96d0d5385507dd746f262f75aa7a110d135",
        ));
        assert_eq!(decrypt(&KEY, &blob).unwrap(), b"{\"hello\":\"world\"}\n");
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn wrong_key_fails() {
        let blob = encrypt(&KEY, b"secret").unwrap();
        assert!(decrypt(&[0xFF; 32], &blob).is_err());
        let legacy = encrypt_legacy(&KEY, [1; 12], b"secret");
        assert!(decrypt(&[0xFF; 32], &legacy).is_err());
    }

    #[test]
    fn short_blobs_fail() {
        assert!(decrypt(&KEY, &[]).is_err());
        assert!(decrypt(&KEY, &[0u8; 8]).is_err());
        assert!(decrypt(&KEY, &MAGIC).is_err());
    }

    #[test]
    fn errors_name_neither_key_nor_plaintext() {
        let err = decrypt(&KEY, &encrypt(&[1; 32], b"PLAINTEXT").unwrap())
            .unwrap_err()
            .to_string();
        assert!(!err.contains("PLAINTEXT") && !err.contains("4242"), "{err}");
    }
}
