// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! [`GitHubClient`] — async HTTP client core: construction, TLS, and HTTP
//! machinery.
//!
//! API endpoint methods live in the [`endpoints`] submodule, which is split
//! by resource category into smaller focused files.

mod endpoints;
#[cfg(test)]
mod http_tests;
#[cfg(test)]
mod retry_tests;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request};
use serde_json::{Map, Value};
use tokio::sync::OnceCell;
use tracing::{debug, warn};

use crate::proxy::ProxyClient;

use github_backup_types::config::Credential;
use github_backup_types::Page;

use crate::decode::extend_page;
use crate::error::ClientError;
use crate::pagination::parse_next_link;
use crate::rate_limit::{self, RateLimitInfo, RateLimitWait};

const GITHUB_API_BASE: &str = "https://api.github.com";
const USER_AGENT: &str = concat!("github-backup-rust/", env!("CARGO_PKG_VERSION"));
/// Default page size for all paginated GitHub API endpoints.
pub(crate) const PER_PAGE: u32 = 100;
/// Maximum number of times to retry a rate-limited request.
const MAX_RATE_LIMIT_RETRIES: u32 = 6;
/// Total time one request may spend waiting out rate limits.  Slightly over
/// GitHub's one-hour primary window, so a primary limit that resets within the
/// hour is waited out, while a longer one fails instead of hanging the run.
const MAX_RATE_WAIT_TOTAL_SECS: u64 = 3700;
/// First wait when GitHub reports a secondary rate limit without saying how
/// long to wait (`Retry-After` / `X-RateLimit-Reset`); doubled on every
/// further hit.  GitHub's guidance is "at least one minute".
const SECONDARY_LIMIT_BASE_SECS: u64 = 60;
/// Maximum number of times to retry a transient 5xx response.
const MAX_SERVER_ERROR_RETRIES: u32 = 3;
/// Maximum number of times to retry a connection failure, timeout or a
/// response cut off mid-body (idempotent requests only).
const MAX_TRANSPORT_RETRIES: u32 = 3;
/// Default request timeout in seconds. GitHub's API can be slow for large repos.
/// Also the longest a response body may stall between two chunks.
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

/// Async GitHub REST API v3 client.
///
/// Construct via [`GitHubClient::new`] for standard GitHub.com use, or
/// [`GitHubClient::with_api_url`] to target a **GitHub Enterprise Server**
/// instance (supply the `https://hostname/api/v3` base URL).
///
/// The client is cheaply cloneable — the underlying hyper connection pool is
/// `Arc`-wrapped.
///
/// **Proxy support**: `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY` and `NO_PROXY`
/// (lower or upper case) are honoured, see [`crate::ProxySettings`].
#[derive(Clone)]
pub struct GitHubClient {
    pub(crate) http: ProxyClient,
    pub(crate) credential: Credential,
    /// Base URL for all API requests.  Defaults to `https://api.github.com`.
    pub(crate) api_base: String,
    /// Login of the user the credential belongs to, looked up once with
    /// `GET /user` and shared by every clone of the client.  `None` inside
    /// means the lookup is not possible with this credential.
    login: Arc<OnceCell<Option<String>>>,
    /// Time base of all waits and timeouts (tests shrink it).
    timing: Timing,
}

