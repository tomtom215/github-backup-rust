// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Independent AWS Signature Version 4 *verification* for the fake S3 server.
//!
//! This file is written from the AWS specification and deliberately shares no
//! code with `github_backup_s3::signing`.  It rebuilds the canonical request
//! from the request **as it was received** (method, request-target, the headers
//! named in `SignedHeaders`, the hash of the body that actually arrived) and
//! compares the resulting signature with the one in the `Authorization`
//! header, which is exactly what a real S3 server does.  A client that signs
//! something other than what it sends is therefore rejected here.
//!
//! The implementation is itself validated against the four worked examples AWS
//! publishes for S3 (see `tests/sigv4_vectors.rs`).

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Lower-case hexadecimal encoding.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of `data` as lower-case hex.
pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

/// HMAC-SHA256.
pub fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// AWS `UriEncode` over raw bytes: unreserved characters (`A-Za-z0-9-_.~`) are
/// kept, `/` is kept only when `keep_slash`, everything else becomes `%XX`
/// with upper-case hex digits.
pub fn uri_encode(bytes: &[u8], keep_slash: bool) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b'/' if keep_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Strict percent-decoding.  Fails on a `%` that is not followed by two hex
/// digits and on raw bytes that must never appear in a request-target
/// (non-ASCII bytes, control characters, spaces).
pub fn uri_decode(s: &str) -> Result<Vec<u8>, String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' {
            let hi = bytes.get(i + 1).copied().and_then(hex_digit);
            let lo = bytes.get(i + 2).copied().and_then(hex_digit);
            match (hi, lo) {
                (Some(hi), Some(lo)) => out.push(hi * 16 + lo),
                _ => return Err(format!("invalid percent-escape at byte {i} of {s:?}")),
            }
            i += 3;
        } else if b >= 0x80 || b <= 0x20 || b == 0x7f {
            return Err(format!("raw byte 0x{b:02x} in request-target {s:?}"));
        } else {
            out.push(b);
            i += 1;
        }
    }
    Ok(out)
}

/// S3 canonical URI: the (decoded) path URI-encoded exactly once, `/` kept.
pub fn canonical_uri(raw_path: &str) -> Result<String, String> {
    Ok(uri_encode(&uri_decode(raw_path)?, true))
}

/// Splits a raw query string into decoded `(name, value)` pairs.
/// A decoded query parameter.
pub type QueryPair = (Vec<u8>, Vec<u8>);

pub fn query_pairs(raw_query: &str) -> Result<Vec<QueryPair>, String> {
    let mut pairs = Vec::new();
    for part in raw_query.split('&').filter(|p| !p.is_empty()) {
        let (name, value) = part.split_once('=').unwrap_or((part, ""));
        pairs.push((uri_decode(name)?, uri_decode(value)?));
    }
    Ok(pairs)
}

/// Canonical query string: names and values URI-encoded (slash included),
/// sorted by encoded name then encoded value, always in `name=value` form.
pub fn canonical_query(raw_query: &str) -> Result<String, String> {
    let mut pairs: Vec<(String, String)> = query_pairs(raw_query)?
        .into_iter()
        .map(|(n, v)| (uri_encode(&n, false), uri_encode(&v, false)))
        .collect();
    pairs.sort();
    Ok(pairs
        .iter()
        .map(|(n, v)| format!("{n}={v}"))
        .collect::<Vec<_>>()
        .join("&"))
}

/// A parsed `Authorization: AWS4-HMAC-SHA256 ...` header.
#[derive(Debug, Clone)]
pub struct Authorization {
    pub access_key: String,
    pub date: String,
    pub region: String,
    pub service: String,
    pub signed_headers: Vec<String>,
    pub signature: String,
}

/// Parses an SigV4 `Authorization` header value.
pub fn parse_authorization(value: &str) -> Result<Authorization, String> {
    let rest = value
        .strip_prefix("AWS4-HMAC-SHA256 ")
        .ok_or_else(|| format!("unsupported authorization scheme in {value:?}"))?;
    let mut credential = None;
    let mut signed = None;
    let mut signature = None;
    for part in rest.split(',') {
        let (k, v) = part
            .trim()
            .split_once('=')
            .ok_or_else(|| format!("malformed authorization component {part:?}"))?;
        match k {
            "Credential" => credential = Some(v.to_string()),
            "SignedHeaders" => signed = Some(v.to_string()),
            "Signature" => signature = Some(v.to_string()),
            other => return Err(format!("unknown authorization component {other:?}")),
        }
    }
    let credential = credential.ok_or("missing Credential")?;
    let scope: Vec<&str> = credential.split('/').collect();
    if scope.len() != 5 || scope[4] != "aws4_request" {
        return Err(format!("malformed credential scope {credential:?}"));
    }
    Ok(Authorization {
        access_key: scope[0].to_string(),
        date: scope[1].to_string(),
        region: scope[2].to_string(),
        service: scope[3].to_string(),
        signed_headers: signed
            .ok_or("missing SignedHeaders")?
            .split(';')
            .map(str::to_string)
            .collect(),
        signature: signature.ok_or("missing Signature")?,
    })
}

