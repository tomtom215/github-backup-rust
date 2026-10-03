// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! S3-compatible storage backend configuration.

use std::fmt;

use hyper::Uri;
use serde::{Deserialize, Serialize};

use crate::error::S3Error;

/// Configuration for an S3-compatible object store.
///
/// Works with AWS S3, Backblaze B2 (S3-compatible API), MinIO, Cloudflare R2,
/// DigitalOcean Spaces, Wasabi, and any other S3-compatible service.
///
/// The secret access key and the session token are never printed by `Debug`
/// and never serialised.
///
/// # Examples
///
/// AWS S3:
/// ```no_run
/// use github_backup_s3::config::S3Config;
///
/// let cfg = S3Config {
///     bucket: "my-github-backups".to_string(),
///     region: "us-east-1".to_string(),
///     prefix: "github/".to_string(),
///     endpoint: None,
///     access_key_id: std::env::var("AWS_ACCESS_KEY_ID").unwrap(),
///     secret_access_key: std::env::var("AWS_SECRET_ACCESS_KEY").unwrap(),
///     session_token: std::env::var("AWS_SESSION_TOKEN").ok(),
/// };
/// ```
///
/// Backblaze B2:
/// ```no_run
/// use github_backup_s3::config::S3Config;
///
/// let cfg = S3Config {
///     bucket: "my-b2-bucket".to_string(),
///     region: "us-west-004".to_string(),
///     prefix: "github/".to_string(),
///     endpoint: Some("https://s3.us-west-004.backblazeb2.com".to_string()),
///     access_key_id: std::env::var("B2_KEY_ID").unwrap(),
///     secret_access_key: std::env::var("B2_APP_KEY").unwrap(),
///     session_token: None,
/// };
/// ```
#[derive(Clone, Serialize, Deserialize)]
pub struct S3Config {
    /// The S3 bucket name.  The bucket must already exist.
    pub bucket: String,

    /// AWS region for the bucket (e.g., `us-east-1`, `eu-west-1`).
    ///
    /// For B2, this is the region portion of the endpoint hostname (e.g.,
    /// `us-west-004` for `s3.us-west-004.backblazeb2.com`).
    pub region: String,

    /// Key prefix to apply to all objects.
    ///
    /// Allows several backups in one bucket.  Normalised to end in exactly one
    /// `/` (see [`normalize_prefix`]); may be empty for the bucket root.
    /// Example: `"github-backup/"`.
    pub prefix: String,

    /// Custom S3 endpoint URL for non-AWS services, with its scheme
    /// (`https://…` or `http://…`).
    ///
    /// For AWS S3, leave this `None` (the standard endpoint is derived from
    /// `bucket` and `region`).
    ///
    /// For B2: `"https://s3.<region>.backblazeb2.com"`.
    /// For MinIO: `"http://localhost:9000"`.
    /// For Cloudflare R2: `"https://<account_id>.r2.cloudflarestorage.com"`.
    pub endpoint: Option<String>,

    /// AWS access key ID (or equivalent for S3-compatible services).
    ///
    /// Can also be read from the `AWS_ACCESS_KEY_ID` environment variable.
    pub access_key_id: String,

    /// AWS secret access key (or equivalent for S3-compatible services).
    ///
    /// Can also be read from the `AWS_SECRET_ACCESS_KEY` environment variable.
    #[serde(skip_serializing, default)]
    pub secret_access_key: String,

    /// Session token for temporary credentials (`AWS_SESSION_TOKEN`).
    ///
    /// Sent as the signed `x-amz-security-token` header.
    #[serde(skip_serializing, default)]
    pub session_token: Option<String>,
}

impl fmt::Debug for S3Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Config")
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("prefix", &self.prefix)
            .field("endpoint", &self.endpoint)
            .field("credentials", &"[redacted]")
            .finish()
    }
}

