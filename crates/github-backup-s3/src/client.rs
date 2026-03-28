// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Async S3-compatible object store client.
//!
//! Built on hyper + rustls (no OpenSSL, no reqwest, no AWS SDK).
//! Implements the minimal S3 API surface needed for backup: `PutObject` and
//! `HeadObject`.  Any S3-compatible service is supported, including:
//!
//! - AWS S3
//! - Backblaze B2 (S3-compatible API)
//! - MinIO (self-hosted)
//! - Cloudflare R2
//! - DigitalOcean Spaces
//! - Wasabi

use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use tracing::debug;

use crate::config::S3Config;
use crate::error::S3Error;
use crate::signing::Signer;

const USER_AGENT: &str = concat!("github-backup-rust/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT_SECS: u64 = 120;

type HyperClient = Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    Full<Bytes>,
>;

/// Async S3-compatible object store client.
///
/// Construct via [`S3Client::new`]. The underlying connection pool is cheaply
/// cloneable via its internal `Arc`.
#[derive(Clone)]
pub struct S3Client {
    http: HyperClient,
    config: S3Config,
    signer: Signer,
}

impl std::fmt::Debug for S3Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Client")
            .field("bucket", &self.config.bucket)
            .field("region", &self.config.region)
            .field("endpoint", &self.config.endpoint)
            .field("credentials", &"[redacted]")
            .finish()
    }
}

impl S3Client {
    /// Creates a new [`S3Client`] using the system CA bundle for TLS.
    ///
    /// # Errors
    ///
    /// Returns [`S3Error::Tls`] if the native CA bundle cannot be loaded.
    pub fn new(config: S3Config) -> Result<Self, S3Error> {
        let http = build_http_client()?;
        let signer = Signer::new_s3(
            config.access_key_id.clone(),
            config.secret_access_key.clone(),
            config.region.clone(),
        );
        Ok(Self {
            http,
            config,
            signer,
        })
    }

    /// Uploads `data` to the object at `key` in the configured bucket.
    ///
    /// The `content_type` should be set appropriately (e.g.
    /// `application/json` for JSON files, `application/octet-stream` for
    /// binary assets).
    ///
    /// # Errors
    ///
    /// Returns [`S3Error`] on network, auth, or service errors.
    pub async fn put_object(
        &self,
        key: &str,
        data: &[u8],
        content_type: &str,
    ) -> Result<(), S3Error> {
        let url = self.object_url(key);
        let host = self.host();
        let path = self.object_path(key);

        debug!(bucket = %self.config.bucket, key, "S3 PutObject");

        let signed = self.signer.sign_put(&host, &path, content_type, data);

        let req = Request::builder()
            .method(Method::PUT)
            .uri(&url)
            .header("Host", &host)
            .header("User-Agent", USER_AGENT)
            .header("Content-Type", content_type)
            .header("x-amz-date", &signed.amz_date)
            .header("x-amz-content-sha256", &signed.content_sha256)
            .header("Authorization", &signed.authorization)
            .header("Content-Length", data.len().to_string())
            .body(Full::new(Bytes::copy_from_slice(data)))
            .map_err(S3Error::Request)?;

        let response = tokio::time::timeout(
            Duration::from_secs(REQUEST_TIMEOUT_SECS),
            self.http.request(req),
        )
        .await
        .map_err(|_| S3Error::Timeout { url: url.clone() })??;

        let status = response.status();
        if status.is_success() || status == StatusCode::OK {
            return Ok(());
        }

        let body = collect_body(response.into_body()).await?;
        Err(S3Error::Api {
            status: status.as_u16(),
            body: String::from_utf8_lossy(&body).into_owned(),
        })
    }

    /// Checks whether an object with `key` exists in the configured bucket.
    ///
    /// Uses `HeadObject` which does not transfer the object body.
    ///
    /// # Errors
    ///
    /// Returns [`S3Error`] on network or auth errors (not on 404).
    pub async fn object_exists(&self, key: &str) -> Result<bool, S3Error> {
        let url = self.object_url(key);
        let host = self.host();
        let path = self.object_path(key);

        let signed = self.signer.sign_get(&host, &path);

        let req = Request::builder()
            .method(Method::HEAD)
            .uri(&url)
            .header("Host", &host)
            .header("User-Agent", USER_AGENT)
            .header("x-amz-date", &signed.amz_date)
            .header("x-amz-content-sha256", &signed.content_sha256)
            .header("Authorization", &signed.authorization)
            .body(Full::new(Bytes::new()))
            .map_err(S3Error::Request)?;

        let response = tokio::time::timeout(
            Duration::from_secs(REQUEST_TIMEOUT_SECS),
            self.http.request(req),
        )
        .await
        .map_err(|_| S3Error::Timeout { url: url.clone() })??;

        match response.status() {
            StatusCode::OK => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            status => {
                let body = collect_body(response.into_body()).await?;
                Err(S3Error::Api {
                    status: status.as_u16(),
                    body: String::from_utf8_lossy(&body).into_owned(),
                })
            }
        }
    }

