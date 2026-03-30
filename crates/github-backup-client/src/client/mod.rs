// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! [`GitHubClient`] — async HTTP client core: construction, TLS, and HTTP
//! machinery.
//!
//! API endpoint methods live in the [`endpoints`] submodule, which is split
//! by resource category into smaller focused files.

mod endpoints;
mod proxy;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use tracing::{debug, info, warn};

use proxy::ProxyConnector;

use github_backup_types::config::Credential;

use crate::error::ClientError;
use crate::pagination::parse_next_link;
use crate::rate_limit::RateLimitInfo;

const GITHUB_API_BASE: &str = "https://api.github.com";
const USER_AGENT: &str = concat!("github-backup-rust/", env!("CARGO_PKG_VERSION"));
/// Default page size for all paginated GitHub API endpoints.
pub(crate) const PER_PAGE: u32 = 100;
/// Maximum number of times to retry a rate-limited request.
const MAX_RATE_LIMIT_RETRIES: u32 = 3;
/// Maximum number of times to retry a transient 5xx response.
const MAX_SERVER_ERROR_RETRIES: u32 = 3;
/// Default request timeout in seconds. GitHub's API can be slow for large repos.
pub(crate) const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Backing HTTP client — either a direct TLS connection or a CONNECT-tunnelled
/// proxy connection.  Both variants share the same `hyper_util::client::legacy`
/// error type so call sites need no special casing.
#[derive(Clone)]
pub(crate) enum HyperClientKind {
    Direct(Client<hyper_rustls::HttpsConnector<HttpConnector>, Full<Bytes>>),
    Proxied(Client<ProxyConnector, Full<Bytes>>),
}

impl HyperClientKind {
    async fn request(
        &self,
        req: hyper::Request<Full<Bytes>>,
    ) -> Result<hyper::Response<hyper::body::Incoming>, hyper_util::client::legacy::Error> {
        match self {
            HyperClientKind::Direct(c) => c.request(req).await,
            HyperClientKind::Proxied(c) => c.request(req).await,
        }
    }
}

/// Async GitHub REST API v3 client.
///
/// Construct via [`GitHubClient::new`] for standard GitHub.com use, or
/// [`GitHubClient::with_api_url`] to target a **GitHub Enterprise Server**
/// instance (supply the `https://hostname/api/v3` base URL).
///
/// The client is cheaply cloneable — the underlying hyper connection pool is
/// `Arc`-wrapped.
///
/// **Proxy support**: if `HTTPS_PROXY` (or `https_proxy`) is set in the
/// environment the client automatically routes all connections through the
/// proxy via HTTP `CONNECT` tunnelling.  Credentials embedded in the URL
/// (`http://user:pass@host:port`) are forwarded as a `Proxy-Authorization`
/// header.
#[derive(Clone)]
pub struct GitHubClient {
    pub(crate) http: HyperClientKind,
    pub(crate) credential: Credential,
    /// Base URL for all API requests.  Defaults to `https://api.github.com`.
    pub(crate) api_base: String,
}

impl std::fmt::Debug for GitHubClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubClient")
            .field("credential", &"[redacted]")
            .finish()
    }
}

impl GitHubClient {
    /// Creates a new [`GitHubClient`] targeting `https://api.github.com`.
    ///
    /// For GitHub Enterprise Server use [`GitHubClient::with_api_url`].
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Tls`] if the native CA bundle cannot be loaded.
    pub fn new(credential: Credential) -> Result<Self, ClientError> {
        Self::with_api_url(credential, GITHUB_API_BASE)
    }

    /// Creates a new [`GitHubClient`] targeting the given `api_base_url`.
    ///
    /// Use this for **GitHub Enterprise Server** instances, where the API is
    /// typically at `https://github.example.com/api/v3`.  The URL is stored
    /// verbatim and used as the prefix for all API requests.
    ///
    /// If `HTTPS_PROXY` (or `https_proxy`) is set in the environment, the
    /// client will route HTTPS requests through that proxy via HTTP `CONNECT`.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Tls`] if the native CA bundle cannot be loaded.
    pub fn with_api_url(credential: Credential, api_base_url: &str) -> Result<Self, ClientError> {
        let http = if let Some(proxy_config) = proxy::proxy_config_from_env() {
            info!(
                host = %proxy_config.host,
                port = proxy_config.port,
                "routing GitHub API calls through HTTPS proxy"
            );
            let tls_config = build_tls_config()?;
            let connector = ProxyConnector::new(proxy_config, tls_config);
            HyperClientKind::Proxied(Client::builder(TokioExecutor::new()).build(connector))
        } else {
            let tls_config = build_tls_config()?;
            let https = hyper_rustls::HttpsConnectorBuilder::new()
                .with_tls_config(tls_config)
                .https_only()
                .enable_http1()
                .build();
            HyperClientKind::Direct(Client::builder(TokioExecutor::new()).build(https))
        };

        let api_base = api_base_url.trim_end_matches('/').to_string();
        Ok(Self {
            http,
            credential,
            api_base,
        })
    }