/// Normalises a key prefix: surrounding whitespace and empty path segments
/// are dropped and a non-empty result ends in exactly one `/`.
///
/// `"/a//b"`, `"a/b"` and `"a/b///"` all become `"a/b/"`; `""` and `"/"`
/// become `""`.  Listing with a prefix that does not end in `/` would also
/// match sibling prefixes (`github-backup` matches `github-backup-old/…`),
/// which is why every listing goes through this function.
#[must_use]
pub fn normalize_prefix(prefix: &str) -> String {
    let parts: Vec<&str> = prefix
        .trim()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("{}/", parts.join("/"))
    }
}

impl S3Config {
    /// Returns the full S3 key for `relative_path` by prepending the prefix.
    ///
    /// Ensures there is no double slash at the boundary.
    #[must_use]
    pub fn full_key(&self, relative_path: &str) -> String {
        format!(
            "{}{}",
            normalize_prefix(&self.prefix),
            relative_path.trim_start_matches('/')
        )
    }

    /// The normalised key prefix (empty, or ending in exactly one `/`).
    #[must_use]
    pub fn normalized_prefix(&self) -> String {
        normalize_prefix(&self.prefix)
    }

    /// Checks everything that can be checked without talking to the server.
    ///
    /// # Errors
    ///
    /// Returns [`S3Error::InvalidConfig`] (or [`S3Error::InvalidEndpoint`])
    /// with a message that says what to change.
    pub fn validate(&self) -> Result<(), S3Error> {
        validate_bucket(&self.bucket)?;
        validate_region(&self.region)?;
        if self.access_key_id.trim().is_empty() {
            return Err(S3Error::InvalidConfig(
                "the access key id is empty".to_string(),
            ));
        }
        if self.secret_access_key.trim().is_empty() {
            return Err(S3Error::InvalidConfig(
                "the secret access key is empty".to_string(),
            ));
        }
        if self
            .session_token
            .as_deref()
            .is_some_and(|t| t.trim().is_empty())
        {
            return Err(S3Error::InvalidConfig(
                "the session token is empty (leave it unset instead)".to_string(),
            ));
        }
        if let Some(endpoint) = &self.endpoint {
            parse_endpoint(endpoint)?;
        }
        Ok(())
    }
}

fn validate_bucket(bucket: &str) -> Result<(), S3Error> {
    let bad = |why: &str| {
        Err(S3Error::InvalidConfig(format!(
            "bucket name {bucket:?} is not usable: {why}"
        )))
    };
    if bucket.is_empty() {
        return bad("it is empty");
    }
    if bucket.contains("://") || bucket.starts_with("s3:") {
        return bad("pass only the bucket name, not a URL (for example `my-backups`, not `s3://my-backups`)");
    }
    if bucket.contains('/') {
        return bad("it contains `/`; put key prefixes in --s3-prefix instead");
    }
    if !bucket
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return bad("only letters, digits, `.`, `-` and `_` are allowed");
    }
    if bucket.len() > 255 {
        return bad("it is longer than 255 characters");
    }
    Ok(())
}

fn validate_region(region: &str) -> Result<(), S3Error> {
    if region.is_empty()
        || !region
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return Err(S3Error::InvalidConfig(format!(
            "region {region:?} is not usable: expected something like `us-east-1` (`auto` for Cloudflare R2)"
        )));
    }
    Ok(())
}

/// A validated custom endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedEndpoint {
    /// `true` for `https://`.
    pub https: bool,
    /// `host[:port]` exactly as it must appear in the URL.
    pub authority: String,
    /// The `Host` header value: the authority without a default port.
    pub host_header: String,
    /// `true` when the user gave no scheme and `https://` was assumed.
    pub assumed_https: bool,
}