    /// Builds the full URL for an object key.
    fn object_url(&self, key: &str) -> String {
        let key = key.trim_start_matches('/');
        match &self.config.endpoint {
            Some(endpoint) => {
                // Path-style URL for custom endpoints (MinIO, B2, etc.).
                let endpoint = endpoint.trim_end_matches('/');
                format!("{endpoint}/{}/{key}", self.config.bucket)
            }
            None => {
                // Virtual-hosted-style URL for AWS S3.
                format!(
                    "https://{}.s3.{}.amazonaws.com/{key}",
                    self.config.bucket, self.config.region
                )
            }
        }
    }

    /// Returns the S3 host header value.
    fn host(&self) -> String {
        match &self.config.endpoint {
            Some(endpoint) => {
                // Extract the host from the custom endpoint URL.
                endpoint
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .trim_end_matches('/')
                    .to_string()
            }
            None => format!(
                "{}.s3.{}.amazonaws.com",
                self.config.bucket, self.config.region
            ),
        }
    }

    /// Returns the URL path for an object key.
    fn object_path(&self, key: &str) -> String {
        let key = key.trim_start_matches('/');
        match &self.config.endpoint {
            // Path-style: /bucket/key
            Some(_) => format!("/{}/{key}", self.config.bucket),
            // Virtual-hosted: /key
            None => format!("/{key}"),
        }
    }
}

/// Collects a hyper body into a [`Bytes`] buffer.
async fn collect_body(
    body: impl hyper::body::Body<Data = Bytes, Error = hyper::Error>,
) -> Result<Bytes, S3Error> {
    Ok(body.collect().await?.to_bytes())
}

/// Builds an HTTPS client using the system native CA bundle.
fn build_http_client() -> Result<HyperClient, S3Error> {
    let mut root_store = rustls::RootCertStore::empty();
    let cert_result = rustls_native_certs::load_native_certs();
    if cert_result.certs.is_empty() {
        let msg = cert_result
            .errors
            .first()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "no CA certificates found".to_string());
        return Err(S3Error::Tls(msg));
    }
    root_store.add_parsable_certificates(cert_result.certs);
    let tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_or_http()
        .enable_http1()
        .build();

    Ok(Client::builder(TokioExecutor::new()).build(https))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::S3Config;

    fn sample_config() -> S3Config {
        S3Config {
            bucket: "my-bucket".to_string(),
            region: "us-east-1".to_string(),
            prefix: "backups/".to_string(),
            endpoint: None,
            access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
        }
    }

    fn sample_config_custom_endpoint() -> S3Config {
        S3Config {
            endpoint: Some("https://s3.us-west-004.backblazeb2.com".to_string()),
            ..sample_config()
        }
    }

    #[test]
    fn object_url_uses_virtual_hosted_style_for_aws() {
        let client = S3Client::new(sample_config()).unwrap();
        let url = client.object_url("owner/repo/info.json");
        assert_eq!(
            url,
            "https://my-bucket.s3.us-east-1.amazonaws.com/owner/repo/info.json"
        );
    }

    #[test]
    fn object_url_uses_path_style_for_custom_endpoint() {
        let client = S3Client::new(sample_config_custom_endpoint()).unwrap();
        let url = client.object_url("owner/repo/info.json");
        assert_eq!(
            url,
            "https://s3.us-west-004.backblazeb2.com/my-bucket/owner/repo/info.json"
        );
    }

    #[test]
    fn host_returns_virtual_hosted_for_aws() {
        let client = S3Client::new(sample_config()).unwrap();
        assert_eq!(client.host(), "my-bucket.s3.us-east-1.amazonaws.com");
    }

    #[test]
    fn host_returns_custom_endpoint_host() {
        let client = S3Client::new(sample_config_custom_endpoint()).unwrap();
        assert_eq!(client.host(), "s3.us-west-004.backblazeb2.com");
    }

    #[test]
    fn object_path_virtual_hosted() {
        let client = S3Client::new(sample_config()).unwrap();
        assert_eq!(client.object_path("foo/bar.json"), "/foo/bar.json");
    }

    #[test]
    fn object_path_custom_endpoint() {
        let client = S3Client::new(sample_config_custom_endpoint()).unwrap();
        assert_eq!(
            client.object_path("foo/bar.json"),
            "/my-bucket/foo/bar.json"
        );
    }

    #[test]
    fn s3_client_debug_redacts_credentials() {
        let client = S3Client::new(sample_config()).unwrap();
        let debug = format!("{client:?}");
        assert!(
            !debug.contains("AKIAIOSFODNN7EXAMPLE"),
            "access key must be redacted"
        );
        assert!(debug.contains("[redacted]"));
    }
}
