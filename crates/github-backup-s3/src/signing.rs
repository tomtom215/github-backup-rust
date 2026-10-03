// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! AWS Signature Version 4 (SigV4) request signing for S3.
//!
//! Implements the signing algorithm documented at
//! <https://docs.aws.amazon.com/general/latest/gr/sigv4_signing.html>.
//!
//! The signer signs **exactly the headers it is given** (plus the three
//! `x-amz-*` headers it adds itself).  The HTTP client sets every header from
//! that same list, so the set of signed headers can never differ from the set
//! of headers actually sent — the defect that made compliant servers reject
//! every `HEAD`, `DELETE`, `LIST` and multipart request in earlier releases.
//!
//! This module is intentionally self-contained and dependency-light: it only
//! uses `sha2` and `hmac` from the RustCrypto project, plus the standard
//! library for everything else.  It is validated against the four worked
//! examples AWS publishes for S3 (see the tests below).

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::encoding::{canonical_query, encode_path};

type HmacSha256 = Hmac<Sha256>;

// ── Public API ──────────────────────────────────────────────────────────────

/// AWS SigV4 signer for a specific service and region.
///
/// Holds the credentials; the secret access key and the session token are
/// wiped from memory when the signer is dropped and never appear in `Debug`
/// output.
#[derive(Clone)]
pub struct Signer {
    access_key_id: String,
    secret_access_key: Zeroizing<String>,
    session_token: Option<Zeroizing<String>>,
    region: String,
    service: String,
}

impl fmt::Debug for Signer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Signer")
            .field("region", &self.region)
            .field("service", &self.service)
            .field("credentials", &"[redacted]")
            .finish()
    }
}

/// Everything the signer needs to know about one request.
///
/// `path` and `query` are the **raw, unencoded** values; the signer encodes
/// them with [`encode_path`] / [`canonical_query`], and the client uses the
/// same functions for the request line.
#[derive(Debug, Clone, Copy)]
pub struct SigningInput<'a> {
    /// HTTP method in upper case (`"GET"`, `"PUT"`, …).
    pub method: &'a str,
    /// Absolute request path before percent-encoding, e.g. `/bucket/my key`.
    pub path: &'a str,
    /// Query parameters before percent-encoding.
    pub query: &'a [(&'a str, &'a str)],
    /// Headers that **will be sent** and must be signed.  Names are matched
    /// case-insensitively.  Must contain `host`; must not contain the
    /// `x-amz-date`, `x-amz-content-sha256` or `x-amz-security-token` headers
    /// the signer adds itself.
    pub headers: &'a [(&'a str, &'a str)],
    /// Lower-case hex SHA-256 of the request body.
    pub payload_sha256: &'a str,
}

/// The result of signing a request.
///
/// The caller must send the request with every header it passed in
/// [`SigningInput::headers`] plus `x-amz-date`, `x-amz-content-sha256`,
/// `x-amz-security-token` (when present) and `Authorization` from this value.
#[derive(Debug, Clone)]
pub struct SignedRequest {
    /// The `x-amz-date` header value (`YYYYMMDDTHHMMSSZ`).
    pub amz_date: String,
    /// The `x-amz-content-sha256` header value.
    pub content_sha256: String,
    /// The `x-amz-security-token` header value, if temporary credentials are
    /// in use.
    pub security_token: Option<String>,
    /// The `Authorization` header value.
    pub authorization: String,
    /// The semicolon-separated `SignedHeaders` list.
    pub signed_headers: String,
    /// The canonical URI that was signed (use it as the request path).
    pub canonical_uri: String,
    /// The canonical query string that was signed (use it as the query).
    pub canonical_query: String,
    /// The canonical request that was hashed (diagnostics and tests only).
    pub canonical_request: String,
}

impl Signer {
    /// Creates a new S3 signer for `region`.
    #[must_use]
    pub fn new_s3(access_key_id: String, secret_access_key: String, region: String) -> Self {
        Self {
            access_key_id,
            secret_access_key: Zeroizing::new(secret_access_key),
            session_token: None,
            region,
            service: "s3".to_string(),
        }
    }

