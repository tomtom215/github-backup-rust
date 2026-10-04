// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Async S3-compatible object store client.
//!
//! Built on hyper + rustls (no OpenSSL, no reqwest, no AWS SDK).  Implements
//! the S3 API surface needed for backup — `PutObject`, `HeadObject`,
//! `DeleteObject`, `ListObjectsV2` and multipart upload — against any
//! S3-compatible service:
//!
//! - AWS S3
//! - Backblaze B2 (S3-compatible API)
//! - MinIO (self-hosted)
//! - Cloudflare R2
//! - DigitalOcean Spaces
//! - Wasabi
//!
//! # Request construction
//!
//! Every request is described once (method, key, query parameters, headers,
//! body).  The signer signs exactly that description and the very same header
//! list is put on the wire, so *what is signed is what is sent*.  Object keys
//! are percent-encoded by one function ([`crate::encoding::encode_path`]) for
//! both the request line and the SigV4 canonical URI.
//!
//! # Retries and timeouts
//!
//! Server-side failures (`5xx`, `SlowDown`, `RequestTimeout`, `429`), dropped
//! connections and stalls are retried with exponential back-off and jitter, a
//! bounded number of times, re-signing every attempt.  A request is abandoned
//! only when **no bytes moved** for the idle timeout, so a slow link
//! uploading a large part is never cut off while it is making progress.
//!
//! # Proxies
//!
//! The client connects to the endpoint directly.  `HTTPS_PROXY` / `NO_PROXY`
//! are **not** honoured for S3 traffic (the GitHub client's proxy connector is
//! private to that crate); see the storage documentation.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::OsRng;
use bytes::Bytes;
use http_body_util::{BodyExt, Limited};
use hyper::body::{Body, Frame, Incoming, SizeHint};
use hyper::header::HeaderValue;
use hyper::{HeaderMap, Method, Request};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use tracing::{debug, info, warn};

use crate::config::{parse_endpoint, S3Config};
#[cfg(test)]
use crate::encoding::encode_path;
use crate::encoding::{percent_decode, request_query, xml_tag, xml_tags};
use crate::error::{error_chain, ApiError, S3Error};
use crate::signing::{sha256_hex, SignedRequest, Signer, SigningInput};

const USER_AGENT: &str = concat!("github-backup-rust/", env!("CARGO_PKG_VERSION"));

/// Default size of one multipart part (and the size above which an object is
/// uploaded with multipart).  S3 requires at least 5 MiB per part except the
/// last, and allows 10 000 parts.
pub const DEFAULT_PART_SIZE: usize = 16 * 1024 * 1024;

/// Largest response body that is read into memory (list pages are the
/// biggest legitimate responses, about 1.5 MB).
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

/// Request bodies are handed to hyper in frames of this size so that progress
/// is observable.
const BODY_FRAME_BYTES: usize = 64 * 1024;

/// Longest `Retry-After` the client will honour.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

type HyperClient = Client<hyper_rustls::HttpsConnector<HttpConnector>, ProgressBody>;

// ── Options ─────────────────────────────────────────────────────────────────

/// Retry behaviour for transient failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Total attempts per request, first try included (1 = no retries).
    pub max_attempts: u32,
    /// Back-off before the first retry; doubles each time.
    pub base_delay: Duration,
    /// Upper bound for a single back-off.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(20),
        }
    }
}

impl RetryPolicy {
    /// A policy that never retries.
    #[must_use]
    pub fn none() -> Self {
        Self {
            max_attempts: 1,
            ..Self::default()
        }
    }

    /// Back-off before retry number `attempt` (1-based): exponential with
    /// "equal jitter" — half fixed, half random — so concurrent uploads do
    /// not retry in lock-step.
    #[must_use]
    pub fn delay_for(&self, attempt: u32) -> Duration {
        let exponent = attempt.saturating_sub(1).min(20);
        let ceiling = self
            .base_delay
            .saturating_mul(1u32 << exponent)
            .min(self.max_delay);
        let half = ceiling / 2;
        let half_ms = u64::try_from(half.as_millis()).unwrap_or(u64::MAX);
        let jitter = if half_ms == 0 {
            Duration::ZERO
        } else {
            Duration::from_millis(OsRng.next_u64() % (half_ms + 1))
        };
        half + jitter
    }
}

/// Tunables of an [`S3Client`].  The defaults suit production; tests shrink
/// them.
#[derive(Debug, Clone)]
pub struct ClientOptions {
    /// Retry behaviour.
    pub retry: RetryPolicy,
    /// A request that moves no data for this long is abandoned (and retried).
    pub idle_timeout: Duration,
    /// Maximum time to establish a TCP connection.
    pub connect_timeout: Duration,
    /// Multipart part size; objects larger than this are uploaded in parts.
    pub part_size: usize,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            retry: RetryPolicy::default(),
            idle_timeout: Duration::from_secs(60),
            connect_timeout: Duration::from_secs(15),
            part_size: DEFAULT_PART_SIZE,
        }
    }
}

// ── Public result types ─────────────────────────────────────────────────────

/// What `HeadObject` reports about an existing object.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectInfo {
    /// `Content-Length` in bytes, when the server sent one.
    pub size: Option<u64>,
    /// The `ETag`, as sent.
    pub etag: Option<String>,
    /// User metadata (`x-amz-meta-*`), keyed by the lower-case name without
    /// the prefix.
    pub metadata: HashMap<String, String>,
}

/// The result of a `HeadObject` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadOutcome {
    /// The object exists.
    Found(ObjectInfo),
    /// The server says the key does not exist (`404`).
    Missing,
    /// The server answered `403`.  A `HEAD` response has no body, so this
    /// cannot be told apart from "the key does not exist and the credentials
    /// lack `s3:ListBucket`" (AWS answers `403` instead of `404` then).
    Forbidden,
}

/// An in-progress multipart upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipartUpload {
    /// The object key being written.
    pub key: String,
    /// The server-assigned upload id.
    pub upload_id: String,
}