    /// Returns the API base URL (without trailing slash).
    ///
    /// Used by endpoint methods to build request URLs.
    #[must_use]
    pub(crate) fn api(&self) -> &str {
        &self.api_base
    }

    /// Checks whether the current token has the required OAuth scopes.
    ///
    /// Makes a lightweight `GET /user` request and inspects the
    /// `X-OAuth-Scopes` response header.  Returns the list of granted scopes.
    ///
    /// Fine-grained PATs do not use the `X-OAuth-Scopes` model; for those
    /// tokens the header is absent and an empty `Vec` is returned — the caller
    /// should not treat that as an error.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on network or API errors.
    pub async fn get_token_scopes(&self) -> Result<Vec<String>, ClientError> {
        let url = format!("{}/user", self.api_base);
        let req = self
            .build_request(Method::GET, &url)?
            .header("Accept", "application/vnd.github.v3+json")
            .body(Full::new(Bytes::new()))
            .map_err(ClientError::Http)?;

        let response = tokio::time::timeout(
            Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            self.http.request(req),
        )
        .await
        .map_err(|_| ClientError::Timeout { url: url.clone() })??;

        let status = response.status();
        let headers = response.headers().clone();

        if !status.is_success() {
            let body = collect_body(response.into_body()).await?;
            return Err(ClientError::ApiError {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }

        Ok(RateLimitInfo::oauth_scopes(&headers))
    }

    /// Returns the raw token string if the credential is a [`Credential::Token`],
    /// or `None` for anonymous / other credential types.
    ///
    /// Used by the backup engine to inject the token into git clone commands
    /// for HTTPS authentication on private repositories.
    #[must_use]
    pub fn token(&self) -> Option<String> {
        match &self.credential {
            Credential::Token(t) => Some(t.clone()),
            Credential::Anonymous => None,
        }
    }

    // ── Internal HTTP machinery ───────────────────────────────────────────

    /// Fetches all pages of a paginated endpoint, collecting results into
    /// a single `Vec<T>`.
    pub(crate) async fn get_all_pages<T>(&self, initial_url: &str) -> Result<Vec<T>, ClientError>
    where
        T: serde::de::DeserializeOwned,
    {
        let mut results = Vec::new();
        let mut next_url: Option<String> = Some(initial_url.to_string());

        while let Some(url) = next_url.take() {
            debug!(url = %url, "GET");
            let (page, link_header) = self.get_json_with_link::<Vec<T>>(&url).await?;
            results.extend(page);
            next_url = link_header.as_deref().and_then(parse_next_link);
        }

        Ok(results)
    }

    /// Performs a single GET request and returns the deserialised body along
    /// with the raw `Link` header value (if present).
    ///
    /// Handles rate limiting (403/429) with exponential back-off and retries
    /// transient 5xx server errors up to [`MAX_SERVER_ERROR_RETRIES`] times.
    pub(crate) async fn get_json_with_link<T>(
        &self,
        url: &str,
    ) -> Result<(T, Option<String>), ClientError>
    where
        T: serde::de::DeserializeOwned,
    {
        let mut rate_retries = 0u32;
        let mut server_retries = 0u32;

        loop {
            let req = self
                .build_request(Method::GET, url)?
                .header("Accept", "application/vnd.github.v3+json")
                .body(Full::new(Bytes::new()))
                .map_err(ClientError::Http)?;

            let response = tokio::time::timeout(
                Duration::from_secs(DEFAULT_TIMEOUT_SECS),
                self.http.request(req),
            )
            .await
            .map_err(|_| ClientError::Timeout {
                url: url.to_string(),
            })??;

            let status = response.status();
            let headers = response.headers().clone();
            let rate_info = RateLimitInfo::from_headers(&headers);

            // ── Rate limiting ──────────────────────────────────────────────
            //
            // GitHub sends two kinds of rate-limit responses:
            //   • Primary limits   (X-RateLimit-Remaining == 0, 403/429)
            //   • Secondary limits (abuse detection, 429 with Retry-After)
            //
            // We handle both:
            //   1. If Retry-After is present, sleep for that many seconds.
            //   2. If X-RateLimit-Reset is present and remaining is 0, sleep
            //      until the precise reset time (plus a clock-skew buffer).
            //   3. Otherwise fall back to a 60-second sleep.
            let is_rate_limited = status == StatusCode::TOO_MANY_REQUESTS
                || (status == StatusCode::FORBIDDEN
                    && rate_info.map(|r| r.is_exhausted()).unwrap_or(false));

            if is_rate_limited {
                if rate_retries >= MAX_RATE_LIMIT_RETRIES {
                    let wait = RateLimitInfo::retry_after(&headers)
                        .or_else(|| rate_info.map(|r| r.seconds_until_reset(unix_now())))
                        .unwrap_or(60);
                    return Err(ClientError::RateLimitExceeded {
                        retry_after_secs: wait,
                    });
                }

                let wait = RateLimitInfo::retry_after(&headers)
                    .or_else(|| rate_info.map(|r| r.seconds_until_reset(unix_now())))
                    .unwrap_or(60)
                    .max(1);

                warn!(
                    wait_secs = wait,
                    attempt = rate_retries + 1,
                    "rate limit hit, sleeping until reset"
                );
                tokio::time::sleep(Duration::from_secs(wait)).await;
                rate_retries += 1;
                continue;
            }

            // ── Transient server errors (5xx) ─────────────────────────────
            if status.is_server_error() {
                if server_retries >= MAX_SERVER_ERROR_RETRIES {
                    let body = collect_body(response.into_body()).await?;
                    return Err(ClientError::ApiError {
                        status: status.as_u16(),
                        body: String::from_utf8_lossy(&body).into_owned(),
                    });
                }
                let backoff = Duration::from_secs(2u64.pow(server_retries));
                warn!(
                    status = status.as_u16(),
                    backoff_secs = backoff.as_secs(),
                    attempt = server_retries + 1,
                    "transient server error, retrying"
                );
                tokio::time::sleep(backoff).await;
                server_retries += 1;
                continue;
            }

            // ── Client errors ─────────────────────────────────────────────
            if !status.is_success() {
                let body = collect_body(response.into_body()).await?;
                return Err(ClientError::ApiError {
                    status: status.as_u16(),
                    body: String::from_utf8_lossy(&body).into_owned(),
                });
            }

            let link_header = headers
                .get("link")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);

            let body = collect_body(response.into_body()).await?;
            let parsed: T = serde_json::from_slice(&body)?;

            return Ok((parsed, link_header));
        }
    }

    /// Builds a [`hyper::http::request::Builder`] pre-populated with auth
    /// and user-agent headers.
    ///
    /// The `Authorization` header is omitted for [`Credential::Anonymous`]
    /// so that GitHub's unauthenticated rate-limit bucket applies.
    pub(crate) fn build_request(
        &self,
        method: Method,
        url: &str,
    ) -> Result<hyper::http::request::Builder, ClientError> {
        let mut builder = Request::builder()
            .method(method)
            .uri(url)
            .header("User-Agent", USER_AGENT);

        if let Some(auth) = self.credential.authorization_header() {
            builder = builder.header("Authorization", auth);
        }

        Ok(builder)
    }

    /// Performs a single POST request with a JSON body and deserialises the
    /// response.
    ///
    /// Handles rate limiting (403/429) and transient 5xx errors identically to
    /// [`get_json_with_link`][Self::get_json_with_link].
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on network, TLS, API, or deserialisation errors.
    pub(crate) async fn post_json<T, B>(&self, url: &str, body: &B) -> Result<T, ClientError>
    where
        T: serde::de::DeserializeOwned,
        B: serde::Serialize,
    {
        let body_bytes = Bytes::from(serde_json::to_vec(body)?);
        let mut rate_retries = 0u32;
        let mut server_retries = 0u32;

        loop {
            let req = self
                .build_request(Method::POST, url)?
                .header("Accept", "application/vnd.github.v3+json")
                .header("Content-Type", "application/json")
                .body(Full::new(body_bytes.clone()))
                .map_err(ClientError::Http)?;

            let response = tokio::time::timeout(
                Duration::from_secs(DEFAULT_TIMEOUT_SECS),
                self.http.request(req),
            )
            .await
            .map_err(|_| ClientError::Timeout {
                url: url.to_string(),
            })??;

            let status = response.status();
            let headers = response.headers().clone();
            let rate_info = RateLimitInfo::from_headers(&headers);

            let is_rate_limited = status == StatusCode::TOO_MANY_REQUESTS
                || (status == StatusCode::FORBIDDEN
                    && rate_info.map(|r| r.is_exhausted()).unwrap_or(false));

            if is_rate_limited {
                if rate_retries >= MAX_RATE_LIMIT_RETRIES {
                    let wait = RateLimitInfo::retry_after(&headers)
                        .or_else(|| rate_info.map(|r| r.seconds_until_reset(unix_now())))
                        .unwrap_or(60);
                    return Err(ClientError::RateLimitExceeded {
                        retry_after_secs: wait,
                    });
                }
                let wait = RateLimitInfo::retry_after(&headers)
                    .or_else(|| rate_info.map(|r| r.seconds_until_reset(unix_now())))
                    .unwrap_or(60)
                    .max(1);
                warn!(
                    wait_secs = wait,
                    attempt = rate_retries + 1,
                    "rate limit hit during POST, sleeping"
                );
                tokio::time::sleep(Duration::from_secs(wait)).await;
                rate_retries += 1;
                continue;
            }

            if status.is_server_error() {
                if server_retries >= MAX_SERVER_ERROR_RETRIES {
                    let body = collect_body(response.into_body()).await?;
                    return Err(ClientError::ApiError {
                        status: status.as_u16(),
                        body: String::from_utf8_lossy(&body).into_owned(),
                    });
                }
                let backoff = Duration::from_secs(2u64.pow(server_retries));
                warn!(
                    status = status.as_u16(),
                    backoff_secs = backoff.as_secs(),
                    "transient server error on POST, retrying"
                );
                tokio::time::sleep(backoff).await;
                server_retries += 1;
                continue;
            }

            if !status.is_success() {
                let body = collect_body(response.into_body()).await?;
                return Err(ClientError::ApiError {
                    status: status.as_u16(),
                    body: String::from_utf8_lossy(&body).into_owned(),
                });
            }

            let body = collect_body(response.into_body()).await?;
            return Ok(serde_json::from_slice(&body)?);
        }
    }
}

/// Returns the current time as a Unix timestamp in seconds.
pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Collects a hyper body into a [`Bytes`] buffer.
pub(crate) async fn collect_body(
    body: impl hyper::body::Body<Data = Bytes, Error = hyper::Error>,
) -> Result<Bytes, ClientError> {
    Ok(body.collect().await?.to_bytes())
}

/// Builds a [`rustls::ClientConfig`] using the system native CA bundle.
fn build_tls_config() -> Result<rustls::ClientConfig, ClientError> {
    let mut root_store = rustls::RootCertStore::empty();
    let cert_result = rustls_native_certs::load_native_certs();
    if cert_result.certs.is_empty() {
        let msg = cert_result
            .errors
            .first()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "no CA certificates found".to_string());
        return Err(ClientError::Tls(msg));
    }
    root_store.add_parsable_certificates(cert_result.certs);
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use github_backup_types::config::Credential;

