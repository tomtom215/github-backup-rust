// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! AWS Signature Version 4 (SigV4) request signing for S3.
//!
//! Implements the signing algorithm documented at:
//! <https://docs.aws.amazon.com/general/latest/gr/sigv4_signing.html>
//!
//! This module is intentionally self-contained and dependency-light: it only
//! uses `sha2` and `hmac` from the RustCrypto project, plus the standard
//! library for everything else.

use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

// ── Public API ──────────────────────────────────────────────────────────────

/// AWS SigV4 signer for a specific service and region.
#[derive(Debug, Clone)]
pub struct Signer {
    access_key_id: String,
    secret_access_key: String,
    region: String,
    service: String,
}

/// A set of additional headers to add to the request for SigV4 signing.
///
/// The caller should add these headers to the HTTP request before sending.
#[derive(Debug, Clone)]
pub struct SignedHeaders {
    /// The `x-amz-date` header value (`YYYYMMDDTHHMMSSZ`).
    pub amz_date: String,
    /// The `x-amz-content-sha256` header value.
    pub content_sha256: String,
    /// The `Authorization` header value.
    pub authorization: String,
}

impl Signer {
    /// Creates a new S3 signer for `region`.
    #[must_use]
    pub fn new_s3(access_key_id: String, secret_access_key: String, region: String) -> Self {
        Self {
            access_key_id,
            secret_access_key,
            region,
            service: "s3".to_string(),
        }
    }

    /// Computes SigV4 signing headers for an S3 `PutObject` request.
    ///
    /// `host` should be the S3 hostname (e.g., `bucket.s3.amazonaws.com`).
    /// `path` should be the URL path (e.g., `/key/to/object`).
    /// `body` is the full request body.
    ///
    /// Returns a [`SignedHeaders`] that the caller must add to the HTTP request.
    #[must_use]
    pub fn sign_put(
        &self,
        host: &str,
        path: &str,
        content_type: &str,
        body: &[u8],
    ) -> SignedHeaders {
        let (datetime, date) = utc_datetime_pair();
        self.sign("PUT", host, path, "", content_type, body, &datetime, &date)
    }

    /// Computes SigV4 signing headers for an arbitrary request.
    ///
    /// - `method`       — HTTP method (`"POST"`, `"DELETE"`, etc.)
    /// - `host`         — Host header value
    /// - `path`         — URL path (e.g. `/key/to/object`)
    /// - `query`        — Pre-encoded query string without `?` (e.g.
    ///   `"partNumber=1&uploadId=abc"`)
    /// - `content_type` — `Content-Type` header value
    /// - `body`         — Request body bytes
    #[must_use]
    pub fn sign_request(
        &self,
        method: &str,
        host: &str,
        path: &str,
        query: &str,
        content_type: &str,
        body: &[u8],
    ) -> SignedHeaders {
        let (datetime, date) = utc_datetime_pair();
        self.sign(
            method,
            host,
            path,
            query,
            content_type,
            body,
            &datetime,
            &date,
        )
    }

    /// Computes SigV4 signing headers for an S3 `HeadObject` or `GetObject`
    /// request (empty body).
    #[must_use]
    pub fn sign_get(&self, host: &str, path: &str) -> SignedHeaders {
        let (datetime, date) = utc_datetime_pair();
        self.sign(
            "HEAD",
            host,
            path,
            "",
            "application/octet-stream",
            b"",
            &datetime,
            &date,
        )
    }

    /// Internal signing implementation.
    ///
    /// `method` is the HTTP method (e.g., `"PUT"`, `"HEAD"`).
    /// `query` is the pre-encoded query string (empty for simple PutObject).
    #[allow(clippy::too_many_arguments)]
    fn sign(
        &self,
        method: &str,
        host: &str,
        path: &str,
        query: &str,
        content_type: &str,
        body: &[u8],
        datetime: &str,
        date: &str,
    ) -> SignedHeaders {
        let payload_hash = sha256_hex(body);

        // Canonical headers — must be sorted by lowercase header name.
        let canonical_headers = format!(
            "content-type:{content_type}\nhost:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{datetime}\n"
        );
        let signed_headers = "content-type;host;x-amz-content-sha256;x-amz-date";

        // Step 1: Canonical Request.
        let canonical_uri = uri_encode_path(path);
        let canonical_request = format!(
            "{method}\n{canonical_uri}\n{query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
        );

        // Step 2: String to Sign.
        let credential_scope = format!("{date}/{}/{}/aws4_request", self.region, self.service);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{datetime}\n{credential_scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );

        // Step 3: Signing Key (HMAC chain).
        let k_secret = format!("AWS4{}", self.secret_access_key);
        let k_date = hmac_sha256(k_secret.as_bytes(), date.as_bytes());
        let k_region = hmac_sha256(&k_date, self.region.as_bytes());
        let k_service = hmac_sha256(&k_region, self.service.as_bytes());
        let k_signing = hmac_sha256(&k_service, b"aws4_request");

        // Step 4: Signature.
        let signature = hex_encode(&hmac_sha256(&k_signing, string_to_sign.as_bytes()));

        // Step 5: Authorization Header.
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
            self.access_key_id
        );

        SignedHeaders {
            amz_date: datetime.to_string(),
            content_sha256: payload_hash,
            authorization,
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Computes HMAC-SHA256 of `data` using `key`.
fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC can take a key of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Computes SHA-256 of `data` and returns the result as a lowercase hex string.
fn sha256_hex(data: &[u8]) -> String {
    hex_encode(&Sha256::digest(data))
}

/// Encodes `bytes` as lowercase hexadecimal.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// URI-encodes a path, encoding all characters except `/` and unreserved
/// ASCII (`A-Z`, `a-z`, `0-9`, `-`, `_`, `.`, `~`).
fn uri_encode_path(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len() * 2);
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                encoded.push(byte as char)
            }
            other => {
                encoded.push('%');
                encoded.push_str(&format!("{other:02X}"));
            }
        }
    }
    encoded
}