// ── Transport internals ─────────────────────────────────────────────────────

/// Tracks when a request last moved data, for the idle timeout.
#[derive(Debug)]
struct Progress {
    start: Instant,
    last_ms: AtomicU64,
}

impl Progress {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            last_ms: AtomicU64::new(0),
        }
    }

    fn touch(&self) {
        let ms = u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.last_ms.store(ms, Ordering::Relaxed);
    }

    fn idle_for(&self) -> Duration {
        self.start
            .elapsed()
            .saturating_sub(Duration::from_millis(self.last_ms.load(Ordering::Relaxed)))
    }
}

/// A request body that hands its bytes to hyper in small frames and records
/// each frame as progress.
#[derive(Debug)]
struct ProgressBody {
    data: Bytes,
    pos: usize,
    progress: Arc<Progress>,
}

impl Body for ProgressBody {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        if this.pos >= this.data.len() {
            return Poll::Ready(None);
        }
        let end = (this.pos + BODY_FRAME_BYTES).min(this.data.len());
        let chunk = this.data.slice(this.pos..end);
        this.pos = end;
        this.progress.touch();
        Poll::Ready(Some(Ok(Frame::data(chunk))))
    }

    fn is_end_stream(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact((self.data.len() - self.pos) as u64)
    }
}

/// Polls `fut` until it finishes, giving up once `progress` has been idle for
/// `idle`.  Returns `None` on timeout.
async fn with_idle_timeout<F: Future>(
    fut: F,
    progress: &Progress,
    idle: Duration,
) -> Option<F::Output> {
    tokio::pin!(fut);
    loop {
        let remaining = idle.saturating_sub(progress.idle_for());
        if remaining.is_zero() {
            return None;
        }
        tokio::select! {
            out = &mut fut => return Some(out),
            () = tokio::time::sleep(remaining) => {}
        }
    }
}

/// A complete response.
struct RawResponse {
    status: u16,
    headers: HeaderMap,
    body: Bytes,
}

impl RawResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// The error this response represents, if it is one.  With
    /// `error_in_ok_body`, a `2xx` response whose body is an `<Error>`
    /// document counts too (`CompleteMultipartUpload` can do that).
    fn api_error(&self, operation: &'static str, error_in_ok_body: bool) -> Option<ApiError> {
        let is_success = (200..300).contains(&self.status);
        if is_success && !(error_in_ok_body && body_is_error_document(&self.body)) {
            return None;
        }
        let retry_after = self
            .header("retry-after")
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|secs| Duration::from_secs(secs).min(MAX_RETRY_AFTER));
        Some(ApiError::from_response(
            operation,
            self.status,
            &self.body,
            self.header("x-amz-bucket-region").map(str::to_string),
            retry_after,
        ))
    }
}

fn body_is_error_document(body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(body);
    text.contains("<Error>") && xml_tag(&text, "Code").is_some()
}

/// A request to send, before signing.
struct RequestSpec {
    operation: &'static str,
    method: Method,
    key: Option<String>,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    body: Bytes,
    payload_sha256: String,
    error_in_ok_body: bool,
}

impl RequestSpec {
    fn new(operation: &'static str, method: Method) -> Self {
        Self {
            operation,
            method,
            key: None,
            query: Vec::new(),
            headers: Vec::new(),
            body: Bytes::new(),
            payload_sha256: sha256_hex(b""),
            error_in_ok_body: false,
        }
    }

    fn key(mut self, key: &str) -> Self {
        self.key = Some(key.to_string());
        self
    }

    fn query(mut self, name: &str, value: impl Into<String>) -> Self {
        self.query.push((name.to_string(), value.into()));
        self
    }

    fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_ascii_lowercase(), value.into()));
        self
    }

    fn body(mut self, body: Bytes) -> Self {
        self.payload_sha256 = sha256_hex(&body);
        self.body = body;
        self
    }

    fn user_metadata(mut self, metadata: &[(&str, &str)]) -> Result<Self, S3Error> {
        for (name, value) in metadata {
            let valid_name = !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            let valid_value = !value.is_empty()
                && value.bytes().all(|b| (0x20..0x7f).contains(&b))
                && value.trim() == *value;
            if !valid_name || !valid_value {
                return Err(S3Error::InvalidKey(format!(
                    "metadata `{name}` must be a lower-case name with a printable-ASCII value"
                )));
            }
            self.headers
                .push((format!("x-amz-meta-{name}"), (*value).to_string()));
        }
        Ok(self)
    }

    fn error_in_ok_body(mut self) -> Self {
        self.error_in_ok_body = true;
        self
    }
}

/// How the bucket is addressed.
#[derive(Debug, Clone)]
struct Target {
    https: bool,
    /// `host[:port]` as it appears in the URL.
    authority: String,
    /// Value of the `Host` header (default port removed).
    host_header: String,
    /// `true`: `/{bucket}/{key}`; `false`: the bucket is in the host name.
    path_style: bool,
    bucket: String,
}

impl Target {
    fn new(config: &S3Config) -> Result<Self, S3Error> {
        match &config.endpoint {
            Some(raw) => {
                let endpoint = parse_endpoint(raw)?;
                if endpoint.assumed_https {
                    warn!(
                        endpoint = %raw,
                        "the S3 endpoint has no scheme; assuming https:// (write the scheme out to silence this)"
                    );
                }
                if !endpoint.https && !is_loopback(&endpoint.authority) {
                    warn!(
                        endpoint = %raw,
                        "the S3 endpoint uses plain http://: backup data and object names travel unencrypted"
                    );
                }
                Ok(Self {
                    https: endpoint.https,
                    authority: endpoint.authority,
                    host_header: endpoint.host_header,
                    path_style: true,
                    bucket: config.bucket.clone(),
                })
            }
            None => {
                // Virtual-hosted style for AWS.  A dot in the bucket name
                // breaks the wildcard TLS certificate, so such buckets use
                // path-style addressing on the regional endpoint.
                let dotted = config.bucket.contains('.');
                let authority = if dotted {
                    format!("s3.{}.amazonaws.com", config.region)
                } else {
                    format!("{}.s3.{}.amazonaws.com", config.bucket, config.region)
                };
                Ok(Self {
                    https: true,
                    host_header: authority.clone(),
                    authority,
                    path_style: dotted,
                    bucket: config.bucket.clone(),
                })
            }
        }
    }

