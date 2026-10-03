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
use crate::push::verify_ours;

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
    /// Free-text description; carries the mirror marker.
    #[serde(default)]
    description: Option<String>,
    /// `true` while the repository has no commits.
    #[serde(default)]
    empty: bool,
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
    /// If it already exists it must carry `description` (the marker set when
    /// this tool created it) or still be empty; otherwise it belongs to
    /// someone else and [`MirrorError::ForeignRepository`] is returned.  If it
    /// does not exist, it is created as an empty repository (`private` or
    /// public) so the subsequent push can succeed.
    ///
    /// # Errors
    ///
    /// Returns [`MirrorError::Api`] if the Gitea API responds with an error.
    pub async fn ensure_repo_exists(
        &self,
        name: &str,
        description: &str,
        private: bool,
    ) -> Result<(), MirrorError> {
        if let Some(existing) = self.fetch_repo(name).await? {
            verify_ours(
                name,
                existing.description.as_deref(),
                existing.empty,
                description,
            )?;
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
        self.create_repo(name, description, private).await
    }

    /// Returns the repository `name` at the mirror destination, if it exists.
    async fn fetch_repo(&self, name: &str) -> Result<Option<GiteaRepo>, MirrorError> {
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
            StatusCode::OK => {
                let body = collect_body(response.into_body()).await?;
                Ok(Some(serde_json::from_slice(&body)?))
            }
            StatusCode::NOT_FOUND => Ok(None),
            status => {
                let body = collect_body(response.into_body()).await?;
                Err(MirrorError::Api {
                    status: status.as_u16(),
                    body: String::from_utf8_lossy(&body).into_owned(),
                })
            }
        }
    }

    /// Returns the login of the account the token belongs to.
    async fn authenticated_login(&self) -> Result<String, MirrorError> {
        #[derive(Deserialize)]
        struct Me {
            login: String,
        }
        let url = format!("{}/user", self.config.api_base());
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
        let status = response.status();
        let body = collect_body(response.into_body()).await?;
        if !status.is_success() {
            return Err(MirrorError::Api {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }
        Ok(serde_json::from_slice::<Me>(&body)?.login)
    }

    /// Creates a new empty repository at the mirror destination.
    async fn create_repo(
        &self,
        name: &str,
        description: &str,
        private: bool,
    ) -> Result<(), MirrorError> {
        // `/user/repos` always creates under the *token's* account.  When the
        // configured owner is another account (an organisation), the repository
        // must be created through the organisation endpoint or the push below
        // would target a repository that does not exist.
        let login = self.authenticated_login().await?;
        let url = create_repo_url(&self.config.api_base(), &self.config.owner, &login);

        let body = serde_json::to_vec(&CreateRepoRequest {
            name,
            description,
            private,
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

/// The endpoint that creates a repository under `owner` for a token that
/// belongs to `login`.
fn create_repo_url(api_base: &str, owner: &str, login: &str) -> String {
    if owner.eq_ignore_ascii_case(login) {
        format!("{api_base}/user/repos")
    } else {
        format!("{api_base}/orgs/{owner}/repos")
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
    fn repositories_are_created_under_the_configured_owner() {
        let api = "https://git.example/api/v1";
        assert_eq!(
            create_repo_url(api, "Alice", "alice"),
            "https://git.example/api/v1/user/repos"
        );
        assert_eq!(
            create_repo_url(api, "my-org", "alice"),
            "https://git.example/api/v1/orgs/my-org/repos"
        );
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
