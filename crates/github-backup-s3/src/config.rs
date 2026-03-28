// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! S3-compatible storage backend configuration.

use serde::{Deserialize, Serialize};

/// Configuration for an S3-compatible object store.
///
/// Works with AWS S3, Backblaze B2 (S3-compatible API), MinIO, Cloudflare R2,
/// DigitalOcean Spaces, Wasabi, and any other S3-compatible service.
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
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Config {
    /// The S3 bucket name.
    pub bucket: String,

    /// AWS region for the bucket (e.g., `us-east-1`, `eu-west-1`).
    ///
    /// For B2, this is the region portion of the endpoint hostname (e.g.,
    /// `us-west-004` for `s3.us-west-004.backblazeb2.com`).
    pub region: String,

    /// Key prefix to apply to all objects.
    ///
    /// Allows multiple backups or owners in one bucket.  Should end with `/`.
    /// Example: `"github-backup/octocat/"`.
    pub prefix: String,

    /// Custom S3 endpoint URL for non-AWS services.
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
    pub secret_access_key: String,
}

impl S3Config {
    /// Returns the full S3 key for `relative_path` by prepending the prefix.
    ///
    /// Ensures there is no double slash at the boundary.
    #[must_use]
    pub fn full_key(&self, relative_path: &str) -> String {
        let prefix = self.prefix.trim_end_matches('/');
        let path = relative_path.trim_start_matches('/');
        if prefix.is_empty() {
            path.to_string()
        } else {
            format!("{prefix}/{path}")
        }
    }
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
}
