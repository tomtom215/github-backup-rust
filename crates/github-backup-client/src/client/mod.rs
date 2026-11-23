// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! [`GitHubClient`] — async HTTP client core: construction, TLS, and HTTP
//! machinery.
//!
//! API endpoint methods live in the [`endpoints`] submodule, which is split
//! by resource category into smaller focused files.

mod endpoints;
mod graphql;
#[cfg(test)]
mod http_tests;
mod redirect;
#[cfg(test)]
mod retry_tests;

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use serde_json::{Map, Value};
use tokio::sync::OnceCell;
use tokio::time::Instant;
use tracing::{debug, warn};
use url::Url;

use crate::proxy::ProxyClient;

use github_backup_types::config::Credential;
use github_backup_types::Page;

use crate::decode::extend_page;
use crate::error::ClientError;
use crate::pagination::parse_next_link;
use crate::rate_limit::{self, RateLimitInfo, RateLimitWait};

use redirect::{Hop, MAX_REDIRECTS};

const GITHUB_API_BASE: &str = "https://api.github.com";
pub(crate) const USER_AGENT: &str = concat!("github-backup-rust/", env!("CARGO_PKG_VERSION"));
/// The media type GitHub recommends for REST API requests.
const ACCEPT_GITHUB_JSON: &str = "application/vnd.github+json";
/// The request header that selects the REST API version (`X-GitHub-Api-Version`,
/// lower case as `HeaderName::from_static` requires).
pub(crate) const API_VERSION_HEADER: &str = "x-github-api-version";
/// The REST API version every request asks for, unless it names another one.
///
/// `2022-11-28` is the version GitHub applies to requests without the header
/// and is supported until 2028-03-10.  The newer `2026-03-10` removes fields
/// this tool backs up losslessly (`merge_commit_sha` and `assignee` from pull
/// requests and issues, `has_downloads` from repositories, `history` and
/// `forks` from gists), and GitHub Enterprise Server releases that predate it
/// reject it.  An endpoint that needs another version passes the header per
/// request (see [`GitHubClient::get_json_with_headers`]).
const API_VERSION: &str = "2022-11-28";
/// Default page size for all paginated GitHub API endpoints.
pub(crate) const PER_PAGE: u32 = 100;
/// Maximum number of times to retry a rate-limited request.
const MAX_RATE_LIMIT_RETRIES: u32 = 6;
/// Longest single rate-limit wait.  Slightly over GitHub's one-hour primary
/// window, so a primary limit that resets within the hour is waited out,
/// while a longer one fails instead of hanging the run.
const MAX_RATE_WAIT_SECS: u64 = 3700;
/// Total time one request may spend waiting out rate limits: two windows, so
/// that a few secondary limits followed by a primary limit that resets in an
/// hour are still waited out.
const MAX_RATE_WAIT_TOTAL_SECS: u64 = 2 * MAX_RATE_WAIT_SECS;
/// First wait when GitHub reports a secondary rate limit without saying how
/// long to wait (`Retry-After` / `X-RateLimit-Reset`); doubled on every
/// further hit.  GitHub's guidance is "at least one minute".
const SECONDARY_LIMIT_BASE_SECS: u64 = 60;
/// Maximum number of times to retry a transient 5xx response.
const MAX_SERVER_ERROR_RETRIES: u32 = 3;
/// Maximum number of times to retry a connection failure, timeout or a
/// response cut off mid-body (idempotent requests only).
const MAX_TRANSPORT_RETRIES: u32 = 3;
/// Default limit, in seconds, on receiving the response headers, and on the
/// silence between two chunks of a response body.  It does not bound a whole
/// transfer: a slow but steady body is fine, a stalled one is not.
pub(crate) const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// Cap on one back-off sleep after a 5xx response or a transport failure
/// (5 minutes).  Rate-limit waits are bounded by [`MAX_RATE_WAIT_SECS`]
/// instead.
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
/// `Arc`-wrapped, and so is the rate-limit back-off: when one request is told
/// to wait, every clone waits with it.
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
    /// When GitHub last asked this client (any clone) to stop sending.
    rate_gate: Arc<RateGate>,
    /// Endpoints whose deprecation notice has been logged already.
    deprecations: Arc<Mutex<HashSet<String>>>,
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