    /// The raw (unencoded) request path for `key` (`None` = the bucket).
    fn raw_path(&self, key: Option<&str>) -> String {
        let key = key.map(|k| k.trim_start_matches('/'));
        match (self.path_style, key) {
            (true, Some(k)) => format!("/{}/{k}", self.bucket),
            (true, None) => format!("/{}", self.bucket),
            (false, Some(k)) => format!("/{k}"),
            (false, None) => "/".to_string(),
        }
    }

    fn scheme(&self) -> &'static str {
        if self.https {
            "https"
        } else {
            "http"
        }
    }
}

fn is_loopback(authority: &str) -> bool {
    let host = authority
        .rsplit_once(':')
        .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
        .map_or(authority, |(host, _)| host);
    host == "localhost" || host == "[::1]" || host.starts_with("127.")
}

struct Inner {
    http: HyperClient,
    /// Configuration with the credentials removed.
    config: S3Config,
    target: Target,
    signer: Signer,
    options: ClientOptions,
}

/// Async S3-compatible object store client.
///
/// Construct via [`S3Client::new`]. Cloning is cheap: clones share the
/// connection pool and the (single) copy of the credentials.
#[derive(Clone)]
pub struct S3Client {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for S3Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Client")
            .field("bucket", &self.inner.config.bucket)
            .field("region", &self.inner.config.region)
            .field("endpoint", &self.inner.config.endpoint)
            .field("credentials", &"[redacted]")
            .finish()
    }
}

impl S3Client {
    /// Creates a new [`S3Client`] using the system CA bundle for TLS and the
    /// default [`ClientOptions`].
    ///
    /// # Errors
    ///
    /// Returns [`S3Error::InvalidConfig`] / [`S3Error::InvalidEndpoint`] for
    /// unusable settings and [`S3Error::Tls`] if the native CA bundle cannot
    /// be loaded (not required for a plain-`http://` endpoint).
    pub fn new(config: S3Config) -> Result<Self, S3Error> {
        Self::with_options(config, ClientOptions::default())
    }

    /// Like [`S3Client::new`] with explicit [`ClientOptions`].
    ///
    /// # Errors
    ///
    /// See [`S3Client::new`].
    pub fn with_options(mut config: S3Config, options: ClientOptions) -> Result<Self, S3Error> {
        config.validate()?;
        let target = Target::new(&config)?;
        let http = build_http_client(&options, target.https)?;
        // The signer owns the only long-lived copy of the credentials; the
        // config kept for addressing carries none.
        let signer = Signer::new_s3(
            config.access_key_id.clone(),
            std::mem::take(&mut config.secret_access_key),
            config.region.clone(),
        )
        .with_session_token(config.session_token.take());
        Ok(Self {
            inner: Arc::new(Inner {
                http,
                config,
                target,
                signer,
                options,
            }),
        })
    }

    /// The options this client was built with.
    #[must_use]
    pub fn options(&self) -> &ClientOptions {
        &self.inner.options
    }

    /// The bucket this client talks to.
    #[must_use]
    pub fn bucket(&self) -> &str {
        &self.inner.config.bucket
    }

    // ── Operations ──────────────────────────────────────────────────────

    /// Uploads `data` to the object at `key` in the configured bucket.
    ///
    /// `metadata` entries become `x-amz-meta-<name>` headers (lower-case
    /// names, printable-ASCII values).
    ///
    /// # Errors
    ///
    /// Returns [`S3Error`] on network, auth, or service errors (after the
    /// retry policy is exhausted).
    pub async fn put_object(
        &self,
        key: &str,
        data: impl Into<Bytes>,
        content_type: &str,
        metadata: &[(&str, &str)],
    ) -> Result<(), S3Error> {
        validate_key(key)?;
        debug!(bucket = %self.bucket(), key, "S3 PutObject");
        let spec = RequestSpec::new("PutObject", Method::PUT)
            .key(key)
            .header("content-type", content_type)
            .user_metadata(metadata)?
            .body(data.into());
        self.expect_success(&spec).await.map(drop)
    }

    /// Looks the object up with `HeadObject`.
    ///
    /// # Errors
    ///
    /// Returns [`S3Error`] on network errors and on statuses other than
    /// `200`, `403` and `404` (for example `301` for a wrong region).
    pub async fn head_object(&self, key: &str) -> Result<HeadOutcome, S3Error> {
        validate_key(key)?;
        let spec = RequestSpec::new("HeadObject", Method::HEAD).key(key);
        let response = self.execute(&spec).await?;
        match response.status {
            200..=299 => {
                let mut info = ObjectInfo {
                    size: response
                        .header("content-length")
                        .and_then(|v| v.trim().parse::<u64>().ok()),
                    etag: response.header("etag").map(str::to_string),
                    metadata: HashMap::new(),
                };
                for (name, value) in &response.headers {
                    if let (Some(rest), Ok(value)) =
                        (name.as_str().strip_prefix("x-amz-meta-"), value.to_str())
                    {
                        info.metadata.insert(rest.to_string(), value.to_string());
                    }
                }
                Ok(HeadOutcome::Found(info))
            }
            404 => Ok(HeadOutcome::Missing),
            403 => Ok(HeadOutcome::Forbidden),
            _ => Err(response
                .api_error(spec.operation, false)
                .map(S3Error::api)
                .unwrap_or_else(|| {
                    S3Error::InvalidResponse(format!("HEAD status {}", response.status))
                })),
        }
    }