/// Returns the current UTC time as `(datetime, date)` where `datetime` is
/// `YYYYMMDDTHHMMSSZ` and `date` is `YYYYMMDD`.
fn utc_datetime_pair() -> (String, String) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let datetime = format_datetime_utc(secs);
    let date = datetime[..8].to_string();
    (datetime, date)
}

/// Formats a Unix timestamp (seconds since epoch) as `YYYYMMDDTHHMMSSZ`.
fn format_datetime_utc(unix_secs: u64) -> String {
    let (year, month, day) = unix_days_to_ymd(unix_secs / 86400);
    let secs_today = unix_secs % 86400;
    let hour = secs_today / 3600;
    let min = (secs_today % 3600) / 60;
    let sec = secs_today % 60;
    format!("{year:04}{month:02}{day:02}T{hour:02}{min:02}{sec:02}Z")
}

/// Converts days since the Unix epoch (`1970-01-01`) to `(year, month, day)`.
///
/// Uses [Howard Hinnant's civil-from-days algorithm][ref].
///
/// [ref]: https://howardhinnant.github.io/date_algorithms.html
fn unix_days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Shift epoch from 1970-01-01 to 0000-03-01 to simplify leap-year math.
    let z: i64 = days as i64 + 719_468;
    let era: i64 = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe: i64 = z - era * 146_097; // day-of-era [0, 146096]
    let yoe: i64 = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y: i64 = yoe + era * 400;
    let doy: i64 = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp: i64 = (5 * doy + 2) / 153; // [0, 11]
    let d: i64 = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m: i64 = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y: i64 = if m <= 2 { y + 1 } else { y };
    (y as u64, m as u64, d as u64)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_of_empty_is_known_value() {
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let h = sha256_hex(b"");
        assert_eq!(
            h,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_hex_is_32_bytes_lowercase_hex() {
        // SHA-256 always produces 32 bytes = 64 hex characters.
        let h = sha256_hex(b"abc");
        assert_eq!(h.len(), 64, "SHA-256 output must be 64 hex chars");
        assert!(h
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn sha256_hex_different_inputs_differ() {
        let h1 = sha256_hex(b"abc");
        let h2 = sha256_hex(b"def");
        assert_ne!(h1, h2, "different inputs must produce different hashes");
    }

    #[test]
    fn hmac_sha256_output_is_32_bytes() {
        let key = b"key";
        let data = b"The quick brown fox jumps over the lazy dog";
        let result = hmac_sha256(key, data);
        assert_eq!(result.len(), 32, "HMAC-SHA256 must produce 32 bytes");
        let hex = hex_encode(&result);
        assert_eq!(hex.len(), 64, "hex encoding of 32 bytes must be 64 chars");
    }

    #[test]
    fn uri_encode_path_leaves_unreserved_chars_unchanged() {
        assert_eq!(
            uri_encode_path("/foo/bar-baz_qux.txt"),
            "/foo/bar-baz_qux.txt"
        );
    }

    #[test]
    fn uri_encode_path_encodes_spaces_and_special_chars() {
        let encoded = uri_encode_path("/path/with spaces/and+plus");
        assert!(encoded.contains("%20"), "space should be %20");
        assert!(encoded.contains("%2B"), "plus should be %2B");
    }

    #[test]
    fn format_datetime_utc_unix_epoch() {
        // 1970-01-01T00:00:00Z = 0 seconds
        assert_eq!(format_datetime_utc(0), "19700101T000000Z");
    }

    #[test]
    fn format_datetime_utc_known_timestamp() {
        // 2024-03-15T12:30:45Z
        // 2024-03-15: days since epoch = 19_797
        // 12*3600 + 30*60 + 45 = 45045 seconds
        let secs = 19_797 * 86400 + 45045;
        assert_eq!(format_datetime_utc(secs), "20240315T123045Z");
    }

    #[test]
    fn unix_days_to_ymd_epoch() {
        assert_eq!(unix_days_to_ymd(0), (1970, 1, 1));
    }

    #[test]
    fn unix_days_to_ymd_known_dates() {
        // 2024-03-15 = 19797 days since epoch
        assert_eq!(unix_days_to_ymd(19_797), (2024, 3, 15));
        // 2000-01-01 = 10957 days since epoch
        assert_eq!(unix_days_to_ymd(10_957), (2000, 1, 1));
        // 2023-12-31 = 19722 days since epoch
        assert_eq!(unix_days_to_ymd(19_722), (2023, 12, 31));
    }

    #[test]
    fn signer_produces_authorization_header() {
        let signer = Signer::new_s3(
            "AKIAIOSFODNN7EXAMPLE".to_string(),
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
            "us-east-1".to_string(),
        );
        let headers = signer.sign_put(
            "my-bucket.s3.amazonaws.com",
            "/test/key.json",
            "application/json",
            b"{}",
        );
        assert!(
            headers.authorization.starts_with("AWS4-HMAC-SHA256"),
            "Authorization header must start with AWS4-HMAC-SHA256"
        );
        assert!(
            headers.authorization.contains("AKIAIOSFODNN7EXAMPLE"),
            "Authorization header must contain access key ID"
        );
        assert_eq!(
            headers.amz_date.len(),
            16,
            "datetime should be YYYYMMDDTHHMMSSZ"
        );
        assert!(
            headers.amz_date.ends_with('Z'),
            "datetime should end with Z"
        );
    }
}