/// A request as the server received it.
pub struct Received<'a> {
    pub method: &'a str,
    /// Raw request-target path, exactly as on the wire.
    pub raw_path: &'a str,
    /// Raw request-target query (without `?`), exactly as on the wire.
    pub raw_query: &'a str,
    /// Received headers (lower-case names); repeated names allowed.
    pub headers: &'a [(String, String)],
    /// SHA-256 of the body that actually arrived.
    pub body_sha256: &'a str,
}

impl Received<'_> {
    /// Value of header `name` (repeated headers are joined with `,`).
    pub fn header(&self, name: &str) -> Option<String> {
        let values: Vec<&str> = self
            .headers
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect();
        if values.is_empty() {
            None
        } else {
            Some(values.join(","))
        }
    }
}

/// Collapses runs of whitespace and trims, as the canonical header rule says.
fn normalize_header_value(v: &str) -> String {
    v.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Builds the canonical request for the given signed headers.
pub fn canonical_request(
    method: &str,
    canonical_uri: &str,
    canonical_query: &str,
    header_lookup: &dyn Fn(&str) -> Option<String>,
    signed_headers: &[String],
    payload_hash: &str,
) -> String {
    let mut canonical_headers = String::new();
    for name in signed_headers {
        let value = header_lookup(name).unwrap_or_default();
        canonical_headers.push_str(&format!("{name}:{}\n", normalize_header_value(&value)));
    }
    format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{}\n{payload_hash}",
        signed_headers.join(";")
    )
}

/// Computes the hex signature for `canonical_request`.
pub fn signature(
    secret: &str,
    amz_date: &str,
    region: &str,
    service: &str,
    canonical_request: &str,
) -> String {
    let date = &amz_date[..8];
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let k_date = hmac(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac(&k_date, region.as_bytes());
    let k_service = hmac(&k_region, service.as_bytes());
    let k_signing = hmac(&k_service, b"aws4_request");
    hex(&hmac(&k_signing, string_to_sign.as_bytes()))
}

/// Seconds since the Unix epoch for a `YYYYMMDDTHHMMSSZ` timestamp.
pub fn parse_amz_date(s: &str) -> Option<i64> {
    if s.len() != 16 || !s.is_ascii() || &s[8..9] != "T" || !s.ends_with('Z') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s[r].parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(4..6)?, num(6..8)?);
    let (hh, mm, ss) = (num(9..11)?, num(11..13)?, num(13..15)?);
    // Days from civil (Howard Hinnant), written independently of the crate.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

/// Why a request was refused.
#[derive(Debug, Clone)]
pub struct Rejection {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
    /// Extra response headers (for example `x-amz-bucket-region`).
    pub headers: Vec<(String, String)>,
    /// Canonical request the server computed (for diagnostics).
    pub canonical_request: Option<String>,
    /// True when the problem is about the signature itself (as opposed to the
    /// request-target encoding); lets shadow mode ignore only these.
    pub signature_class: bool,
}

impl Rejection {
    fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            headers: Vec::new(),
            canonical_request: None,
            signature_class: true,
        }
    }
}

/// Server-side verification settings.
pub struct VerifyConfig<'a> {
    pub access_key: &'a str,
    pub secret: &'a str,
    pub region: &'a str,
    /// The server's notion of "now" (Unix seconds).
    pub now_unix: i64,
    /// When set, `x-amz-security-token` must be present with this value.
    pub session_token: Option<&'a str>,
}

/// Checks that the request-target is in canonical form exactly as SigV4 would
/// encode it, so a client cannot rely on server-side leniency.
pub fn check_encoding(rx: &Received<'_>) -> Result<(), Rejection> {
    let bad = |msg: String| {
        let mut r = Rejection::new(400, "InvalidURI", msg);
        r.signature_class = false;
        r
    };
    let canon_path = canonical_uri(rx.raw_path).map_err(&bad)?;
    if canon_path != rx.raw_path {
        return Err(bad(format!(
            "request path is not canonically percent-encoded: sent {:?}, canonical form {:?}",
            rx.raw_path, canon_path
        )));
    }
    for part in rx.raw_query.split('&').filter(|p| !p.is_empty()) {
        let (name, value) = part.split_once('=').unwrap_or((part, ""));
        for piece in [name, value] {
            let canon = uri_encode(&uri_decode(piece).map_err(&bad)?, false);
            if canon != piece {
                return Err(bad(format!(
                    "query component {piece:?} is not canonically percent-encoded (expected {canon:?})"
                )));
            }
        }
    }
    Ok(())
}

