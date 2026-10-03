// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Content digests that decide whether an object must be uploaded again.
//!
//! Every uploaded object carries the digest of the **plaintext** file as the
//! user-metadata header `x-amz-meta-sha256`.  On the next run the digest of
//! the local file is compared with the one `HeadObject` returns, so a file is
//! skipped only when its content is unchanged — never merely because it has
//! the same size.
//!
//! * Plain uploads store the file's SHA-256.
//! * Encrypted uploads store a **keyed** digest instead: HMAC-SHA256 under a
//!   subkey derived from the encryption key.  The bucket therefore never
//!   learns an unkeyed plaintext hash (which would let anyone with read
//!   access confirm a guess about a file's content), and a changed encryption
//!   key changes every digest, which is what makes a key rotation re-upload
//!   every object.
//!
//! Equal digests do reveal that two objects (or two versions of one object)
//! have equal plaintext; that is inherent to deduplicating uploads.

use std::io::Read;
use std::path::Path;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::signing::hex_encode;

type HmacSha256 = Hmac<Sha256>;

/// Name of the user-metadata entry (`x-amz-meta-sha256`).
pub const METADATA_NAME: &str = "sha256";

/// `info` string for deriving the digest key (RFC 5869 HKDF-Expand).
const DIGEST_KEY_INFO: &[u8] = b"github-backup-rust/s3/plaintext-digest/v1";

/// Read buffer size used while hashing files.
const READ_BUFFER_BYTES: usize = 64 * 1024;

/// Derives a 32-byte subkey from `key` for the purpose named by `info`.
///
/// This is HKDF-Expand (RFC 5869, section 2.3) with HMAC-SHA256 for one
/// output block: `T(1) = HMAC-SHA256(PRK, info || 0x01)`.  `key` is the PRK
/// directly — a random 256-bit key is already a uniformly random PRK, so the
/// Extract step is skipped as RFC 5869 section 3.3 allows.  Using a derived
/// subkey keeps the AES key from being used with a second primitive.
pub(crate) fn derive_subkey(key: &[u8; 32], info: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(info);
    mac.update(&[1u8]);
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&mac.finalize().into_bytes());
    out
}

/// Computes the digest of everything `reader` yields: SHA-256 hex, or — with
/// an encryption key — HMAC-SHA256 hex under the derived digest subkey.
///
/// # Errors
///
/// Returns the I/O error of the reader.
pub fn digest_reader<R: Read>(mut reader: R, key: Option<&[u8; 32]>) -> std::io::Result<String> {
    let mut buffer = vec![0u8; READ_BUFFER_BYTES];
    match key {
        None => {
            let mut hasher = Sha256::new();
            loop {
                let n = reader.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buffer[..n]);
            }
            Ok(hex_encode(&hasher.finalize()))
        }
        Some(key) => {
            let subkey = derive_subkey(key, DIGEST_KEY_INFO);
            let mut mac =
                HmacSha256::new_from_slice(&*subkey).expect("HMAC accepts any key length");
            loop {
                let n = reader.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                mac.update(&buffer[..n]);
            }
            Ok(hex_encode(&mac.finalize().into_bytes()))
        }
    }
}

/// Digest of the file at `path` (see [`digest_reader`]).
///
/// # Errors
///
/// Returns the I/O error of opening or reading the file.
pub fn digest_file(path: &Path, key: Option<&[u8; 32]>) -> std::io::Result<String> {
    digest_reader(std::fs::File::open(path)?, key)
}

/// Digest of an in-memory buffer (see [`digest_reader`]).
#[must_use]
pub fn digest_bytes(data: &[u8], key: Option<&[u8; 32]>) -> String {
    digest_reader(data, key).expect("reading from a byte slice cannot fail")
}

/// Constant-time comparison of two hex digests.  Digests that differ in
/// length are unequal.
#[must_use]
pub fn digests_equal(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [0x42; 32];

    #[test]
    fn plain_digest_is_sha256() {
        assert_eq!(
            digest_bytes(b"abc", None),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            digest_bytes(b"", None),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hkdf_expand_matches_rfc_5869_test_case_1() {
        // RFC 5869, Appendix A.1: the first 32 bytes of OKM are T(1).
        let prk: [u8; 32] = [
            0x07, 0x77, 0x09, 0x36, 0x2c, 0x2e, 0x32, 0xdf, 0x0d, 0xdc, 0x3f, 0x0d, 0xc4, 0x7b,
            0xba, 0x63, 0x90, 0xb6, 0xc7, 0x3b, 0xb5, 0x0f, 0x9c, 0x31, 0x22, 0xec, 0x84, 0x4a,
            0xd7, 0xc2, 0xb3, 0xe5,
        ];
        let info = [0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9];
        assert_eq!(
            hex_encode(&*derive_subkey(&prk, &info)),
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf"
        );
    }

    #[test]
    fn subkey_for_the_digest_purpose_is_stable() {
        // Cross-checked with an independent implementation (Python hmac).
        assert_eq!(
            hex_encode(&*derive_subkey(&KEY, DIGEST_KEY_INFO)),
            "cc80a4fa50556df1442f9bd6ad3de488e5b26adbfc0b909e7bc16a0103b96e18"
        );
    }

    #[test]
    fn keyed_digest_is_hmac_under_the_derived_subkey() {
        assert_eq!(
            digest_bytes(b"hello world\n", Some(&KEY)),
            "0b610eb7e154b4f74e7c7393656c5227c53595b320fdda8eeb86adc5c07a375e"
        );
        assert_eq!(
            digest_bytes(b"", Some(&KEY)),
            "9d5ffae5ae6408772afb9f9bcb1297dc71cc5c03a723857aee3cc216ba613594"
        );
    }

    #[test]
    fn keyed_digest_differs_from_the_plain_digest_and_between_keys() {
        let plain = digest_bytes(b"data", None);
        let a = digest_bytes(b"data", Some(&KEY));
        let b = digest_bytes(b"data", Some(&[0x43; 32]));
        assert_ne!(plain, a);
        assert_ne!(a, b);
    }

    #[test]
    fn digest_is_independent_of_read_chunking() {
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let whole = digest_bytes(&data, None);
        // A reader that yields one byte at a time.
        struct Trickle<'a>(&'a [u8]);
        impl Read for Trickle<'_> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0.is_empty() || buf.is_empty() {
                    return Ok(0);
                }
                buf[0] = self.0[0];
                self.0 = &self.0[1..];
                Ok(1)
            }
        }
        assert_eq!(
            digest_reader(Trickle(&data[..5_000]), None).unwrap(),
            digest_bytes(&data[..5_000], None)
        );
        assert_eq!(digest_reader(&data[..], None).unwrap(), whole);
    }

    #[test]
    fn file_digest_matches_buffer_digest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.bin");
        std::fs::write(&path, b"file contents").unwrap();
        assert_eq!(
            digest_file(&path, None).unwrap(),
            digest_bytes(b"file contents", None)
        );
        assert_eq!(
            digest_file(&path, Some(&KEY)).unwrap(),
            digest_bytes(b"file contents", Some(&KEY))
        );
        assert!(digest_file(&dir.path().join("missing"), None).is_err());
    }

    #[test]
    fn digests_equal_compares_content_and_length() {
        assert!(digests_equal("abc", "abc"));
        assert!(!digests_equal("abc", "abd"));
        assert!(!digests_equal("abc", "abcd"));
        assert!(!digests_equal("", "a"));
        assert!(digests_equal("", ""));
    }
}