    #[test]
    fn github_client_new_succeeds_with_token() {
        let cred = Credential::Token("ghp_test".to_string());
        let result = GitHubClient::new(cred);
        assert!(result.is_ok(), "client construction should succeed");
    }

    #[test]
    fn github_client_debug_redacts_credential() {
        let cred = Credential::Token("secret_token".to_string());
        let client = GitHubClient::new(cred).expect("construct client");
        let debug_str = format!("{client:?}");
        assert!(
            !debug_str.contains("secret_token"),
            "credential must be redacted in Debug output"
        );
        assert!(debug_str.contains("[redacted]"));
    }

    #[test]
    fn github_client_token_returns_token_string() {
        let cred = Credential::Token("ghp_mytoken".to_string());
        let client = GitHubClient::new(cred).expect("construct client");
        assert_eq!(client.token(), Some("ghp_mytoken".to_string()));
    }

    #[test]
    fn github_client_default_api_base_is_github() {
        let cred = Credential::Token("ghp_test".to_string());
        let client = GitHubClient::new(cred).expect("construct client");
        assert_eq!(client.api(), "https://api.github.com");
    }

    #[test]
    fn github_client_with_api_url_uses_custom_base() {
        let cred = Credential::Token("ghp_test".to_string());
        let client =
            GitHubClient::with_api_url(cred, "https://github.example.com/api/v3").expect("client");
        assert_eq!(client.api(), "https://github.example.com/api/v3");
    }

    #[test]
    fn github_client_with_api_url_strips_trailing_slash() {
        let cred = Credential::Token("ghp_test".to_string());
        let client =
            GitHubClient::with_api_url(cred, "https://github.example.com/api/v3/").expect("client");
        assert_eq!(client.api(), "https://github.example.com/api/v3");
    }
}