/// Verifies the SigV4 signature of a received request the way S3 does.
///
/// On success returns the parsed `Authorization` header.
pub fn verify(rx: &Received<'_>, cfg: &VerifyConfig<'_>) -> Result<Authorization, Rejection> {
    let auth_value = rx.header("authorization").ok_or_else(|| {
        Rejection::new(
            403,
            "AccessDenied",
            "Access Denied (no Authorization header)",
        )
    })?;
    let auth = parse_authorization(&auth_value)
        .map_err(|e| Rejection::new(400, "AuthorizationHeaderMalformed", e))?;

    if auth.access_key != cfg.access_key {
        return Err(Rejection::new(
            403,
            "InvalidAccessKeyId",
            "The AWS Access Key Id you provided does not exist in our records.",
        ));
    }
    if auth.service != "s3" {
        return Err(Rejection::new(
            400,
            "AuthorizationHeaderMalformed",
            format!("service {:?} is not s3", auth.service),
        ));
    }
    if auth.region != cfg.region {
        let mut r = Rejection::new(
            400,
            "AuthorizationHeaderMalformed",
            format!(
                "the region '{}' is wrong; expecting '{}'",
                auth.region, cfg.region
            ),
        );
        r.headers
            .push(("x-amz-bucket-region".to_string(), cfg.region.to_string()));
        return Err(r);
    }

    let amz_date = rx
        .header("x-amz-date")
        .ok_or_else(|| Rejection::new(400, "AccessDenied", "Missing x-amz-date"))?;
    let sent_at = parse_amz_date(&amz_date).ok_or_else(|| {
        Rejection::new(
            400,
            "AuthorizationHeaderMalformed",
            format!("bad x-amz-date {amz_date:?}"),
        )
    })?;
    if amz_date[..8] != auth.date {
        return Err(Rejection::new(
            400,
            "AuthorizationHeaderMalformed",
            "credential date does not match x-amz-date",
        ));
    }
    if (cfg.now_unix - sent_at).abs() > 900 {
        return Err(Rejection::new(
            403,
            "RequestTimeTooSkewed",
            "The difference between the request time and the current time is too large.",
        ));
    }

    if !auth.signed_headers.iter().any(|h| h == "host") {
        return Err(Rejection::new(
            400,
            "AuthorizationHeaderMalformed",
            "host must be a signed header",
        ));
    }
    // A header named in SignedHeaders that the request does not carry can
    // never produce a matching signature on a real server.
    let absent: Vec<&String> = auth
        .signed_headers
        .iter()
        .filter(|h| rx.header(h).is_none())
        .collect();
    if !absent.is_empty() {
        return Err(Rejection::new(
            403,
            "SignatureDoesNotMatch",
            format!("SignedHeaders lists header(s) that were not sent: {absent:?}"),
        ));
    }
    // S3 insists that every x-amz-* header present in the request is signed.
    let unsigned: Vec<String> = rx
        .headers
        .iter()
        .map(|(n, _)| n.clone())
        .filter(|n| n.starts_with("x-amz-") && !auth.signed_headers.contains(n))
        .collect();
    if !unsigned.is_empty() {
        return Err(Rejection::new(
            403,
            "AccessDenied",
            format!(
                "There were headers present in the request which were not signed: {unsigned:?}"
            ),
        ));
    }
    if let Some(token) = cfg.session_token {
        if rx.header("x-amz-security-token").as_deref() != Some(token) {
            return Err(Rejection::new(
                403,
                "InvalidToken",
                "The provided token is malformed or otherwise invalid.",
            ));
        }
    } else if rx.header("x-amz-security-token").is_some() {
        return Err(Rejection::new(
            403,
            "InvalidToken",
            "a session token was sent but none is expected",
        ));
    }

    let payload_hash = rx.header("x-amz-content-sha256").ok_or_else(|| {
        Rejection::new(
            400,
            "InvalidRequest",
            "Missing required header for this request: x-amz-content-sha256",
        )
    })?;
    if payload_hash != rx.body_sha256 {
        return Err(Rejection::new(
            400,
            "XAmzContentSHA256Mismatch",
            "The provided 'x-amz-content-sha256' header does not match what was computed.",
        ));
    }

    let canon_uri = canonical_uri(rx.raw_path).map_err(|e| Rejection::new(400, "InvalidURI", e))?;
    let canon_query =
        canonical_query(rx.raw_query).map_err(|e| Rejection::new(400, "InvalidURI", e))?;
    let lookup = |name: &str| rx.header(name);
    let creq = canonical_request(
        rx.method,
        &canon_uri,
        &canon_query,
        &lookup,
        &auth.signed_headers,
        &payload_hash,
    );
    let expected = signature(cfg.secret, &amz_date, &auth.region, &auth.service, &creq);
    if expected != auth.signature {
        let mut r = Rejection::new(
            403,
            "SignatureDoesNotMatch",
            "The request signature we calculated does not match the signature you provided.",
        );
        r.canonical_request = Some(creq);
        return Err(r);
    }
    Ok(auth)
}