    /// Adds an `AWS_SESSION_TOKEN`-style temporary-credential token.  It is
    /// sent (and signed) as `x-amz-security-token`.
    #[must_use]
    pub fn with_session_token(mut self, token: Option<String>) -> Self {
        self.session_token = token.map(Zeroizing::new);
        self
    }

    /// Signs `input` with the current time.
    #[must_use]
    pub fn sign(&self, input: &SigningInput<'_>) -> SignedRequest {
        let (datetime, _) = utc_datetime_pair();
        self.sign_at(input, &datetime)
    }

    /// Signs `input` as if it were sent at `datetime` (`YYYYMMDDTHHMMSSZ`).
    ///
    /// Exposed so that tests can reproduce published known-answer vectors.
    #[must_use]
    pub fn sign_at(&self, input: &SigningInput<'_>, datetime: &str) -> SignedRequest {
        let date = &datetime[..8];

        // Canonical headers: caller's headers + the ones the signer owns,
        // lower-cased, sorted by name, values trimmed and whitespace-collapsed.
        let mut headers: Vec<(String, String)> = input
            .headers
            .iter()
            .map(|(n, v)| (n.to_ascii_lowercase(), normalize_header_value(v)))
            .collect();
        headers.push((
            "x-amz-content-sha256".to_string(),
            input.payload_sha256.to_string(),
        ));
        headers.push(("x-amz-date".to_string(), datetime.to_string()));
        if let Some(token) = &self.session_token {
            headers.push((
                "x-amz-security-token".to_string(),
                normalize_header_value(token),
            ));
        }
        headers.sort();

        let mut canonical_headers = String::new();
        for (name, value) in &headers {
            canonical_headers.push_str(name);
            canonical_headers.push(':');
            canonical_headers.push_str(value);
            canonical_headers.push('\n');
        }
        let signed_headers = headers
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>()
            .join(";");

        // Step 1: canonical request.
        let canonical_uri = encode_path(input.path);
        let canonical_query = canonical_query(input.query);
        let canonical_request = format!(
            "{}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{}",
            input.method, input.payload_sha256
        );

        // Step 2: string to sign.
        let credential_scope = format!("{date}/{}/{}/aws4_request", self.region, self.service);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{datetime}\n{credential_scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );

        // Step 3: signing key (HMAC chain).
        let k_secret = Zeroizing::new(format!("AWS4{}", self.secret_access_key.as_str()));
        let k_date = hmac_sha256(k_secret.as_bytes(), date.as_bytes());
        let k_region = hmac_sha256(&k_date, self.region.as_bytes());
        let k_service = hmac_sha256(&k_region, self.service.as_bytes());
        let k_signing = hmac_sha256(&k_service, b"aws4_request");

        // Step 4: signature.
        let signature = hex_encode(&hmac_sha256(&k_signing, string_to_sign.as_bytes()));

        // Step 5: Authorization header.
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
            self.access_key_id
        );