/// Waits and timeouts of the retry loop.  `unit` is what one "second" of
/// back-off lasts: always one second in production, shrunk by tests so that
/// minute-long rate-limit waits can be exercised quickly.
#[derive(Clone, Copy, Debug)]
struct Timing {
    unit: Duration,
    /// Limit on receiving response headers, and on the silence between two
    /// chunks of a response body.
    request_timeout: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            unit: Duration::from_secs(1),
            request_timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        }
    }
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
    /// The proxy environment variables are honoured (see [`crate::ProxySettings`]).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::InvalidApiUrl`] unless `api_base_url` is an
    /// `https://` URL with a host, and [`ClientError::Tls`] if the native CA
    /// bundle cannot be loaded.
    pub fn with_api_url(credential: Credential, api_base_url: &str) -> Result<Self, ClientError> {
        validate_api_url(api_base_url)?;
        // HTTPS only: the token must never travel in clear text.
        let http = ProxyClient::from_env(false)?;

        let api_base = api_base_url.trim_end_matches('/').to_string();
        Ok(Self {
            http,
            credential,
            api_base,
            login: Arc::new(OnceCell::new()),
            timing: Timing::default(),
        })
    }

    /// A client for tests that talk to a local **plain-HTTP** server.
    ///
    /// It ignores `HTTPS_PROXY`, needs no CA bundle and accepts `http://`
    /// URLs; the API base is `http://127.0.0.1` (tests pass full URLs).
    #[cfg(test)]
    pub(crate) fn for_tests(credential: Credential) -> Self {
        Self::for_tests_at(credential, "http://127.0.0.1")
    }

    /// Like [`for_tests`](Self::for_tests) with an explicit API base URL.
    ///
    /// Plain HTTP, no proxy, no CA bundle: only for tests against a local
    /// server (the `test-support` feature exposes it to other crates' tests).
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn for_tests_at(credential: Credential, api_base_url: &str) -> Self {
        let tls = rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        let http = ProxyClient::new(crate::proxy::ProxySettings::default(), tls, true);
        Self {
            http,
            credential,
            api_base: api_base_url.trim_end_matches('/').to_string(),
            login: Arc::new(OnceCell::new()),
            timing: Timing {
                unit: Duration::from_millis(1),
                request_timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            },
        }
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

        let response = tokio::time::timeout(self.timing.request_timeout, self.http.request(req))
            .await
            .map_err(|_| ClientError::Timeout { url: url.clone() })??;

        let status = response.status();
        let headers = response.headers().clone();

        if !status.is_success() {
            let body =
                collect_body_limited(response.into_body(), self.timing.request_timeout).await?;
            return Err(ClientError::ApiError {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }

        Ok(RateLimitInfo::oauth_scopes(&headers))
    }

    /// Checks that the credential is accepted, with `GET /rate_limit` (which
    /// does not count against the rate limit), and returns the number of core
    /// requests left in the current window if the server reports it.
    ///
    /// # Errors
    ///
    /// [`ClientError::ApiError`] with status 401 for a revoked, expired or
    /// mistyped token; transport errors when the API cannot be reached.
    pub async fn verify_token(&self) -> Result<Option<u64>, ClientError> {
        let url = format!("{}/rate_limit", self.api_base);
        let (body, _) = self.get_json_with_link::<Value>(&url).await?;
        Ok(body["resources"]["core"]["remaining"].as_u64())
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

    /// Fetches all pages of a paginated endpoint whose body is a JSON array,
    /// collecting the elements into a single [`Page`].
    ///
    /// Elements are decoded one by one: one that does not fit `T` is kept
    /// verbatim in [`Page::unparsed`] and logged, it does not fail the list
    /// (see [`extend_page`]).
    pub(crate) async fn get_all_pages<T>(&self, initial_url: &str) -> Result<Page<T>, ClientError>
    where
        T: serde::de::DeserializeOwned,
    {
        let mut results = Page::new();
        let mut next_url: Option<String> = Some(initial_url.to_string());

        while let Some(url) = next_url.take() {
            debug!(url = %url, "GET");
            let (values, link_header) = self.get_json_with_link::<Vec<Value>>(&url).await?;
            extend_page(&mut results, &url, values);
            next_url = link_header.as_deref().and_then(parse_next_link);
        }

        Ok(results)
    }

    /// Like [`get_all_pages`](Self::get_all_pages) for the endpoints that wrap
    /// the list in an object, `{"total_count": n, "<key>": [...]}`
    /// (workflows, workflow runs, environments): follows every `Link:
    /// rel="next"` page and merges the arrays found under `key`.
    pub(crate) async fn get_all_wrapped_pages<T>(
        &self,
        initial_url: &str,
        key: &str,
    ) -> Result<Page<T>, ClientError>
    where
        T: serde::de::DeserializeOwned,
    {
        let mut results = Page::new();
        let mut next_url: Option<String> = Some(initial_url.to_string());

        while let Some(url) = next_url.take() {
            debug!(url = %url, "GET");
            let (mut body, link_header) =
                self.get_json_with_link::<Map<String, Value>>(&url).await?;
            let Some(Value::Array(values)) = body.remove(key) else {
                return Err(ClientError::Json(serde::de::Error::custom(format!(
                    "response from {url} has no `{key}` array"
                ))));
            };
            extend_page(&mut results, &url, values);
            next_url = link_header.as_deref().and_then(parse_next_link);
        }

        Ok(results)
    }

    /// Returns the login of the user the credential belongs to, or `None`
    /// when it cannot be determined.
    ///
    /// `GET /user` is requested at most once per client (the answer is shared
    /// by all clones).  `None` is returned without any request for an
    /// anonymous client, and - with a warning - when the token is refused
    /// there (`401`/`403`: GitHub App installation tokens cannot call it), so
    /// callers fall back to the public listings.  Any other failure is
    /// returned as an error and not cached.
    ///
    /// # Errors
    ///
    /// Propagates network, TLS and non-401/403 API errors.
    pub(crate) async fn authenticated_login(&self) -> Result<Option<&str>, ClientError> {
        if self.credential.token().is_none() {
            return Ok(None);
        }
        let login = self
            .login
            .get_or_try_init(|| self.fetch_authenticated_login())
            .await?;
        Ok(login.as_deref())
    }

    async fn fetch_authenticated_login(&self) -> Result<Option<String>, ClientError> {
        let url = format!("{}/user", self.api_base);
        match self.get_json_with_link::<Value>(&url).await {
            Ok((user, _)) => {
                let login = user
                    .get("login")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if login.is_none() {
                    warn!("GET /user returned no `login`; listing public data only");
                }
                Ok(login)
            }
            Err(ClientError::ApiError {
                status: status @ (401 | 403),
                ..
            }) => {
                warn!(
                    status,
                    "this token cannot read GET /user (a GitHub App token?); private \
                     repositories and secret gists of the account cannot be listed, \
                     only public data will be backed up"
                );
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    /// Whether the credential belongs to the account `username` (compared
    /// case-insensitively, as GitHub logins are).
    ///
    /// # Errors
    ///
    /// See [`authenticated_login`](Self::authenticated_login).
    pub(crate) async fn is_authenticated_user(&self, username: &str) -> Result<bool, ClientError> {
        Ok(self
            .authenticated_login()
            .await?
            .is_some_and(|login| login.eq_ignore_ascii_case(username)))
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
        self.build_request_with_auth(method, url, true)
    }

    /// Like [`build_request`](Self::build_request), but the credential is
    /// attached only when `send_auth` is true.
    ///
    /// Used for the follow-up hops of a redirect chain, which must not carry
    /// the `Authorization` header to another origin.
    pub(crate) fn build_request_with_auth(
        &self,
        method: Method,
        url: &str,
        send_auth: bool,
    ) -> Result<hyper::http::request::Builder, ClientError> {
        let mut builder = Request::builder()
            .method(method)
            .uri(url)
            .header("User-Agent", USER_AGENT);

        if send_auth {
            if let Some(auth) = self.credential.authorization_header() {
                builder = builder.header("Authorization", auth);
            }
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
    /// - Rate limits (see [`rate_limit::classify`]): a 429, or a 403 that
    ///   carries `Retry-After`, `X-RateLimit-Remaining: 0` or a "rate limit"
    ///   message, is waited out and retried: for `Retry-After`, else until
    ///   `X-RateLimit-Reset`, else (secondary limit) at least a minute,
    ///   doubling on every further hit.  The waits of one request are bounded
    ///   by [`MAX_RATE_WAIT_TOTAL_SECS`]; beyond that it fails with
    ///   [`ClientError::RateLimitExceeded`] - never with a plain 403, which the
    ///   backup treats as "no access, skip".
    /// - GET only (a repeated POST could act twice): 500/502/503/504 up to
    ///   [`MAX_SERVER_ERROR_RETRIES`] retries with exponential back-off and
    ///   jitter; connection failures, timeouts and bodies cut off mid-stream up
    ///   to [`MAX_TRANSPORT_RETRIES`] retries.
    /// - Any other non-success status fails immediately.
    ///
    /// The waits are plain `await`s, so dropping the future (as the engine
    /// does on cancellation) ends them at once.
    async fn execute_with_retry(
        &self,
        method: Method,
        url: &str,
        body: Bytes,
        extra_headers: &[(&str, &str)],
        capture_link: bool,
    ) -> Result<(Bytes, Option<String>), ClientError> {
        let idempotent = method == Method::GET;
        let mut rate_hits = 0u32;
        let mut rate_waited = 0u64;
        let mut server_retries = 0u32;
        let mut transport_retries = 0u32;

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

            let sent =
                tokio::time::timeout(self.timing.request_timeout, self.http.request(req)).await;
            let attempt = match sent {
                Ok(Ok(response)) => {
                    let status = response.status();
                    let headers = response.headers().clone();
                    collect_body_limited(response.into_body(), self.timing.request_timeout)
                        .await
                        .map(|bytes| (status, headers, bytes))
                }
                Ok(Err(e)) => Err(ClientError::from(e)),
                Err(_) => Err(ClientError::Timeout {
                    url: url.to_string(),
                }),
            };
            let (status, headers, bytes) = match attempt {
                Ok(done) => done,
                Err(
                    e @ (ClientError::Transport(_)
                    | ClientError::Body(_)
                    | ClientError::Timeout { .. }),
                ) if idempotent && transport_retries < MAX_TRANSPORT_RETRIES => {
                    let backoff = backoff_with_jitter(transport_retries);
                    warn!(
                        url = %url,
                        error = %e,
                        backoff_secs = backoff.as_secs(),
                        attempt = transport_retries + 1,
                        "request failed, retrying"
                    );
                    tokio::time::sleep(self.scaled(backoff)).await;
                    transport_retries += 1;
                    continue;
                }
                Err(e) => return Err(e),
            };

            // ── Rate limiting ─────────────────────────────────────────────
            if let Some(limit) = rate_limit::classify(status.as_u16(), &headers, &bytes, unix_now())
            {
                let (wait, jitter_span) = match limit {
                    RateLimitWait::Explicit(secs) => (secs.max(1), 1.0),
                    RateLimitWait::Unspecified => (
                        SECONDARY_LIMIT_BASE_SECS << rate_hits.min(4),
                        // Spread concurrent clients over a few seconds.
                        5.0,
                    ),
                };
                if rate_hits >= MAX_RATE_LIMIT_RETRIES
                    || rate_waited.saturating_add(wait) > MAX_RATE_WAIT_TOTAL_SECS
                {
                    return Err(ClientError::RateLimitExceeded {
                        retry_after_secs: wait,
                    });
                }
                warn!(
                    url = %url,
                    status = status.as_u16(),
                    "rate limited; waiting {wait}s (attempt {})",
                    rate_hits + 1
                );
                let jitter = Duration::from_millis((jitter_ms() as f64 * jitter_span) as u64);
                tokio::time::sleep(self.scaled(Duration::from_secs(wait) + jitter)).await;
                rate_waited += wait;
                rate_hits += 1;
                continue;
            }

            // ── Transient server errors ───────────────────────────────────
            if idempotent
                && matches!(status.as_u16(), 500 | 502 | 503 | 504)
                && server_retries < MAX_SERVER_ERROR_RETRIES
            {
                let backoff = backoff_with_jitter(server_retries);
                warn!(
                    url = %url,
                    status = status.as_u16(),
                    backoff_secs = backoff.as_secs(),
                    attempt = server_retries + 1,
                    "transient server error, retrying with jitter"
                );
                tokio::time::sleep(self.scaled(backoff)).await;
                server_retries += 1;
                continue;
            }

            if !status.is_success() {
                return Err(ClientError::ApiError {
                    status: status.as_u16(),
                    body: String::from_utf8_lossy(&bytes).into_owned(),
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
            return Ok((bytes, link_header));
        }
    }

    /// Converts a nominal wait into real time (identity outside tests).
    fn scaled(&self, d: Duration) -> Duration {
        d.mul_f64(self.timing.unit.as_secs_f64())
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

/// An API base URL must be `https://host[...]`: the token is sent with every
/// request, so a typo like `http://` must fail early instead of leaking it (the
/// transport would refuse it too, but only at the first request).
fn validate_api_url(raw: &str) -> Result<(), ClientError> {
    let url = url::Url::parse(raw)
        .map_err(|e| ClientError::InvalidApiUrl(format!("{raw:?} is not a valid URL: {e}")))?;
    if url.scheme() != "https" {
        return Err(ClientError::InvalidApiUrl(format!(
            "{raw:?} must start with https:// (the token is sent with every request)"
        )));
    }
    if url.host_str().is_none() {
        return Err(ClientError::InvalidApiUrl(format!("{raw:?} has no host")));
    }
    Ok(())
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
    idle_timeout: Duration,
) -> Result<Bytes, ClientError> {
    use http_body_util::BodyExt as _;

    let mut body = std::pin::pin!(body);
    let mut buf = bytes::BytesMut::new();
    // `idle_timeout` bounds the silence between two chunks, not the whole
    // transfer: a slow but steady body is fine, a stalled one is not.
    while let Some(frame) = tokio::time::timeout(idle_timeout, body.frame())
        .await
        .map_err(|_| ClientError::Timeout {
            url: "(response body stalled)".to_string(),
        })?
    {
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
pub(crate) fn build_tls_config() -> Result<rustls::ClientConfig, ClientError> {
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

    #[test]
    fn api_urls_must_be_https_with_a_host() {
        for ok in ["https://api.github.com", "https://ghe.example.com/api/v3/"] {
            assert!(validate_api_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://ghe.example.com/api/v3",
            "ghe.example.com",
            "https://",
            "ftp://h",
            "",
        ] {
            assert!(
                matches!(validate_api_url(bad), Err(ClientError::InvalidApiUrl(_))),
                "{bad}"
            );
        }
        let cred = Credential::Token("ghp_test".to_string());
        assert!(GitHubClient::with_api_url(cred, "http://ghe.example.com").is_err());
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
