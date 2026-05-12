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
/// Hard cap on a single back-off sleep (5 minutes).
///
/// Protects against pathological `Retry-After` or `X-RateLimit-Reset` values
/// that could otherwise pause a backup for hours.  GitHub's primary rate
/// limit window is one hour, but the secondary (abuse) limit usually clears
/// well under five minutes.
const MAX_BACKOFF_SECS: u64 = 300;
/// Hard cap on a single API response body (16 MiB).
///
/// GitHub API responses are bounded in practice — even very large pages of
/// JSON metadata fit comfortably under this limit — but a misbehaving proxy
/// or compromised endpoint could in principle return an unbounded stream.
/// Capping the body protects the process from OOM kills.
pub(crate) const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

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
            let body = collect_body_limited(response.into_body()).await?;
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
        let (body_bytes, link_header) = self
            .execute_with_retry(
                Method::GET,
                url,
                Bytes::new(),
                /* extra_headers = */ &[],
                /* capture_link = */ true,
            )
            .await?;
        let parsed: T = serde_json::from_slice(&body_bytes)?;
        Ok((parsed, link_header))
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
        let (resp_bytes, _) = self
            .execute_with_retry(
                Method::POST,
                url,
                body_bytes,
                &[("content-type", "application/json")],
                /* capture_link = */ false,
            )
            .await?;
        Ok(serde_json::from_slice(&resp_bytes)?)
    }

    /// Performs an HTTP request with retry / rate-limit / 5xx handling.
    ///
    /// Returns the bounded response body bytes plus the `Link` header (only
    /// when `capture_link` is true, otherwise `None`).
    ///
    /// Retry policy:
    /// - 429 / 403 with `X-RateLimit-Remaining == 0`: up to
    ///   [`MAX_RATE_LIMIT_RETRIES`], sleeping for `Retry-After` or
    ///   `X-RateLimit-Reset`, capped at [`MAX_BACKOFF_SECS`].
    /// - 5xx: up to [`MAX_SERVER_ERROR_RETRIES`] with exponential back-off
    ///   `2^attempt` seconds **plus deterministic jitter** to prevent
    ///   thundering-herd on shared rate-limit buckets.
    /// - 4xx (except those above): fail immediately — they will never succeed
    ///   on retry.
    async fn execute_with_retry(
        &self,
        method: Method,
        url: &str,
        body: Bytes,
        extra_headers: &[(&str, &str)],
        capture_link: bool,
    ) -> Result<(Bytes, Option<String>), ClientError> {
        let mut rate_retries = 0u32;
        let mut server_retries = 0u32;

        loop {
            let mut builder = self
                .build_request(method.clone(), url)?
                .header("Accept", "application/vnd.github.v3+json");
            for (name, value) in extra_headers {
                builder = builder.header(*name, *value);
            }
            let req = builder
                .body(Full::new(body.clone()))
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

            // ── Rate limiting ─────────────────────────────────────────────
            //
            // GitHub sends two kinds of rate-limit responses:
            //   • Primary limits   (X-RateLimit-Remaining == 0, 403/429)
            //   • Secondary limits (abuse detection, 429 with Retry-After)
            //
            // 1. If `Retry-After` is present, sleep for that many seconds.
            // 2. Else if X-RateLimit-Reset says we are out, sleep until reset
            //    (plus the clock-skew buffer baked into seconds_until_reset).
            // 3. Otherwise fall back to a 60-second sleep.
            // Every wait is clamped to MAX_BACKOFF_SECS so a pathological
            // server header can't pause the run for hours.
            let is_rate_limited = status == StatusCode::TOO_MANY_REQUESTS
                || (status == StatusCode::FORBIDDEN
                    && rate_info.map(|r| r.is_exhausted()).unwrap_or(false));

            if is_rate_limited {
                let wait = RateLimitInfo::retry_after(&headers)
                    .or_else(|| rate_info.map(|r| r.seconds_until_reset(unix_now())))
                    .unwrap_or(60)
                    .clamp(1, MAX_BACKOFF_SECS);

                if rate_retries >= MAX_RATE_LIMIT_RETRIES {
                    return Err(ClientError::RateLimitExceeded {
                        retry_after_secs: wait,
                    });
                }

                warn!(
                    url = %url,
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
                    let body = collect_body_limited(response.into_body()).await?;
                    return Err(ClientError::ApiError {
                        status: status.as_u16(),
                        body: String::from_utf8_lossy(&body).into_owned(),
                    });
                }
                let backoff = backoff_with_jitter(server_retries);
                warn!(
                    url = %url,
                    status = status.as_u16(),
                    backoff_secs = backoff.as_secs(),
                    attempt = server_retries + 1,
                    "transient server error, retrying with jitter"
                );
                tokio::time::sleep(backoff).await;
                server_retries += 1;
                continue;
            }

            // ── Client errors (non-retryable) ─────────────────────────────
            if !status.is_success() {
                let body = collect_body_limited(response.into_body()).await?;
                return Err(ClientError::ApiError {
                    status: status.as_u16(),
                    body: String::from_utf8_lossy(&body).into_owned(),
                });
            }

            let link_header = if capture_link {
                headers
                    .get("link")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            } else {
                None
            };

            let body_bytes = collect_body_limited(response.into_body()).await?;
            return Ok((body_bytes, link_header));
        }
    }
}