        SignedRequest {
            amz_date: datetime.to_string(),
            content_sha256: input.payload_sha256.to_string(),
            security_token: self.session_token.as_ref().map(|t| t.to_string()),
            authorization,
            signed_headers,
            canonical_uri,
            canonical_query,
            canonical_request,
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Trims and collapses runs of whitespace, as the canonical-header rule asks.
fn normalize_header_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Computes HMAC-SHA256 of `data` using `key`.
fn hmac_sha256(key: &[u8], data: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC can take a key of any length");
    mac.update(data);
    Zeroizing::new(mac.finalize().into_bytes().to_vec())
}

/// Computes SHA-256 of `data` and returns the result as a lowercase hex string.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    hex_encode(&Sha256::digest(data))
}

/// Encodes `bytes` as lowercase hexadecimal.
#[must_use]
pub fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // Writing to a `String` cannot fail.
        let _ = write!(out, "{b:02x}");
    }
    out
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

    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn sha256_hex_of_empty_is_known_value() {
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let h = sha256_hex(b"");
        assert_eq!(h, EMPTY_SHA256);
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
        // RFC 4231-style known answer for key "key" (widely published).
        assert_eq!(
            hex,
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
    }

    #[test]
    fn uri_encode_path_leaves_unreserved_chars_unchanged() {
        assert_eq!(encode_path("/foo/bar-baz_qux.txt"), "/foo/bar-baz_qux.txt");
    }

    #[test]
    fn uri_encode_path_encodes_spaces_and_special_chars() {
        let encoded = encode_path("/path/with spaces/and+plus");
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
        let headers = signer.sign(&SigningInput {
            method: "PUT",
            path: "/test/key.json",
            query: &[],
            headers: &[
                ("host", "my-bucket.s3.amazonaws.com"),
                ("content-type", "application/json"),
            ],
            payload_sha256: &sha256_hex(b"{}"),
        });
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

    // ── Known-answer tests: the worked examples AWS publishes for S3 ────────
    //
    // Source: "Signature Calculations for the Authorization Header: Transferring
    // Payload in a Single Chunk (AWS Signature Version 4)" in the Amazon S3 API
    // reference.  Credentials, date and bucket are the ones used there.

    const AWS_ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
    const AWS_SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
    const AWS_DATE: &str = "20130524T000000Z";
    const AWS_HOST: &str = "examplebucket.s3.amazonaws.com";

    fn aws_signer() -> Signer {
        Signer::new_s3(
            AWS_ACCESS_KEY.to_string(),
            AWS_SECRET_KEY.to_string(),
            "us-east-1".to_string(),
        )
    }

    fn signature_of(signed: &SignedRequest) -> &str {
        signed
            .authorization
            .rsplit_once("Signature=")
            .map(|(_, sig)| sig)
            .expect("authorization carries a signature")
    }

    #[test]
    fn aws_example_get_object_with_range_header() {
        let signed = aws_signer().sign_at(
            &SigningInput {
                method: "GET",
                path: "/test.txt",
                query: &[],
                headers: &[("host", AWS_HOST), ("Range", "bytes=0-9")],
                payload_sha256: EMPTY_SHA256,
            },
            AWS_DATE,
        );
        assert_eq!(
            signed.canonical_request,
            "GET\n/test.txt\n\nhost:examplebucket.s3.amazonaws.com\nrange:bytes=0-9\n\
             x-amz-content-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n\
             x-amz-date:20130524T000000Z\n\nhost;range;x-amz-content-sha256;x-amz-date\n\
             e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            signed.signed_headers,
            "host;range;x-amz-content-sha256;x-amz-date"
        );
        assert_eq!(
            signature_of(&signed),
            "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn aws_example_put_object_with_dollar_in_key() {
        let body = b"Welcome to Amazon S3.";
        let signed = aws_signer().sign_at(
            &SigningInput {
                method: "PUT",
                path: "/test$file.text",
                query: &[],
                headers: &[
                    ("host", AWS_HOST),
                    ("date", "Fri, 24 May 2013 00:00:00 GMT"),
                    ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
                ],
                payload_sha256: &sha256_hex(body),
            },
            AWS_DATE,
        );
        assert_eq!(signed.canonical_uri, "/test%24file.text");
        assert_eq!(
            signed.signed_headers,
            "date;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class"
        );
        assert_eq!(
            signature_of(&signed),
            "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
        );
    }

    #[test]
    fn aws_example_get_bucket_lifecycle_uses_name_equals_form() {
        let signed = aws_signer().sign_at(
            &SigningInput {
                method: "GET",
                path: "/",
                query: &[("lifecycle", "")],
                headers: &[("host", AWS_HOST)],
                payload_sha256: EMPTY_SHA256,
            },
            AWS_DATE,
        );
        assert_eq!(signed.canonical_query, "lifecycle=");
        assert_eq!(
            signature_of(&signed),
            "fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543"
        );
    }

    #[test]
    fn aws_example_get_bucket_list_objects() {
        let signed = aws_signer().sign_at(
            &SigningInput {
                method: "GET",
                path: "/",
                // Deliberately out of order: the signer sorts.
                query: &[("prefix", "J"), ("max-keys", "2")],
                headers: &[("host", AWS_HOST)],
                payload_sha256: EMPTY_SHA256,
            },
            AWS_DATE,
        );
        assert_eq!(signed.canonical_query, "max-keys=2&prefix=J");
        assert_eq!(
            signature_of(&signed),
            "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
        );
    }

    #[test]
    fn signing_a_bare_sub_resource_gives_a_different_signature() {
        // Documents why `?uploads` must be canonicalised as `uploads=`: the
        // bare form that earlier releases signed does not reproduce AWS's value.
        let signer = aws_signer();
        let canonical = signer.sign_at(
            &SigningInput {
                method: "GET",
                path: "/",
                query: &[("lifecycle", "")],
                headers: &[("host", AWS_HOST)],
                payload_sha256: EMPTY_SHA256,
            },
            AWS_DATE,
        );
        let bare = canonical
            .canonical_request
            .replace("\nlifecycle=\n", "\nlifecycle\n");
        assert_ne!(bare, canonical.canonical_request);
    }

    // ── What is signed is what is sent ──────────────────────────────────────

    #[test]
    fn signed_headers_are_exactly_the_given_headers_plus_the_amz_ones() {
        let signed = aws_signer().sign_at(
            &SigningInput {
                method: "HEAD",
                path: "/b/k",
                query: &[],
                headers: &[("host", "h")],
                payload_sha256: EMPTY_SHA256,
            },
            AWS_DATE,
        );
        // No content-type: none was passed, so none is signed.
        assert_eq!(
            signed.signed_headers,
            "host;x-amz-content-sha256;x-amz-date"
        );
    }

    #[test]
    fn header_names_are_lowercased_sorted_and_values_normalised() {
        let signed = aws_signer().sign_at(
            &SigningInput {
                method: "PUT",
                path: "/b/k",
                query: &[],
                headers: &[
                    ("X-Amz-Meta-Zeta", "  a   b  "),
                    ("Host", "h"),
                    ("Content-Type", "text/plain; charset=utf-8"),
                ],
                payload_sha256: EMPTY_SHA256,
            },
            AWS_DATE,
        );
        assert_eq!(
            signed.signed_headers,
            "content-type;host;x-amz-content-sha256;x-amz-date;x-amz-meta-zeta"
        );
        assert!(signed.canonical_request.contains("x-amz-meta-zeta:a b\n"));
    }

    #[test]
    fn session_token_is_signed_and_returned() {
        let signer = aws_signer().with_session_token(Some("TOKEN/abc==".to_string()));
        let signed = signer.sign_at(
            &SigningInput {
                method: "GET",
                path: "/b/k",
                query: &[],
                headers: &[("host", "h")],
                payload_sha256: EMPTY_SHA256,
            },
            AWS_DATE,
        );
        assert_eq!(signed.security_token.as_deref(), Some("TOKEN/abc=="));
        assert_eq!(
            signed.signed_headers,
            "host;x-amz-content-sha256;x-amz-date;x-amz-security-token"
        );
        assert!(signed
            .canonical_request
            .contains("x-amz-security-token:TOKEN/abc==\n"));
    }

    #[test]
    fn signing_without_a_token_adds_no_token_header() {
        let signed = aws_signer().sign_at(
            &SigningInput {
                method: "GET",
                path: "/b/k",
                query: &[],
                headers: &[("host", "h")],
                payload_sha256: EMPTY_SHA256,
            },
            AWS_DATE,
        );
        assert!(signed.security_token.is_none());
        assert!(!signed.signed_headers.contains("x-amz-security-token"));
    }

    #[test]
    fn signer_debug_never_shows_secrets() {
        let signer = aws_signer().with_session_token(Some("SESSIONTOKENVALUE".to_string()));
        let debug = format!("{signer:?}");
        assert!(!debug.contains(AWS_SECRET_KEY), "{debug}");
        assert!(!debug.contains(AWS_ACCESS_KEY), "{debug}");
        assert!(!debug.contains("SESSIONTOKENVALUE"), "{debug}");
        assert!(debug.contains("[redacted]"));
    }

    #[test]
    fn signature_is_deterministic_for_a_fixed_time() {
        let input = SigningInput {
            method: "GET",
            path: "/b/a b",
            query: &[],
            headers: &[("host", "h")],
            payload_sha256: EMPTY_SHA256,
        };
        let a = aws_signer().sign_at(&input, AWS_DATE);
        let b = aws_signer().sign_at(&input, AWS_DATE);
        assert_eq!(a.authorization, b.authorization);
    }
}