/// Parses and validates `--s3-endpoint`.
///
/// A scheme-less value is taken to be `https://`.  Credentials, a path and a
/// query string are rejected: they would silently produce wrong signatures.
pub(crate) fn parse_endpoint(raw: &str) -> Result<ParsedEndpoint, S3Error> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(S3Error::InvalidEndpoint(
            "the endpoint is empty".to_string(),
        ));
    }
    let (with_scheme, assumed_https) = if trimmed.contains("://") {
        (trimmed.to_string(), false)
    } else {
        (format!("https://{trimmed}"), true)
    };
    let uri: Uri = with_scheme.parse().map_err(|e| {
        S3Error::InvalidEndpoint(format!(
            "{raw:?} is not a valid URL ({e}); expected e.g. https://s3.example.com"
        ))
    })?;
    let https = match uri.scheme_str() {
        Some("https") => true,
        Some("http") => false,
        _ => {
            return Err(S3Error::InvalidEndpoint(format!(
                "{raw:?} must start with https:// or http://"
            )))
        }
    };
    let authority = uri.authority().ok_or_else(|| {
        S3Error::InvalidEndpoint(format!(
            "{raw:?} has no host; expected e.g. https://s3.example.com"
        ))
    })?;
    if authority.as_str().contains('@') {
        return Err(S3Error::InvalidEndpoint(
            "the endpoint must not contain credentials; use --s3-access-key / --s3-secret-key"
                .to_string(),
        ));
    }
    if !matches!(uri.path(), "" | "/") || uri.query().is_some() {
        return Err(S3Error::InvalidEndpoint(format!(
            "{raw:?} must be just scheme://host[:port]; paths and query strings are not supported"
        )));
    }
    let default_port = if https { 443 } else { 80 };
    let host_header = match authority.port_u16() {
        Some(port) if port == default_port => authority.host().to_string(),
        _ => authority.as_str().to_string(),
    };
    Ok(ParsedEndpoint {
        https,
        authority: authority.as_str().to_string(),
        host_header,
        assumed_https,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> S3Config {
        S3Config {
            bucket: "test-bucket".to_string(),
            region: "us-east-1".to_string(),
            prefix: "backups/".to_string(),
            endpoint: None,
            access_key_id: "key".to_string(),
            secret_access_key: "secret".to_string(),
            session_token: None,
        }
    }

    #[test]
    fn full_key_prepends_prefix() {
        let cfg = sample();
        assert_eq!(
            cfg.full_key("owner/repo/info.json"),
            "backups/owner/repo/info.json"
        );
    }

    #[test]
    fn full_key_no_double_slash() {
        let mut cfg = sample();
        cfg.prefix = "backups/".to_string();
        assert_eq!(cfg.full_key("/owner/repo.json"), "backups/owner/repo.json");
    }

    #[test]
    fn full_key_empty_prefix() {
        let mut cfg = sample();
        cfg.prefix = String::new();
        assert_eq!(cfg.full_key("owner/file.json"), "owner/file.json");
    }

    #[test]
    fn config_roundtrips_json() {
        let cfg = sample();
        let json = serde_json::to_string(&cfg).unwrap();
        let decoded: S3Config = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.bucket, cfg.bucket);
        assert_eq!(decoded.region, cfg.region);
    }

    #[test]
    fn serialisation_never_contains_secrets() {
        let mut cfg = sample();
        cfg.session_token = Some("SESSION-TOKEN-VALUE".to_string());
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(!json.contains("secret"), "{json}");
        assert!(!json.contains("SESSION-TOKEN-VALUE"), "{json}");
    }

    #[test]
    fn debug_never_contains_secrets() {
        let mut cfg = sample();
        cfg.secret_access_key = "TOP-SECRET-KEY".to_string();
        cfg.access_key_id = "AKIAEXAMPLEID".to_string();
        cfg.session_token = Some("SESSION-TOKEN-VALUE".to_string());
        let debug = format!("{cfg:?}");
        assert!(!debug.contains("TOP-SECRET-KEY"), "{debug}");
        assert!(!debug.contains("AKIAEXAMPLEID"), "{debug}");
        assert!(!debug.contains("SESSION-TOKEN-VALUE"), "{debug}");
        assert!(debug.contains("[redacted]"));
        assert!(debug.contains("test-bucket"));
    }

    #[test]
    fn normalize_prefix_ends_in_exactly_one_slash() {
        assert_eq!(normalize_prefix("github-backup"), "github-backup/");
        assert_eq!(normalize_prefix("github-backup/"), "github-backup/");
        assert_eq!(normalize_prefix("github-backup///"), "github-backup/");
        assert_eq!(normalize_prefix("/a//b/"), "a/b/");
        assert_eq!(normalize_prefix("  a/b  "), "a/b/");
        assert_eq!(normalize_prefix(""), "");
        assert_eq!(normalize_prefix("/"), "");
        assert_eq!(normalize_prefix("///"), "");
    }

    #[test]
    fn full_key_handles_leading_slash_prefix() {
        let mut cfg = sample();
        cfg.prefix = "/pfx/".to_string();
        assert_eq!(cfg.full_key("x.json"), "pfx/x.json");
    }

    #[test]
    fn validate_accepts_a_sane_config() {
        sample().validate().unwrap();
    }

    #[test]
    fn validate_rejects_bucket_urls_and_odd_names() {
        for bucket in [
            "",
            "s3://my-bucket",
            "my bucket",
            "a/b",
            "buck\u{e9}t",
            "my:bucket",
        ] {
            let mut cfg = sample();
            cfg.bucket = bucket.to_string();
            let err = cfg.validate().expect_err(bucket).to_string();
            assert!(err.contains("bucket name"), "{bucket}: {err}");
        }
    }

    #[test]
    fn validate_rejects_empty_credentials() {
        let mut cfg = sample();
        cfg.access_key_id = "  ".to_string();
        assert!(cfg
            .validate()
            .unwrap_err()
            .to_string()
            .contains("access key id"));
        let mut cfg = sample();
        cfg.secret_access_key = String::new();
        assert!(cfg
            .validate()
            .unwrap_err()
            .to_string()
            .contains("secret access key"));
        let mut cfg = sample();
        cfg.session_token = Some(String::new());
        assert!(cfg
            .validate()
            .unwrap_err()
            .to_string()
            .contains("session token"));
    }

    #[test]
    fn validate_rejects_bad_region() {
        let mut cfg = sample();
        cfg.region = "us east".to_string();
        assert!(cfg.validate().unwrap_err().to_string().contains("region"));
    }

    #[test]
    fn endpoint_requires_scheme_or_assumes_https() {
        let e = parse_endpoint("s3.wasabisys.com").unwrap();
        assert!(e.https && e.assumed_https);
        assert_eq!(e.host_header, "s3.wasabisys.com");
        let e = parse_endpoint("https://s3.wasabisys.com").unwrap();
        assert!(e.https && !e.assumed_https);
    }

    #[test]
    fn endpoint_keeps_non_default_ports_and_strips_default_ones() {
        let e = parse_endpoint("http://127.0.0.1:9000").unwrap();
        assert!(!e.https);
        assert_eq!(e.authority, "127.0.0.1:9000");
        assert_eq!(e.host_header, "127.0.0.1:9000");
        assert_eq!(
            parse_endpoint("https://example.com:443")
                .unwrap()
                .host_header,
            "example.com"
        );
        assert_eq!(
            parse_endpoint("http://example.com:80").unwrap().host_header,
            "example.com"
        );
        assert_eq!(
            parse_endpoint("https://example.com:8443")
                .unwrap()
                .host_header,
            "example.com:8443"
        );
    }

    #[test]
    fn endpoint_tolerates_trailing_slash_only() {
        assert!(parse_endpoint("https://example.com/").is_ok());
        assert!(parse_endpoint("http://host/sub").is_err());
        assert!(parse_endpoint("http://host/?x=1").is_err());
    }

    #[test]
    fn endpoint_rejects_credentials_and_wrong_schemes() {
        let err = parse_endpoint("https://user:pw@example.com")
            .unwrap_err()
            .to_string();
        assert!(err.contains("credentials"), "{err}");
        assert!(parse_endpoint("ftp://example.com").is_err());
        assert!(parse_endpoint("").is_err());
    }
}
