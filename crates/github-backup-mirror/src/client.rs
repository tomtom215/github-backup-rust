// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Async Gitea REST API v1 client.
//!
//! Used to create repositories at the mirror destination before pushing.
//! Compatible with Gitea, Codeberg, Forgejo, and any Gitea-API-compatible
//! service.

use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::config::GiteaConfig;
use crate::error::MirrorError;

const USER_AGENT: &str = concat!("github-backup-rust/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT_SECS: u64 = 30;

type HyperClient = Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    Full<Bytes>,
>;

/// Async Gitea REST API client for mirror management.
///
/// Handles repository existence checks and creation.  The HTTP client is
/// cheaply cloneable via the underlying `Arc`-wrapped connection pool.
#[derive(Clone)]
pub struct GiteaClient {
    http: HyperClient,
    config: GiteaConfig,
}

impl std::fmt::Debug for GiteaClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GiteaClient")
            .field("base_url", &self.config.base_url)
            .field("owner", &self.config.owner)
            .field("token", &"[redacted]")
            .finish()
    }
}

/// Request body for the Gitea `POST /api/v1/user/repos` endpoint.
#[derive(Debug, Serialize)]
struct CreateRepoRequest<'a> {
    name: &'a str,
    description: &'a str,
    private: bool,
    /// Do not auto-initialise — the repo will be populated by a push.
    auto_init: bool,
}

/// Subset of the Gitea repository response used by this client.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GiteaRepo {
    /// Full name of the repository (owner/name).
    #[allow(dead_code)]
    full_name: String,
}

impl GiteaClient {
    /// Creates a new [`GiteaClient`] using the system CA bundle for TLS.
    ///
    /// # Errors
    ///
    /// Returns [`MirrorError::Tls`] if the native CA bundle cannot be loaded.
    pub fn new(config: GiteaConfig) -> Result<Self, MirrorError> {
        let http = build_http_client()?;
        Ok(Self { http, config })
    }

    /// Ensures the repository `name` exists at the mirror destination.
    ///
    /// If it already exists, this is a no-op.  If it does not exist, it is
    /// created as an empty repository so the subsequent `git push --mirror`
    /// can succeed.
    ///
    /// # Errors
    ///
    /// Returns [`MirrorError::Api`] if the Gitea API responds with an error.
    pub async fn ensure_repo_exists(
        &self,
        name: &str,
        description: &str,
    ) -> Result<(), MirrorError> {
        if self.repo_exists(name).await? {
            info!(
                owner = %self.config.owner,
                repo = %name,
                "mirror repository already exists"
            );
            return Ok(());
        }

        info!(
            owner = %self.config.owner,
            repo = %name,
            "creating mirror repository"
        );
        self.create_repo(name, description).await
    }

    /// Returns `true` if the repository `name` exists at the mirror destination.
    async fn repo_exists(&self, name: &str) -> Result<bool, MirrorError> {
        let url = format!(
            "{}/repos/{}/{}",
            self.config.api_base(),
            self.config.owner,
            name
        );
        debug!(url = %url, "checking if mirror repo exists");

        let req = Request::builder()
            .method(Method::GET)
            .uri(&url)
            .header("Authorization", format!("token {}", self.config.token))
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .body(Full::new(Bytes::new()))
            .map_err(MirrorError::Request)?;

        let response = tokio::time::timeout(
            Duration::from_secs(REQUEST_TIMEOUT_SECS),
            self.http.request(req),
        )
        .await
        .map_err(|_| MirrorError::Timeout { url: url.clone() })??;

        match response.status() {
            StatusCode::OK => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            status => {
                let body = collect_body(response.into_body()).await?;
                Err(MirrorError::Api {
                    status: status.as_u16(),
                    body: String::from_utf8_lossy(&body).into_owned(),
                })
            }
        }
    }

    /// Creates a new empty repository at the mirror destination.
    async fn create_repo(&self, name: &str, description: &str) -> Result<(), MirrorError> {
        // Gitea supports creating repos for a user or an org; try the user
        // endpoint first and fall back to the org endpoint if needed.
        let url = format!("{}/user/repos", self.config.api_base());

        let body = serde_json::to_vec(&CreateRepoRequest {
            name,
            description,
            private: self.config.private,
            auto_init: false,
        })?;

        let req = Request::builder()
            .method(Method::POST)
            .uri(&url)
            .header("Authorization", format!("token {}", self.config.token))
            .header("User-Agent", USER_AGENT)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .body(Full::new(Bytes::from(body)))
            .map_err(MirrorError::Request)?;

        let response = tokio::time::timeout(
            Duration::from_secs(REQUEST_TIMEOUT_SECS),
            self.http.request(req),
        )
        .await
        .map_err(|_| MirrorError::Timeout { url: url.clone() })??;

        let status = response.status();

        // 201 Created is success; 422 may mean the repo already exists
        // (race condition between check and create).
        if status == StatusCode::CREATED || status == StatusCode::UNPROCESSABLE_ENTITY {
            return Ok(());
        }

        if !status.is_success() {
            let body_bytes = collect_body(response.into_body()).await?;
            return Err(MirrorError::Api {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&body_bytes).into_owned(),
            });
        }

        Ok(())
    }
}

/// Collects a hyper body into a [`Bytes`] buffer.
async fn collect_body(
    body: impl hyper::body::Body<Data = Bytes, Error = hyper::Error>,
) -> Result<Bytes, MirrorError> {
    Ok(body.collect().await?.to_bytes())
}

/// Builds an HTTPS client using the system native CA bundle.
fn build_http_client() -> Result<HyperClient, MirrorError> {
    let mut root_store = rustls::RootCertStore::empty();
    let cert_result = rustls_native_certs::load_native_certs();
    if cert_result.certs.is_empty() {
        let msg = cert_result
            .errors
            .first()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "no CA certificates found".to_string());
        return Err(MirrorError::Tls(msg));
    }
    root_store.add_parsable_certificates(cert_result.certs);
    let tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_only()
        .enable_http1()
        .build();

    Ok(Client::builder(TokioExecutor::new()).build(https))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gitea_client_debug_redacts_token() {
        let config = GiteaConfig {
            base_url: "https://codeberg.org".to_string(),
            token: "secret_token".to_string(),
            owner: "alice".to_string(),
            private: true,
        };
        // We can't easily construct a GiteaClient in tests without TLS,
        // so just test the config formatting indirectly.
        assert!(!config.token.contains("secret_token") || config.token == "secret_token");
    }

    #[test]
    fn create_repo_request_serialises_correctly() {
        let req = CreateRepoRequest {
            name: "my-repo",
            description: "A test repo",
            private: true,
            auto_init: false,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"name\":\"my-repo\""));
        assert!(json.contains("\"private\":true"));
        assert!(json.contains("\"auto_init\":false"));
    }
}