    /// Lists every object key that begins with `prefix`, following
    /// pagination.  Keys come back decoded: the request asks for
    /// `encoding-type=url` and the XML entities are resolved.
    ///
    /// # Errors
    ///
    /// Returns [`S3Error`] on network, auth, service or response-shape errors.
    pub async fn list_objects(&self, prefix: &str) -> Result<Vec<String>, S3Error> {
        let mut keys = Vec::new();
        let mut continuation: Option<String> = None;
        loop {
            let mut spec = RequestSpec::new("ListObjectsV2", Method::GET)
                .query("list-type", "2")
                .query("encoding-type", "url")
                .query("max-keys", "1000")
                .query("prefix", prefix);
            if let Some(token) = &continuation {
                spec = spec.query("continuation-token", token.clone());
            }
            let response = self.expect_success(&spec).await?;
            let body = String::from_utf8_lossy(&response.body);
            let url_encoded = xml_tag(&body, "EncodingType").as_deref() == Some("url");
            keys.extend(xml_tags(&body, "Key").into_iter().map(|raw| {
                if url_encoded {
                    percent_decode(&raw, true)
                } else {
                    raw
                }
            }));
            if xml_tag(&body, "IsTruncated").as_deref() != Some("true") {
                return Ok(keys);
            }
            match xml_tag(&body, "NextContinuationToken") {
                Some(token) if continuation.as_deref() != Some(token.as_str()) => {
                    continuation = Some(token);
                }
                _ => {
                    return Err(S3Error::InvalidResponse(
                        "a truncated ListObjectsV2 page carried no new continuation token"
                            .to_string(),
                    ))
                }
            }
        }
    }

    /// Deletes the object at `key` (succeeds when it is already gone).
    ///
    /// # Errors
    ///
    /// Returns [`S3Error`] on network, auth, or service errors.
    pub async fn delete_object(&self, key: &str) -> Result<(), S3Error> {
        validate_key(key)?;
        let spec = RequestSpec::new("DeleteObject", Method::DELETE).key(key);
        self.expect_success(&spec).await.map(drop)
    }

    /// Uploads `data` using S3 Multipart Upload in [`ClientOptions::part_size`]
    /// parts.
    ///
    /// If any step fails — a part after its retries, or the final
    /// `CompleteMultipartUpload`, including a `200 OK` that carries an
    /// `<Error>` — the upload is aborted so no orphaned parts keep accruing
    /// storage charges.
    ///
    /// # Errors
    ///
    /// Returns [`S3Error`] on network, auth, or service errors.
    pub async fn multipart_upload(
        &self,
        key: &str,
        data: &[u8],
        content_type: &str,
        metadata: &[(&str, &str)],
    ) -> Result<(), S3Error> {
        let part_size = self.inner.options.part_size.max(1);
        let upload = self
            .create_multipart_upload(key, content_type, metadata)
            .await?;
        info!(
            key,
            upload_id = %upload.upload_id,
            parts = data.len().div_ceil(part_size).max(1),
            "starting multipart upload"
        );
        let result = async {
            let mut parts: Vec<(u32, String)> = Vec::new();
            // An empty object still needs one (empty) part.
            let mut chunks: Vec<&[u8]> = data.chunks(part_size).collect();
            if chunks.is_empty() {
                chunks.push(&[]);
            }
            for (index, chunk) in chunks.into_iter().enumerate() {
                let number = u32::try_from(index + 1)
                    .map_err(|_| S3Error::InvalidKey("too many multipart parts".to_string()))?;
                let etag = self
                    .upload_part(&upload, number, Bytes::copy_from_slice(chunk))
                    .await?;
                parts.push((number, etag));
            }
            self.complete_multipart_upload(&upload, &parts).await
        }
        .await;
        if let Err(error) = &result {
            warn!(key, upload_id = %upload.upload_id, error = %error, "multipart upload failed; aborting it");
            self.abort_multipart_upload_best_effort(&upload).await;
        }
        result
    }