/// The instant before which no request may be sent, shared by every clone of
/// a client.  GitHub's guidance after a rate limit is to stop sending, not
/// only to retry the request that was refused, so a back-off closes the gate
/// for all concurrent workers.
#[derive(Debug, Default)]
struct RateGate(Mutex<Option<Instant>>);

impl RateGate {
    /// Keeps the gate closed until at least `until`.
    fn close_until(&self, until: Instant) {
        let mut gate = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if gate.is_none_or(|current| current < until) {
            *gate = Some(until);
        }
    }

    /// Waits until the gate is open.
    async fn pass(&self) {
        let until = *self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(until) = until {
            tokio::time::sleep_until(until).await;
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

/// One request as the retry loop sends it.
struct Call<'a> {
    method: Method,
    url: &'a str,
    body: Bytes,
    /// Headers that replace the defaults of the same name (`Accept`,
    /// `X-GitHub-Api-Version`) or are added to them.
    headers: &'a [(&'a str, &'a str)],
    /// Whether the request may be repeated after a 5xx response or a
    /// transport failure (a repeated POST could act twice).
    idempotent: bool,
    /// Follow a redirect to another origin, without the credential, instead
    /// of failing (release assets live on a storage host).
    leave_origin: bool,
    /// A GraphQL request: a `200` can still be a rate limit.
    graphql: bool,
    /// What the call's origin is sent besides `call.headers`.
    origin: OriginHeaders,
}

/// The headers a [`Call`] sends to its own origin (never to another one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OriginHeaders {
    /// An API request: the credential as a bearer token, the API media type
    /// and version.
    Api,
    /// A request to GitHub's web host (an attachment): only the credential,
    /// as HTTP Basic authentication `<token>:x-oauth-basic`.
    WebBasic,
    /// Nothing: the host is not GitHub's API or web host.
    None,
}

impl<'a> Call<'a> {
    fn get(url: &'a str, headers: &'a [(&'a str, &'a str)]) -> Self {
        Self {
            method: Method::GET,
            url,
            body: Bytes::new(),
            headers,
            idempotent: true,
            leave_origin: false,
            graphql: false,
            origin: OriginHeaders::Api,
        }
    }
}

/// What one request has used up of its retry budget.
#[derive(Debug, Default)]
struct Attempts {
    rate_hits: u32,
    rate_waited: u64,
    server_errors: u32,
    transport_failures: u32,
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
        Ok(Self::from_parts(
            http,
            credential,
            api_base_url,
            Timing::default(),
        ))
    }