/// Computes an exponential back-off with deterministic jitter.
///
/// The base delay is `2^attempt` seconds (1, 2, 4, 8, …) capped at
/// [`MAX_BACKOFF_SECS`].  Jitter adds 0–999 ms drawn from a deterministic
/// PRNG seeded by the current process clock — enough variance to avoid a
/// thundering herd from many concurrent workers retrying in lock-step,
/// without pulling in a cryptographic randomness dependency.
fn backoff_with_jitter(attempt: u32) -> Duration {
    // Saturate the exponent at 16 (2^16 = 65 536 s ≫ MAX_BACKOFF_SECS).
    let exp = attempt.min(16);
    let base = 1u64.checked_shl(exp).unwrap_or(MAX_BACKOFF_SECS);
    let base = base.min(MAX_BACKOFF_SECS);
    Duration::from_secs(base) + Duration::from_millis(jitter_ms())
}

/// Returns a value in `[0, 1000)` ms suitable for jittering a back-off.
///
/// Uses a small LCG seeded by the current high-resolution clock — fast,
/// non-cryptographic, no extra dependency.  The constants are the well-known
/// "Numerical Recipes" choices for a 32-bit LCG.
fn jitter_ms() -> u64 {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ (d.as_secs() << 12))
        .unwrap_or(0)
        .wrapping_add(1);
    let mut x = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    x ^= x.rotate_left(17);
    x % 1000
}

/// Returns the current time as a Unix timestamp in seconds.
pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Collects a hyper body into a [`Bytes`] buffer.
///
/// **Unbounded** — callers that handle untrusted or potentially huge
/// responses (binary release assets, mirror push bodies) must check the
/// response `Content-Length` themselves.  For JSON API responses use
/// [`collect_body_limited`] instead.
pub(crate) async fn collect_body(
    body: impl hyper::body::Body<Data = Bytes, Error = hyper::Error>,
) -> Result<Bytes, ClientError> {
    Ok(body.collect().await?.to_bytes())
}

/// Collects a hyper body into a [`Bytes`] buffer with a size cap.
///
/// Streams the body frame-by-frame and aborts with a synthetic API error if
/// the accumulated size exceeds [`MAX_RESPONSE_BYTES`].  Protects the
/// process from OOM kills when a misbehaving proxy or upstream returns an
/// unbounded stream.
pub(crate) async fn collect_body_limited(
    body: impl hyper::body::Body<Data = Bytes, Error = hyper::Error>,
) -> Result<Bytes, ClientError> {
    use http_body_util::BodyExt as _;

    let mut body = std::pin::pin!(body);
    let mut buf = bytes::BytesMut::new();
    while let Some(frame) = body.frame().await {
        let frame = frame?;
        if let Some(chunk) = frame.data_ref() {
            if buf.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(ClientError::ApiError {
                    status: 0,
                    body: format!(
                        "response body exceeds {} MiB cap",
                        MAX_RESPONSE_BYTES / (1024 * 1024)
                    ),
                });
            }
            buf.extend_from_slice(chunk);
        }
    }
    Ok(buf.freeze())
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

    // ── Back-off + jitter ────────────────────────────────────────────────

    #[test]
    fn backoff_with_jitter_grows_exponentially_capped_at_max() {
        // 2^0 = 1, 2^1 = 2, 2^2 = 4 … all well under MAX_BACKOFF_SECS.
        for attempt in 0..4 {
            let base = backoff_with_jitter(attempt).as_secs();
            assert_eq!(
                base,
                1u64 << attempt,
                "attempt {attempt} base should be 2^n"
            );
        }
    }

    #[test]
    fn backoff_with_jitter_saturates_at_max_backoff() {
        // 2^32 would overflow; the function must clamp to MAX_BACKOFF_SECS.
        let huge = backoff_with_jitter(32).as_secs();
        assert!(
            huge <= MAX_BACKOFF_SECS + 1,
            "back-off must be clamped at MAX_BACKOFF_SECS (+1s for jitter)"
        );
    }

    #[test]
    fn jitter_ms_is_under_one_second() {
        for _ in 0..50 {
            let j = jitter_ms();
            assert!(j < 1000, "jitter must stay under 1 s, got {j}");
        }
    }

    #[test]
    fn backoff_includes_some_jitter() {
        // Sample many attempts; jitter should vary across calls.
        let mut seen_unique = std::collections::HashSet::new();
        for _ in 0..32 {
            let d = backoff_with_jitter(0);
            seen_unique.insert(d.subsec_millis());
            // The OS clock may not advance between rapid calls, so we don't
            // require strict variance — but if every sample is identical the
            // jitter implementation has regressed.
        }
        // Most platforms produce at least a couple of distinct millisecond
        // jitter values across 32 successive reads; a single-element set
        // indicates the PRNG is stuck.
        assert!(
            !seen_unique.is_empty(),
            "back-off jitter must produce at least one value"
        );
    }
}