    /// Starts a multipart upload (`POST ?uploads`).
    ///
    /// # Errors
    ///
    /// Returns [`S3Error`] on network, auth, or service errors.
    pub async fn create_multipart_upload(
        &self,
        key: &str,
        content_type: &str,
        metadata: &[(&str, &str)],
    ) -> Result<MultipartUpload, S3Error> {
        validate_key(key)?;
        let spec = RequestSpec::new("CreateMultipartUpload", Method::POST)
            .key(key)
            .query("uploads", "")
            .header("content-type", content_type)
            .user_metadata(metadata)?;
        let response = self.expect_success(&spec).await?;
        let body = String::from_utf8_lossy(&response.body);
        let upload_id = xml_tag(&body, "UploadId")
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                S3Error::InvalidResponse(
                    "CreateMultipartUpload response carried no UploadId".to_string(),
                )
            })?;
        Ok(MultipartUpload {
            key: key.to_string(),
            upload_id,
        })
    }

    /// Uploads one part and returns its `ETag` (as the server sent it).
    ///
    /// # Errors
    ///
    /// Returns [`S3Error`] on network, auth, or service errors.
    pub async fn upload_part(
        &self,
        upload: &MultipartUpload,
        part_number: u32,
        data: impl Into<Bytes>,
    ) -> Result<String, S3Error> {
        let data: Bytes = data.into();
        debug!(key = %upload.key, part_number, bytes = data.len(), "uploading multipart part");
        let spec = RequestSpec::new("UploadPart", Method::PUT)
            .key(&upload.key)
            .query("partNumber", part_number.to_string())
            .query("uploadId", upload.upload_id.clone())
            .body(data);
        let response = self.expect_success(&spec).await?;
        response
            .header("etag")
            .map(|etag| etag.trim().to_string())
            .filter(|etag| !etag.is_empty())
            .ok_or_else(|| {
                S3Error::InvalidResponse("UploadPart response carried no ETag header".to_string())
            })
    }

    /// Completes a multipart upload.  A `200 OK` whose body is an `<Error>`
    /// document is a failure (and is retried when the code says it may help).
    ///
    /// # Errors
    ///
    /// Returns [`S3Error`] on network, auth, or service errors.
    pub async fn complete_multipart_upload(
        &self,
        upload: &MultipartUpload,
        parts: &[(u32, String)],
    ) -> Result<(), S3Error> {
        let parts_xml: String = parts
            .iter()
            .map(|(number, etag)| {
                format!(
                    "<Part><PartNumber>{number}</PartNumber><ETag>{}</ETag></Part>",
                    // Quotes need no escaping in element content; S3 ETags
                    // never contain `&` or `<`.
                    etag.replace('&', "&amp;").replace('<', "&lt;")
                )
            })
            .collect();
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><CompleteMultipartUpload>{parts_xml}</CompleteMultipartUpload>"
        );
        let spec = RequestSpec::new("CompleteMultipartUpload", Method::POST)
            .key(&upload.key)
            .query("uploadId", upload.upload_id.clone())
            .header("content-type", "application/xml")
            .body(Bytes::from(body))
            .error_in_ok_body();
        self.expect_success(&spec).await?;
        info!(key = %upload.key, "multipart upload completed");
        Ok(())
    }

    /// Aborts a multipart upload (`DELETE ?uploadId=…`).
    ///
    /// # Errors
    ///
    /// Returns [`S3Error`] on network, auth, or service errors.
    pub async fn abort_multipart_upload(&self, upload: &MultipartUpload) -> Result<(), S3Error> {
        let spec = RequestSpec::new("AbortMultipartUpload", Method::DELETE)
            .key(&upload.key)
            .query("uploadId", upload.upload_id.clone());
        self.expect_success(&spec).await.map(drop)
    }

    /// Aborts `upload`, logging (not returning) a failure so the original
    /// error stays the one the caller sees.
    pub async fn abort_multipart_upload_best_effort(&self, upload: &MultipartUpload) {
        if let Err(error) = self.abort_multipart_upload(upload).await {
            warn!(
                key = %upload.key,
                upload_id = %upload.upload_id,
                error = %error,
                "could not abort the multipart upload; the uploaded parts stay in the bucket \
                 until it is aborted — add a lifecycle rule `AbortIncompleteMultipartUpload` \
                 to the bucket to have the provider clean them up"
            );
        }
    }

    // ── Addressing helpers ──────────────────────────────────────────────

    /// The full URL of `key`.
    #[cfg(test)]
    fn object_url(&self, key: &str) -> String {
        let target = &self.inner.target;
        format!(
            "{}://{}{}",
            target.scheme(),
            target.authority,
            encode_path(&target.raw_path(Some(key)))
        )
    }

    /// The URL of the bucket root (no trailing slash).
    #[cfg(test)]
    fn bucket_base_url(&self) -> String {
        let target = &self.inner.target;
        if target.path_style {
            format!(
                "{}://{}/{}",
                target.scheme(),
                target.authority,
                target.bucket
            )
        } else {
            format!("{}://{}", target.scheme(), target.authority)
        }
    }

    /// The raw path of the bucket root (as signed).
    #[cfg(test)]
    fn bucket_base_path(&self) -> String {
        self.inner.target.raw_path(None)
    }

    /// The `Host` header value.
    #[cfg(test)]
    fn host(&self) -> String {
        self.inner.target.host_header.clone()
    }

    /// The raw (unencoded) request path of `key`.
    #[cfg(test)]
    fn object_path(&self, key: &str) -> String {
        self.inner.target.raw_path(Some(key))
    }

    // ── Request execution ───────────────────────────────────────────────

    /// Executes `spec` and returns the response when it is a success;
    /// otherwise the API error.
    async fn expect_success(&self, spec: &RequestSpec) -> Result<RawResponse, S3Error> {
        let response = self.execute(spec).await?;
        match response.api_error(spec.operation, spec.error_in_ok_body) {
            None => Ok(response),
            Some(error) => Err(S3Error::api(error)),
        }
    }

    /// Sends `spec`, retrying transient failures.  Returns the final response
    /// whatever its status (the caller interprets it) or the final transport
    /// error.
    async fn execute(&self, spec: &RequestSpec) -> Result<RawResponse, S3Error> {
        let policy = self.inner.options.retry;
        let mut attempt: u32 = 1;
        loop {
            let result = self.send_once(spec).await;
            let (retryable, server_delay, reason) = match &result {
                Ok(response) => match response.api_error(spec.operation, spec.error_in_ok_body) {
                    Some(error) => (
                        error.is_retryable(),
                        error.retry_after.unwrap_or_default(),
                        error.to_string(),
                    ),
                    None => return result,
                },
                Err(error) => (error.is_retryable(), Duration::ZERO, error.to_string()),
            };
            if !retryable || attempt >= policy.max_attempts {
                return result;
            }
            let delay = policy.delay_for(attempt).max(server_delay);
            warn!(
                operation = spec.operation,
                attempt,
                max_attempts = policy.max_attempts,
                retry_in_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                reason = %reason,
                "S3 request failed; retrying"
            );
            tokio::time::sleep(delay).await;
            attempt += 1;
        }
    }

    /// Builds, signs and sends one attempt of `spec`.
    async fn send_once(&self, spec: &RequestSpec) -> Result<RawResponse, S3Error> {
        let progress = Arc::new(Progress::new());
        let (request, url) = self.build_request(spec, Arc::clone(&progress))?;
        let idle = self.inner.options.idle_timeout;
        let timeout = || S3Error::Timeout {
            url: url.clone(),
            idle_secs: idle.as_secs().max(1),
        };
        let response = with_idle_timeout(self.inner.http.request(request), &progress, idle)
            .await
            .ok_or_else(timeout)??;
        let (parts, body) = response.into_parts();
        let body = read_body(body, idle).await.ok_or_else(timeout)??;
        Ok(RawResponse {
            status: parts.status.as_u16(),
            headers: parts.headers,
            body,
        })
    }

    /// Signs `spec` and turns it into an HTTP request.  Every signed header is
    /// set from the same list passed to the signer.
    fn build_request(
        &self,
        spec: &RequestSpec,
        progress: Arc<Progress>,
    ) -> Result<(Request<ProgressBody>, String), S3Error> {
        let inner = &self.inner;
        let raw_path = inner.target.raw_path(spec.key.as_deref());
        let query: Vec<(&str, &str)> = spec
            .query
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect();
        let mut headers: Vec<(&str, &str)> = vec![("host", inner.target.host_header.as_str())];
        headers.extend(spec.headers.iter().map(|(n, v)| (n.as_str(), v.as_str())));

        let signed: SignedRequest = inner.signer.sign(&SigningInput {
            method: spec.method.as_str(),
            path: &raw_path,
            query: &query,
            headers: &headers,
            payload_sha256: &spec.payload_sha256,
        });

        // The request line uses the canonical path that was signed.  The
        // query is the same sorted, encoded parameter list; a value-less
        // sub-resource (`?uploads`) is sent in the bare form SDKs use and
        // signed as `uploads=`.
        let mut url = format!(
            "{}://{}{}",
            inner.target.scheme(),
            inner.target.authority,
            signed.canonical_uri
        );
        let request_query = request_query(&query);
        if !request_query.is_empty() {
            url.push('?');
            url.push_str(&request_query);
        }

        let mut builder = Request::builder().method(spec.method.clone()).uri(&url);
        for (name, value) in &headers {
            builder = builder.header(*name, *value);
        }
        builder = builder
            .header("x-amz-date", &signed.amz_date)
            .header("x-amz-content-sha256", &signed.content_sha256);
        if let Some(token) = &signed.security_token {
            let mut value = HeaderValue::from_str(token).map_err(|_| {
                S3Error::InvalidConfig(
                    "the session token contains characters that cannot be sent in an HTTP header"
                        .to_string(),
                )
            })?;
            value.set_sensitive(true);
            builder = builder.header("x-amz-security-token", value);
        }
        let mut authorization = HeaderValue::from_str(&signed.authorization).map_err(|_| {
            S3Error::InvalidConfig(
                "the Authorization header is not a valid header value".to_string(),
            )
        })?;
        authorization.set_sensitive(true);
        builder = builder
            .header("authorization", authorization)
            .header("user-agent", USER_AGENT);
        if !spec.body.is_empty() || matches!(spec.method, Method::PUT | Method::POST) {
            builder = builder.header("content-length", spec.body.len().to_string());
        }
        let request = builder.body(ProgressBody {
            data: spec.body.clone(),
            pos: 0,
            progress,
        })?;
        Ok((request, url))
    }
}