    fn from_parts(
        http: ProxyClient,
        credential: Credential,
        api_base: &str,
        timing: Timing,
    ) -> Self {
        Self {
            http,
            credential,
            api_base: api_base.trim_end_matches('/').to_string(),
            login: Arc::new(OnceCell::new()),
            rate_gate: Arc::new(RateGate::default()),
            deprecations: Arc::new(Mutex::new(HashSet::new())),
            timing,
        }
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
        Self::from_parts(
            crate::proxy::ProxyClient::for_tests(),
            credential,
            api_base_url,
            Timing {
                unit: Duration::from_millis(1),
                request_timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            },
        )
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
        let (_, headers) = self.execute(&Call::get(&url, &[])).await?;
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
    /// (see [`extend_page`]).  Pagination stops with
    /// [`ClientError::Pagination`] when a next-page link leads to another
    /// origin or back to a page already fetched.
    pub(crate) async fn get_all_pages<T>(&self, initial_url: &str) -> Result<Page<T>, ClientError>
    where
        T: serde::de::DeserializeOwned,
    {
        self.get_all_pages_with_headers(initial_url, &[]).await
    }

    /// Like [`get_all_pages`](Self::get_all_pages), sending `headers` with
    /// every page (see [`get_json_with_headers`](Self::get_json_with_headers)).
    pub(crate) async fn get_all_pages_with_headers<T>(
        &self,
        initial_url: &str,
        headers: &[(&str, &str)],
    ) -> Result<Page<T>, ClientError>
    where
        T: serde::de::DeserializeOwned,
    {
        self.collect_pages(initial_url, headers, |values: Vec<Value>, _| Ok(values))
            .await
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
        self.collect_pages(
            initial_url,
            &[],
            |mut body: Map<String, Value>, url| match body.remove(key) {
                Some(Value::Array(values)) => Ok(values),
                _ => Err(ClientError::Json(serde::de::Error::custom(format!(
                    "response from {url} has no `{key}` array"
                )))),
            },
        )
        .await
    }

    /// Follows the `Link: rel="next"` chain from `initial_url`, taking the
    /// elements of every page out of its body `B` with `values_of`.
    async fn collect_pages<T, B>(
        &self,
        initial_url: &str,
        headers: &[(&str, &str)],
        values_of: impl Fn(B, &str) -> Result<Vec<Value>, ClientError>,
    ) -> Result<Page<T>, ClientError>
    where
        T: serde::de::DeserializeOwned,
        B: serde::de::DeserializeOwned,
    {
        let mut results = Page::new();
        let mut fetched = HashSet::new();
        let mut next_url = Some(String::from(Url::parse(initial_url)?));

        while let Some(url) = next_url.take() {
            debug!(url = %url, "GET");
            let (body, link_header) = self.get_json_with_headers::<B>(&url, headers).await?;
            extend_page(&mut results, &url, values_of(body, &url)?);
            fetched.insert(url.clone());
            next_url = link_header
                .as_deref()
                .and_then(parse_next_link)
                .map(|link| next_page(&url, &link, &fetched))
                .transpose()?;
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
    /// Redirects, rate limits and transient failures are handled as
    /// described at [`send`](Self::send).
    pub(crate) async fn get_json_with_link<T>(
        &self,
        url: &str,
    ) -> Result<(T, Option<String>), ClientError>
    where
        T: serde::de::DeserializeOwned,
    {
        self.get_json_with_headers(url, &[]).await
    }

    /// Like [`get_json_with_link`](Self::get_json_with_link) with extra
    /// request headers.  A header replaces the default of the same name: pass
    /// `(API_VERSION_HEADER, "2026-03-10")` for an endpoint that needs another
    /// REST API version than [`API_VERSION`], or another `Accept` media type.
    pub(crate) async fn get_json_with_headers<T>(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<(T, Option<String>), ClientError>
    where
        T: serde::de::DeserializeOwned,
    {
        let (body, response_headers) = self.execute(&Call::get(url, headers)).await?;
        let link = response_headers
            .get(header::LINK)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        Ok((serde_json::from_slice(&body)?, link))
    }

    /// Performs a single POST request with a JSON body and deserialises the
    /// response.
    ///
    /// Rate limits and redirects are handled as for GETs; a 5xx response or
    /// a transport failure is not retried (the POST could act twice), and a
    /// `301`/`302`/`303` is not followed (it would turn the POST into a GET).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on network, TLS, API, or deserialisation errors.
    pub(crate) async fn post_json<T, B>(&self, url: &str, body: &B) -> Result<T, ClientError>
    where
        T: serde::de::DeserializeOwned,
        B: serde::Serialize,
    {
        let call = Call {
            method: Method::POST,
            body: Bytes::from(serde_json::to_vec(body)?),
            headers: &[("content-type", "application/json")],
            idempotent: false,
            ..Call::get(url, &[])
        };
        let (resp_bytes, _) = self.execute(&call).await?;
        Ok(serde_json::from_slice(&resp_bytes)?)
    }

    /// Sends `call` (see [`send`](Self::send)) and reads the whole, bounded
    /// response body.  A body cut off or stalled mid-stream is retried like a
    /// transport failure; for GraphQL, a `200` that reports a rate limit is
    /// waited out like a `403`/`429`.
    async fn execute(&self, call: &Call<'_>) -> Result<(Bytes, HeaderMap), ClientError> {
        let mut attempts = Attempts::default();
        loop {
            let response = self.send(call, &mut attempts).await?;
            let headers = response.headers().clone();
            let body = match collect_body_limited(response.into_body(), self.timing.request_timeout)
                .await
            {
                Ok(body) => body,
                Err(e) => {
                    self.retry_transport(call, &mut attempts, call.url, e)
                        .await?;
                    continue;
                }
            };
            if call.graphql {
                if let Some(limit) = rate_limit::classify_graphql(&headers, &body, unix_now()) {
                    self.hold_for_rate_limit(limit, &mut attempts, call.url, 200)?;
                    continue;
                }
            }
            return Ok((body, headers));
        }
    }

    /// Sends `call` until it gets a success response, whose body is left
    /// unread for the caller.
    ///
    /// - Redirects (`301`, `302`, `303`, `307`, `308`): up to
    ///   [`MAX_REDIRECTS`] are followed.  A GET follows all of them; any other
    ///   method only `307`/`308`, which keep the method and body.  A redirect
    ///   within the origin keeps the credential; one to another origin fails,
    ///   unless `call.leave_origin`, when it is followed without the
    ///   credential and the API headers.  HTTPS to HTTP is always refused.
    /// - Rate limits (see [`rate_limit::classify`]): a 429, or a 403 that
    ///   carries `Retry-After`, `X-RateLimit-Remaining: 0` or a "rate limit"
    ///   message, is waited out and retried: for `Retry-After`, else until
    ///   `X-RateLimit-Reset`, else (secondary limit) at least a minute,
    ///   doubling on every further hit.  The wait pauses every clone of the
    ///   client.  One wait may last [`MAX_RATE_WAIT_SECS`], all waits of one
    ///   request [`MAX_RATE_WAIT_TOTAL_SECS`]; beyond that the request fails
    ///   with [`ClientError::RateLimitExceeded`] - never with a plain 403,
    ///   which the backup treats as "no access, skip".
    /// - Idempotent requests only: 500/502/503/504 up to
    ///   [`MAX_SERVER_ERROR_RETRIES`] retries with exponential back-off and
    ///   jitter; connection failures, timeouts and bodies cut off mid-stream
    ///   up to [`MAX_TRANSPORT_RETRIES`] retries.
    /// - Any other status fails with [`ClientError::ApiError`].
    ///
    /// Error and redirect bodies are read with the size cap and the idle
    /// timeout.  The waits are plain `await`s, so dropping the future (as the
    /// engine does on cancellation) ends them at once.
    async fn send(
        &self,
        call: &Call<'_>,
        attempts: &mut Attempts,
    ) -> Result<Response<Incoming>, ClientError> {
        let mut url = call.url.to_string();
        let mut same_origin = true;
        let mut redirects = 0u8;

        loop {
            self.rate_gate.pass().await;
            let req = self.request(call, &url, same_origin)?;
            let sent =
                tokio::time::timeout(self.timing.request_timeout, self.http.request(req)).await;
            let response = match sent {
                Ok(Ok(response)) => response,
                Ok(Err(e)) => {
                    self.retry_transport(call, attempts, &url, e.into()).await?;
                    continue;
                }
                Err(_) => {
                    let e = ClientError::Timeout { url: url.clone() };
                    self.retry_transport(call, attempts, &url, e).await?;
                    continue;
                }
            };
            let status = response.status();
            if same_origin {
                self.note_deprecation(&call.method, &url, response.headers());
            }
            if status.is_success() {
                return Ok(response);
            }

            let headers = response.headers().clone();
            let body = match collect_body_limited(response.into_body(), self.timing.request_timeout)
                .await
            {
                Ok(body) => body,
                Err(e) => {
                    self.retry_transport(call, attempts, &url, e).await?;
                    continue;
                }
            };

            if matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
                url = redirect_target(
                    call,
                    &url,
                    status,
                    &headers,
                    &mut same_origin,
                    &mut redirects,
                )?;
                continue;
            }

            if let Some(limit) = rate_limit::classify(status.as_u16(), &headers, &body, unix_now())
            {
                self.hold_for_rate_limit(limit, attempts, &url, status.as_u16())?;
                continue;
            }

            if call.idempotent
                && matches!(status.as_u16(), 500 | 502 | 503 | 504)
                && attempts.server_errors < MAX_SERVER_ERROR_RETRIES
            {
                let backoff = backoff_with_jitter(attempts.server_errors);
                warn!(
                    url = %url,
                    status = status.as_u16(),
                    backoff_secs = backoff.as_secs(),
                    attempt = attempts.server_errors + 1,
                    "transient server error, retrying with jitter"
                );
                tokio::time::sleep(self.scaled(backoff)).await;
                attempts.server_errors += 1;
                continue;
            }

            return Err(ClientError::ApiError {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }
    }

    /// Builds the request for one hop of `call` to `url`.
    ///
    /// The credential, `Accept: application/vnd.github+json` and the API
    /// version go only to the origin of the call, as `call.origin` says; the
    /// headers of the call are sent on every hop and replace the defaults of
    /// the same name.  The
    /// `Authorization` header is omitted for [`Credential::Anonymous`] so that
    /// GitHub's unauthenticated rate-limit bucket applies.
    fn request(
        &self,
        call: &Call<'_>,
        url: &str,
        same_origin: bool,
    ) -> Result<Request<Full<Bytes>>, ClientError> {
        let mut req = Request::builder()
            .method(call.method.clone())
            .uri(url)
            .header(header::USER_AGENT, USER_AGENT)
            .body(Full::new(call.body.clone()))?;
        let headers = req.headers_mut();
        let auth = match (same_origin, call.origin) {
            (false, _) | (true, OriginHeaders::None) => None,
            (true, OriginHeaders::Api) => {
                headers.insert(header::ACCEPT, HeaderValue::from_static(ACCEPT_GITHUB_JSON));
                headers.insert(
                    HeaderName::from_static(API_VERSION_HEADER),
                    HeaderValue::from_static(API_VERSION),
                );
                self.credential.authorization_header()
            }
            (true, OriginHeaders::WebBasic) => self.credential.token().map(|token| {
                use base64::Engine as _;
                let pair = base64::engine::general_purpose::STANDARD
                    .encode(format!("{token}:x-oauth-basic"));
                format!("Basic {pair}")
            }),
        };
        if let Some(auth) = auth {
            let mut value =
                HeaderValue::from_str(&auth).map_err(|e| ClientError::Http(e.into()))?;
            value.set_sensitive(true);
            headers.insert(header::AUTHORIZATION, value);
        }
        for (name, value) in call.headers {
            headers.insert(
                HeaderName::from_bytes(name.as_bytes()).map_err(|e| ClientError::Http(e.into()))?,
                HeaderValue::from_str(value).map_err(|e| ClientError::Http(e.into()))?,
            );
        }
        Ok(req)
    }

    /// Sleeps before retrying a connection failure, a timeout or a body cut
    /// off mid-stream, or returns `error` when `call` may not be retried (not
    /// idempotent, another kind of error, or out of retries).
    async fn retry_transport(
        &self,
        call: &Call<'_>,
        attempts: &mut Attempts,
        url: &str,
        error: ClientError,
    ) -> Result<(), ClientError> {
        let transient = matches!(
            error,
            ClientError::Transport(_) | ClientError::Body(_) | ClientError::Timeout { .. }
        );
        if !transient || !call.idempotent || attempts.transport_failures >= MAX_TRANSPORT_RETRIES {
            return Err(error);
        }
        let backoff = backoff_with_jitter(attempts.transport_failures);
        warn!(
            url = %url,
            error = %error,
            backoff_secs = backoff.as_secs(),
            attempt = attempts.transport_failures + 1,
            "request failed, retrying"
        );
        tokio::time::sleep(self.scaled(backoff)).await;
        attempts.transport_failures += 1;
        Ok(())
    }

    /// Closes the shared [`RateGate`] for the wait `limit` asks for, or fails
    /// with [`ClientError::RateLimitExceeded`] when the wait is out of the
    /// request's budget.
    fn hold_for_rate_limit(
        &self,
        limit: RateLimitWait,
        attempts: &mut Attempts,
        url: &str,
        status: u16,
    ) -> Result<(), ClientError> {
        let (wait, jitter_span) = match limit {
            RateLimitWait::Explicit(secs) => (secs.max(1), 1.0),
            RateLimitWait::Unspecified => (
                SECONDARY_LIMIT_BASE_SECS << attempts.rate_hits.min(4),
                // Spread concurrent clients over a few seconds.
                5.0,
            ),
        };
        if attempts.rate_hits >= MAX_RATE_LIMIT_RETRIES
            || wait > MAX_RATE_WAIT_SECS
            || attempts.rate_waited.saturating_add(wait) > MAX_RATE_WAIT_TOTAL_SECS
        {
            return Err(ClientError::RateLimitExceeded {
                retry_after_secs: wait,
            });
        }
        warn!(
            url = %url,
            status,
            "rate limited; pausing all requests for {wait}s (attempt {})",
            attempts.rate_hits + 1
        );
        let jitter = Duration::from_millis((jitter_ms() as f64 * jitter_span) as u64);
        self.rate_gate
            .close_until(Instant::now() + self.scaled(Duration::from_secs(wait) + jitter));
        attempts.rate_waited += wait;
        attempts.rate_hits += 1;
        Ok(())
    }

    /// Logs, once per endpoint, that GitHub announced its deprecation
    /// (`Deprecation` / `Sunset` response headers, RFC 9745 / RFC 8594).
    fn note_deprecation(&self, method: &Method, url: &str, headers: &HeaderMap) {
        let value = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
        let (deprecation, sunset) = (value("deprecation"), value("sunset"));
        if deprecation.is_none() && sunset.is_none() {
            return;
        }
        let path = Url::parse(url)
            .map(|u| u.path().to_string())
            .unwrap_or_default();
        let endpoint = format!("{method} {}", endpoint_shape(&path));
        let first = self
            .deprecations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(endpoint.clone());
        if first {
            warn!(
                endpoint = %endpoint,
                deprecation = deprecation.unwrap_or("-"),
                sunset = sunset.unwrap_or("-"),
                "GitHub reports this API endpoint as deprecated; it may stop working \
                 after the sunset date"
            );
        }
    }

    /// Converts a nominal wait into real time (identity outside tests).
    fn scaled(&self, d: Duration) -> Duration {
        d.mul_f64(self.timing.unit.as_secs_f64())
    }
}

/// Where the redirect `status` from `url` leads, and whether the next hop
/// stays in the origin (see [`GitHubClient::send`] for the rules).
fn redirect_target(
    call: &Call<'_>,
    url: &str,
    status: StatusCode,
    headers: &HeaderMap,
    same_origin: &mut bool,
    redirects: &mut u8,
) -> Result<String, ClientError> {
    let refuse = |why: String| ClientError::ApiError {
        status: status.as_u16(),
        body: why,
    };
    // RFC 9110, section 15.4: a 301/302/303 may turn a POST into a GET; an
    // API call must not silently change its meaning.
    if call.method != Method::GET && !matches!(status.as_u16(), 307 | 308) {
        return Err(refuse(format!(
            "refusing to follow a {status} redirect of a {} request",
            call.method
        )));
    }
    if *redirects >= MAX_REDIRECTS {
        return Err(refuse("too many redirects".to_string()));
    }
    *redirects += 1;
    let location = headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| refuse("redirect with no Location header".to_string()))?;
    let current = Url::parse(url)?;
    let next = current.join(location)?;
    let from = current.host_str().unwrap_or("");
    let to = next.host_str().unwrap_or("");
    match redirect::classify(&current, &next) {
        Hop::SameOrigin => {}
        Hop::OtherOrigin if call.leave_origin => {
            debug!(
                from,
                to, "redirect leaves the origin: not sending the credential"
            );
            *same_origin = false;
        }
        Hop::OtherOrigin => {
            return Err(refuse(format!(
                "refusing to follow a redirect from {from} to another origin ({to})"
            )));
        }
        Hop::Refuse => {
            return Err(refuse(format!(
                "refusing to follow a redirect from https://{from} to a non-HTTPS URL"
            )));
        }
    }
    if status == StatusCode::MOVED_PERMANENTLY {
        debug!(
            from = current.path(),
            to = next.path(),
            "moved permanently (renamed or transferred?); following"
        );
    }
    Ok(next.into())
}

/// Resolves the next-page `link` of the page at `url`.
///
/// # Errors
///
/// [`ClientError::Pagination`] when the link leaves the origin (the
/// credential must not follow it) or leads back to a page in `fetched` (the
/// list would never end).
fn next_page(url: &str, link: &str, fetched: &HashSet<String>) -> Result<String, ClientError> {
    let current = Url::parse(url)?;
    let next = current.join(link)?;
    if redirect::classify(&current, &next) != Hop::SameOrigin {
        return Err(ClientError::Pagination(format!(
            "the next-page link of {url} leads to another origin ({}); not following it",
            next.origin().ascii_serialization()
        )));
    }
    let next = String::from(next);
    if fetched.contains(&next) {
        return Err(ClientError::Pagination(format!(
            "the next-page link of {url} leads back to {next}, a page already fetched"
        )));
    }
    Ok(next)
}

/// The path of an endpoint with the segments that name one resource (owner
/// and repository names after `repos`, `users`, `orgs`, `gists`, and
/// numbers) replaced by `{}`: a deprecation is logged once per endpoint, not
/// once per repository.
fn endpoint_shape(path: &str) -> String {
    let mut names_left = 0;
    path.split('/')
        .map(|segment| {
            if names_left > 0 {
                names_left -= 1;
                return "{}";
            }
            names_left = match segment {
                "repos" => 2,
                "users" | "orgs" | "gists" => 1,
                _ => 0,
            };
            if !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit()) {
                "{}"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Computes an exponential back-off with jitter.
///
/// The base delay is `2^attempt` seconds (1, 2, 4, 8, …) capped at
/// [`MAX_BACKOFF_SECS`].  Jitter adds 0–999 ms (see [`jitter_ms`]) — enough
/// variance to avoid a thundering herd from many concurrent workers retrying
/// in lock-step, without pulling in a randomness dependency.
fn backoff_with_jitter(attempt: u32) -> Duration {
    // Saturate the exponent at 16 (2^16 = 65 536 s ≫ MAX_BACKOFF_SECS).
    let exp = attempt.min(16);
    let base = 1u64.checked_shl(exp).unwrap_or(MAX_BACKOFF_SECS);
    let base = base.min(MAX_BACKOFF_SECS);
    Duration::from_secs(base) + Duration::from_millis(jitter_ms())
}

/// Returns a value in `[0, 1000)` ms suitable for jittering a back-off,
/// derived from the current high-resolution clock.
fn jitter_ms() -> u64 {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ (d.as_secs() << 12))
        .unwrap_or(0);
    jitter_from(seed)
}

/// Scrambles `seed` with one step of a small LCG — fast, non-cryptographic,
/// no extra dependency — into `[0, 1000)`.  The constants are the well-known
/// "Numerical Recipes" choices for a 32-bit LCG.
fn jitter_from(seed: u64) -> u64 {
    let mut x = seed
        .wrapping_add(1)
        .wrapping_mul(1_664_525)
        .wrapping_add(1_013_904_223);
    x ^= x.rotate_left(17);
    x % 1000
}

/// An API base URL must be `https://host[...]`: the token is sent with every
/// request, so a typo like `http://` must fail early instead of leaking it (the
/// transport would refuse it too, but only at the first request).
pub(crate) fn validate_api_url(raw: &str) -> Result<(), ClientError> {
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

/// Collects a hyper body into a [`Bytes`] buffer with a size cap.
///
/// Streams the body frame by frame and fails with
/// [`ClientError::BodyTooLarge`] once it exceeds [`MAX_RESPONSE_BYTES`], and
/// with [`ClientError::Timeout`] when no chunk arrives for `idle_timeout`.
/// Protects the process from OOM kills and hangs when a misbehaving proxy or
/// upstream returns an unbounded or stalled stream.
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
                return Err(ClientError::BodyTooLarge {
                    limit_bytes: MAX_RESPONSE_BYTES,
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
    fn jitter_spreads_neighbouring_clock_readings() {
        // Workers that fail together read nearly the same clock; their
        // jitter must still differ.  (The clock itself is not sampled here:
        // two rapid reads may be equal on a coarse clock.)
        let values: HashSet<u64> = (0..1000).map(jitter_from).collect();
        assert!(values.iter().all(|&v| v < 1000));
        assert!(
            values.len() > 500,
            "only {} distinct values for 1000 seeds",
            values.len()
        );
    }

    #[test]
    fn endpoint_shape_names_the_endpoint_not_the_resource() {
        assert_eq!(
            endpoint_shape("/repos/octo/cat/issues/42/comments"),
            "/repos/{}/{}/issues/{}/comments"
        );
        assert_eq!(
            endpoint_shape("/api/v3/orgs/acme/repos"),
            "/api/v3/orgs/{}/repos"
        );
        assert_eq!(endpoint_shape("/user/repos"), "/user/repos");
    }
}
