// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Async GitLab REST API v4 client for mirror repository management.
//!
//! Used to create projects on GitLab.com or a self-hosted GitLab CE/EE
//! instance before pushing the bare clone with `git push --mirror`.

use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::config::GitLabConfig;
use crate::error::MirrorError;

const USER_AGENT: &str = concat!("github-backup-rust/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT_SECS: u64 = 30;

type HyperClient = Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    Full<Bytes>,
>;

/// Async GitLab REST API v4 client.
///
/// Handles project existence checks and creation at a GitLab instance.
/// Compatible with GitLab.com and any self-hosted GitLab CE/EE deployment.
#[derive(Clone)]
pub struct GitLabClient {
    http: HyperClient,
    config: GitLabConfig,
}

impl std::fmt::Debug for GitLabClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitLabClient")
            .field("base_url", &self.config.base_url)
            .field("namespace", &self.config.namespace)
            .field("token", &"[redacted]")
            .finish()
    }
}

/// Request body for `POST /api/v4/projects`.
#[derive(Debug, Serialize)]
struct CreateProjectRequest<'a> {
    name: &'a str,
    description: &'a str,
    /// `"private"` or `"public"`.
    visibility: &'a str,
    /// `true` = initialise with a README (we want `false` for push-mirror).
    initialize_with_readme: bool,
    /// Namespace path (username or group).
    namespace_path: &'a str,
}

/// Minimal subset of the GitLab project response.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GitLabProject {
    id: u64,
    path_with_namespace: String,
}

impl GitLabClient {
    /// Creates a new [`GitLabClient`] using the system CA bundle for TLS.
    ///
    /// # Errors
    ///
    /// Returns [`MirrorError::Tls`] if the native CA bundle cannot be loaded.
    pub fn new(config: GitLabConfig) -> Result<Self, MirrorError> {
        let http = build_http_client()?;
        Ok(Self { http, config })
    }

    /// Returns the HTTPS clone URL for `repo_name` at this GitLab instance.
    #[must_use]
    pub fn repo_clone_url(&self, repo_name: &str) -> String {
        self.config.repo_clone_url(repo_name)
    }

    /// Ensures the project `name` exists at the GitLab destination.
    ///
    /// If it already exists, this is a no-op.  If not, it is created as an
    /// empty (uninitialised) project so the subsequent `git push --mirror`
    /// can succeed.
    ///
    /// # Errors
    ///
    /// Returns [`MirrorError::Api`] if the GitLab API responds with an error.
    pub async fn ensure_repo_exists(
        &self,
        name: &str,
        description: &str,
    ) -> Result<(), MirrorError> {
        if self.project_exists(name).await? {
            info!(
                namespace = %self.config.namespace,
                project = %name,
                "GitLab mirror project already exists"
            );
            return Ok(());
        }

        info!(
            namespace = %self.config.namespace,
            project = %name,
            "creating GitLab mirror project"
        );
        self.create_project(name, description).await
    }

    /// Returns `true` if project `name` exists under the configured namespace.
    async fn project_exists(&self, name: &str) -> Result<bool, MirrorError> {
        // GitLab identifies projects by `namespace/name` (URL-encoded).
        let path = format!("{}/{}", self.config.namespace, name);
        let encoded = percent_encode(&path);
        let url = format!("{}/projects/{encoded}", self.config.api_base());
        debug!(url = %url, "checking if GitLab project exists");

        let req = self
            .build_request(Method::GET, &url)?
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

    /// Creates a new uninitialised project at the GitLab destination.
    async fn create_project(&self, name: &str, description: &str) -> Result<(), MirrorError> {
        let url = format!("{}/projects", self.config.api_base());
        let visibility = if self.config.private {
            "private"
        } else {
            "public"
        };

        let body = serde_json::to_vec(&CreateProjectRequest {
            name,
            description,
            visibility,
            initialize_with_readme: false,
            namespace_path: &self.config.namespace,
        })?;

        let req = self
            .build_request(Method::POST, &url)?
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(body)))
            .map_err(MirrorError::Request)?;

        let response = tokio::time::timeout(
            Duration::from_secs(REQUEST_TIMEOUT_SECS),
            self.http.request(req),
        )
        .await
        .map_err(|_| MirrorError::Timeout { url: url.clone() })??;

        let status = response.status();

        // 201 Created is success; 400 may mean the project already exists
        // (race condition between check and create).
        if status == StatusCode::CREATED || status == StatusCode::BAD_REQUEST {
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

    /// Builds an HTTP request pre-populated with auth and user-agent headers.
    ///
    /// GitLab accepts personal access tokens via the `PRIVATE-TOKEN` header.
    fn build_request(
        &self,
        method: Method,
        url: &str,
    ) -> Result<hyper::http::request::Builder, MirrorError> {
        Ok(Request::builder()
            .method(method)
            .uri(url)
            .header("PRIVATE-TOKEN", &self.config.token)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json"))
    }
}

/// Percent-encodes a string for use in a URL path segment.
///
/// Encodes `/` as `%2F` so that `namespace/project` can be embedded as a
/// single path component in the GitLab API URL.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => {
                out.push('%');
                out.push_str(&format!("{other:02X}"));
            }
        }
    }
    out
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
    fn percent_encode_encodes_slash() {
        assert_eq!(percent_encode("foo/bar"), "foo%2Fbar");
    }

    #[test]
    fn percent_encode_leaves_safe_chars_unchanged() {
        assert_eq!(percent_encode("my-repo_v1.0"), "my-repo_v1.0");
    }

    #[test]
    fn create_project_request_serialises_correctly() {
        let req = CreateProjectRequest {
            name: "my-repo",
            description: "Mirror of my-repo",
            visibility: "private",
            initialize_with_readme: false,
            namespace_path: "alice",
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"name\":\"my-repo\""));
        assert!(json.contains("\"visibility\":\"private\""));
        assert!(json.contains("\"initialize_with_readme\":false"));
        assert!(json.contains("\"namespace_path\":\"alice\""));
    }
}