/// Reads a response body (bounded in size and by the idle timeout).
async fn read_body(body: Incoming, idle: Duration) -> Option<Result<Bytes, S3Error>> {
    let collected =
        tokio::time::timeout(idle, Limited::new(body, MAX_RESPONSE_BYTES).collect()).await;
    match collected {
        Err(_) => None,
        Ok(Ok(collected)) => Some(Ok(collected.to_bytes())),
        Ok(Err(error)) => Some(Err(S3Error::Transport {
            message: error_chain(&*error),
            connect: false,
        })),
    }
}

/// Rejects keys S3 would refuse before any request is made.
fn validate_key(key: &str) -> Result<(), S3Error> {
    if key.is_empty() {
        return Err(S3Error::InvalidKey("the object key is empty".to_string()));
    }
    if key.len() > 1024 {
        return Err(S3Error::InvalidKey(format!(
            "the object key is {} bytes long; S3 allows at most 1024 ({key:.60}…)",
            key.len()
        )));
    }
    Ok(())
}

/// Builds an HTTPS client using the system native CA bundle.
fn build_http_client(options: &ClientOptions, needs_tls: bool) -> Result<HyperClient, S3Error> {
    let mut root_store = rustls::RootCertStore::empty();
    let cert_result = rustls_native_certs::load_native_certs();
    if cert_result.certs.is_empty() && needs_tls {
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

    let mut http = HttpConnector::new();
    // The TLS layer above decides between http and https.
    http.enforce_http(false);
    http.set_connect_timeout(Some(options.connect_timeout));
    http.set_nodelay(true);
    http.set_keepalive(Some(Duration::from_secs(30)));

    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_or_http()
        .enable_http1()
        .wrap_connector(http);

    Ok(Client::builder(TokioExecutor::new())
        // Servers drop idle connections after a short while; do not reuse
        // ones that have been idle for long.
        .pool_idle_timeout(Duration::from_secs(15))
        .build(https))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::S3Config;
    use crate::encoding::encode_component;

    fn sample_config() -> S3Config {
        S3Config {
            bucket: "my-bucket".to_string(),
            region: "us-east-1".to_string(),
            prefix: "backups/".to_string(),
            endpoint: None,
            access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
            session_token: None,
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
    fn object_url_percent_encodes_the_key() {
        let client = S3Client::new(sample_config_custom_endpoint()).unwrap();
        assert_eq!(
            client.object_url("a b/what?/f#1/100%/r\u{e9}sum\u{e9}+x&y"),
            "https://s3.us-west-004.backblazeb2.com/my-bucket/a%20b/what%3F/f%231/100%25/r%C3%A9sum%C3%A9%2Bx%26y"
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
    fn host_keeps_a_non_default_port() {
        let mut cfg = sample_config();
        cfg.endpoint = Some("http://127.0.0.1:9000/".to_string());
        let client = S3Client::new(cfg).unwrap();
        assert_eq!(client.host(), "127.0.0.1:9000");
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
    fn dotted_aws_bucket_uses_path_style_on_the_regional_endpoint() {
        let mut cfg = sample_config();
        cfg.bucket = "my.dotted.bucket".to_string();
        let client = S3Client::new(cfg).unwrap();
        assert_eq!(client.host(), "s3.us-east-1.amazonaws.com");
        assert_eq!(
            client.object_url("k.json"),
            "https://s3.us-east-1.amazonaws.com/my.dotted.bucket/k.json"
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

    #[test]
    fn config_kept_by_the_client_carries_no_secret() {
        let mut cfg = sample_config();
        cfg.session_token = Some("SESSION".to_string());
        let client = S3Client::new(cfg).unwrap();
        assert!(client.inner.config.secret_access_key.is_empty());
        assert!(client.inner.config.session_token.is_none());
    }

    #[test]
    fn bucket_base_url_virtual_hosted() {
        let client = S3Client::new(sample_config()).unwrap();
        assert_eq!(
            client.bucket_base_url(),
            "https://my-bucket.s3.us-east-1.amazonaws.com"
        );
    }

    #[test]
    fn bucket_base_url_custom_endpoint() {
        let client = S3Client::new(sample_config_custom_endpoint()).unwrap();
        assert_eq!(
            client.bucket_base_url(),
            "https://s3.us-west-004.backblazeb2.com/my-bucket"
        );
    }

    #[test]
    fn bucket_base_path_virtual_hosted() {
        let client = S3Client::new(sample_config()).unwrap();
        assert_eq!(client.bucket_base_path(), "/");
    }

    #[test]
    fn bucket_base_path_custom_endpoint() {
        let client = S3Client::new(sample_config_custom_endpoint()).unwrap();
        assert_eq!(client.bucket_base_path(), "/my-bucket");
    }

    #[test]
    fn percent_encode_unreserved_unchanged() {
        assert_eq!(
            encode_component("backups/owner/json"),
            "backups%2Fowner%2Fjson"
        );
        assert_eq!(encode_component("abc-_~."), "abc-_~.");
    }

    #[test]
    fn percent_encode_special_chars() {
        assert_eq!(encode_component("a+b=c"), "a%2Bb%3Dc");
        assert_eq!(encode_component("a b"), "a%20b");
    }

    #[test]
    fn extract_all_xml_tags_finds_multiple() {
        let xml = "<r><Key>a/b.json</Key><Key>c/d.json</Key></r>";
        let keys = xml_tags(xml, "Key");
        assert_eq!(keys, vec!["a/b.json", "c/d.json"]);
    }

    #[test]
    fn extract_all_xml_tags_empty() {
        let xml = "<r><Name>bucket</Name></r>";
        let keys = xml_tags(xml, "Key");
        assert!(keys.is_empty());
    }

    // ── Signed headers == sent headers, for every request the client builds ──

    /// Builds each kind of request and checks the three invariants a strict
    /// server relies on: every signed header is on the request, every
    /// `x-amz-*` header on the request is signed, and the request line is the
    /// canonical form that was signed.
    fn assert_signed_equals_sent(client: &S3Client, spec: &RequestSpec) {
        let (request, url) = client
            .build_request(spec, Arc::new(Progress::new()))
            .expect("request builds");
        let authorization = request
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .expect("authorization header")
            .to_string();
        let signed: Vec<&str> = authorization
            .split("SignedHeaders=")
            .nth(1)
            .and_then(|rest| rest.split(',').next())
            .expect("SignedHeaders component")
            .split(';')
            .collect();
        for name in &signed {
            assert!(
                request.headers().contains_key(*name),
                "{}: header `{name}` is signed but not sent (sent: {:?})",
                spec.operation,
                request.headers().keys().collect::<Vec<_>>()
            );
        }
        for name in request.headers().keys() {
            if name.as_str().starts_with("x-amz-") {
                assert!(
                    signed.contains(&name.as_str()),
                    "{}: header `{name}` is sent but not signed",
                    spec.operation
                );
            }
        }
        assert!(signed.contains(&"host"));
        assert_eq!(
            request.headers().get("host").and_then(|v| v.to_str().ok()),
            Some(client.host().as_str())
        );
        // Sorted, no duplicates.
        let mut sorted = signed.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, signed, "SignedHeaders must be sorted and unique");
        // The request line carries the signed canonical path.
        let uri_path = request.uri().path();
        let expected_path = encode_path(&client.inner.target.raw_path(spec.key.as_deref()));
        assert_eq!(uri_path, expected_path, "{url}");
    }

    fn every_spec() -> Vec<RequestSpec> {
        let body = Bytes::from_static(b"{\"hello\":\"world\"}");
        vec![
            RequestSpec::new("HeadObject", Method::HEAD).key("o/a b.json"),
            RequestSpec::new("DeleteObject", Method::DELETE).key("o/x"),
            RequestSpec::new("ListObjectsV2", Method::GET)
                .query("list-type", "2")
                .query("encoding-type", "url")
                .query("max-keys", "1000")
                .query("prefix", "o/"),
            RequestSpec::new("PutObject", Method::PUT)
                .key("o/k.json")
                .header("content-type", "application/json")
                .user_metadata(&[("sha256", "ab12")])
                .unwrap()
                .body(body.clone()),
            RequestSpec::new("CreateMultipartUpload", Method::POST)
                .key("o/big")
                .query("uploads", "")
                .header("content-type", "application/octet-stream")
                .user_metadata(&[("sha256", "ab12")])
                .unwrap(),
            RequestSpec::new("UploadPart", Method::PUT)
                .key("o/big")
                .query("partNumber", "2")
                .query("uploadId", "id/with+odd=chars")
                .body(body.clone()),
            RequestSpec::new("CompleteMultipartUpload", Method::POST)
                .key("o/big")
                .query("uploadId", "id")
                .header("content-type", "application/xml")
                .body(body)
                .error_in_ok_body(),
            RequestSpec::new("AbortMultipartUpload", Method::DELETE)
                .key("o/big")
                .query("uploadId", "id"),
        ]
    }

    #[test]
    fn every_request_signs_exactly_what_it_sends() {
        for custom in [false, true] {
            let cfg = if custom {
                sample_config_custom_endpoint()
            } else {
                sample_config()
            };
            let client = S3Client::new(cfg).unwrap();
            for spec in every_spec() {
                assert_signed_equals_sent(&client, &spec);
            }
        }
    }

    #[test]
    fn session_token_is_sent_and_signed_on_every_request() {
        let mut cfg = sample_config_custom_endpoint();
        cfg.session_token = Some("FQoGZXIvYXdzE//token==".to_string());
        let client = S3Client::new(cfg).unwrap();
        for spec in every_spec() {
            assert_signed_equals_sent(&client, &spec);
            let (request, _) = client
                .build_request(&spec, Arc::new(Progress::new()))
                .unwrap();
            assert_eq!(
                request
                    .headers()
                    .get("x-amz-security-token")
                    .and_then(|v| v.to_str().ok()),
                Some("FQoGZXIvYXdzE//token==")
            );
        }
    }

    #[test]
    fn head_and_delete_and_list_send_no_content_type() {
        let client = S3Client::new(sample_config_custom_endpoint()).unwrap();
        for spec in every_spec() {
            if matches!(
                spec.operation,
                "HeadObject" | "DeleteObject" | "ListObjectsV2" | "AbortMultipartUpload"
            ) {
                let (request, _) = client
                    .build_request(&spec, Arc::new(Progress::new()))
                    .unwrap();
                assert!(!request.headers().contains_key("content-type"));
                let auth = request.headers()["authorization"].to_str().unwrap();
                assert!(!auth.contains("content-type"), "{auth}");
            }
        }
    }

    #[test]
    fn value_less_sub_resource_is_sent_bare_and_signed_with_equals() {
        let client = S3Client::new(sample_config_custom_endpoint()).unwrap();
        let spec = RequestSpec::new("CreateMultipartUpload", Method::POST)
            .key("o/big")
            .query("uploads", "");
        let (request, url) = client
            .build_request(&spec, Arc::new(Progress::new()))
            .unwrap();
        assert!(url.ends_with("/my-bucket/o/big?uploads"), "{url}");
        assert_eq!(request.uri().query(), Some("uploads"));
    }

    #[test]
    fn query_parameters_are_sorted_and_encoded_on_the_request_line() {
        let client = S3Client::new(sample_config_custom_endpoint()).unwrap();
        let spec = RequestSpec::new("UploadPart", Method::PUT)
            .key("o/big")
            .query("uploadId", "a b")
            .query("partNumber", "3");
        let (request, _) = client
            .build_request(&spec, Arc::new(Progress::new()))
            .unwrap();
        assert_eq!(request.uri().query(), Some("partNumber=3&uploadId=a%20b"));
    }

    #[test]
    fn metadata_names_and_values_are_validated() {
        assert!(RequestSpec::new("PutObject", Method::PUT)
            .user_metadata(&[("Upper", "x")])
            .is_err());
        assert!(RequestSpec::new("PutObject", Method::PUT)
            .user_metadata(&[("ok", "line\nbreak")])
            .is_err());
        assert!(RequestSpec::new("PutObject", Method::PUT)
            .user_metadata(&[("ok", " padded ")])
            .is_err());
        assert!(RequestSpec::new("PutObject", Method::PUT)
            .user_metadata(&[("ok", "fine-value_1")])
            .is_ok());
    }

    #[test]
    fn keys_are_validated_before_any_request() {
        assert!(validate_key("a/b").is_ok());
        assert!(validate_key("").is_err());
        assert!(validate_key(&"k".repeat(1025)).is_err());
        assert!(validate_key(&"k".repeat(1024)).is_ok());
    }

    #[test]
    fn retry_delays_are_bounded_and_grow() {
        let policy = RetryPolicy {
            max_attempts: 6,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(1),
        };
        for attempt in 1..=10 {
            let delay = policy.delay_for(attempt);
            let ceiling = (policy.base_delay * (1 << (attempt - 1).min(20))).min(policy.max_delay);
            assert!(
                delay <= ceiling,
                "attempt {attempt}: {delay:?} > {ceiling:?}"
            );
            assert!(
                delay >= ceiling / 2,
                "attempt {attempt}: {delay:?} < half of {ceiling:?}"
            );
        }
        assert!(policy.delay_for(1) <= Duration::from_millis(100));
        assert!(policy.delay_for(8) >= Duration::from_millis(500));
    }

    #[test]
    fn retry_none_makes_a_single_attempt() {
        assert_eq!(RetryPolicy::none().max_attempts, 1);
    }

    #[test]
    fn loopback_detection() {
        assert!(is_loopback("127.0.0.1:9000"));
        assert!(is_loopback("localhost"));
        assert!(is_loopback("localhost:9000"));
        assert!(is_loopback("[::1]:9000"));
        assert!(!is_loopback("minio.internal:9000"));
        assert!(!is_loopback("192.168.1.5"));
    }

    #[tokio::test]
    async fn idle_timeout_fires_when_nothing_moves() {
        let progress = Progress::new();
        let started = Instant::now();
        let result = with_idle_timeout(
            std::future::pending::<()>(),
            &progress,
            Duration::from_millis(150),
        )
        .await;
        assert!(result.is_none());
        assert!(started.elapsed() >= Duration::from_millis(140));
    }

    #[tokio::test]
    async fn idle_timeout_does_not_fire_while_progress_is_made() {
        let progress = Arc::new(Progress::new());
        let ticker = Arc::clone(&progress);
        let work = async move {
            // Ten ticks 60 ms apart (600 ms in total) against a 200 ms idle
            // timeout: the total exceeds the timeout, the gaps do not.
            for _ in 0..10 {
                tokio::time::sleep(Duration::from_millis(60)).await;
                ticker.touch();
            }
            "done"
        };
        let result = with_idle_timeout(work, &progress, Duration::from_millis(200)).await;
        assert_eq!(result, Some("done"));
    }

    #[test]
    fn body_error_document_detection() {
        assert!(body_is_error_document(
            b"<?xml version=\"1.0\"?><Error><Code>InternalError</Code><Message>x</Message></Error>"
        ));
        assert!(!body_is_error_document(
            b"<CompleteMultipartUploadResult><Key>k</Key></CompleteMultipartUploadResult>"
        ));
        assert!(!body_is_error_document(b""));
    }
}
