// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Error types for the S3 storage backend.
//!
//! Failures are reported with the HTTP status **and** the `<Code>` /
//! `<Message>` of the S3 error document, followed by a one-line hint that says
//! what to check — a bare `403` tells an operator nothing.

use std::fmt;
use std::time::Duration;

use thiserror::Error;

use crate::encoding::xml_tag;

/// Longest response body kept inside an [`ApiError`].
const MAX_BODY_CHARS: usize = 2048;

/// A decoded non-success response from the object store.
#[derive(Debug, Clone)]
pub struct ApiError {
    /// The S3 operation that failed (`"PutObject"`, `"ListObjectsV2"`, …).
    pub operation: &'static str,
    /// HTTP status code.
    pub status: u16,
    /// The `<Code>` of the S3 error document (`AccessDenied`, …), if any.
    pub code: Option<String>,
    /// The `<Message>` of the S3 error document, if any.
    pub message: Option<String>,
    /// The raw response body, truncated (for debugging).
    pub body: String,
    /// Region the bucket really lives in, from `x-amz-bucket-region` or the
    /// error document, when the server told us.
    pub bucket_region: Option<String>,
    /// The `Retry-After` the server asked for, if any.
    pub retry_after: Option<Duration>,
}

impl ApiError {
    /// Builds an [`ApiError`] from a response.
    ///
    /// `bucket_region` is the value of the `x-amz-bucket-region` header and
    /// `retry_after` the parsed `Retry-After` header, when present.
    #[must_use]
    pub fn from_response(
        operation: &'static str,
        status: u16,
        body: &[u8],
        bucket_region: Option<String>,
        retry_after: Option<Duration>,
    ) -> Self {
        let text = String::from_utf8_lossy(body);
        let code = xml_tag(&text, "Code").filter(|c| !c.trim().is_empty());
        let message = xml_tag(&text, "Message").filter(|m| !m.trim().is_empty());
        let bucket_region = bucket_region
            .filter(|r| !r.is_empty())
            .or_else(|| xml_tag(&text, "Region"));
        let mut body: String = text.chars().take(MAX_BODY_CHARS).collect();
        if text.chars().count() > MAX_BODY_CHARS {
            body.push_str("...");
        }
        Self {
            operation,
            status,
            code,
            message,
            body,
            bucket_region,
            retry_after,
        }
    }

    /// Returns `true` when the request is worth repeating: server-side
    /// failures, throttling and timeouts, never client or permission errors.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        if matches!(self.status, 408 | 429 | 500 | 502 | 503 | 504) {
            return true;
        }
        matches!(
            self.code.as_deref(),
            Some(
                "SlowDown"
                    | "RequestTimeout"
                    | "InternalError"
                    | "ServiceUnavailable"
                    | "Throttling"
                    | "ThrottlingException"
                    | "RequestLimitExceeded"
                    | "BandwidthLimitExceeded"
                    | "OperationAborted"
            )
        )
    }

    /// Returns `true` for configuration- and authorisation-class failures
    /// that will hit every other object too.
    #[must_use]
    pub fn is_fatal(&self) -> bool {
        if matches!(self.status, 301 | 307 | 308 | 401) {
            return true;
        }
        matches!(
            self.code.as_deref(),
            Some(
                "AccessDenied"
                    | "InvalidAccessKeyId"
                    | "SignatureDoesNotMatch"
                    | "NoSuchBucket"
                    | "RequestTimeTooSkewed"
                    | "ExpiredToken"
                    | "InvalidToken"
                    | "TokenRefreshRequired"
                    | "AuthorizationHeaderMalformed"
                    | "PermanentRedirect"
                    | "AccountProblem"
                    | "AllAccessDisabled"
                    | "InvalidBucketName"
                    | "InvalidSecurity"
            )
        )
    }

    /// One line telling the operator what to check, when we know.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        let region = self.bucket_region.as_deref();
        let by_code = match self.code.as_deref() {
            Some("AccessDenied") => Some(
                "the credentials are accepted but not allowed to do this; check the bucket \
                 policy / IAM permissions (docs: storage/s3, \"IAM policy\") and the bucket name"
                    .to_string(),
            ),
            Some("SignatureDoesNotMatch") => Some(
                "the secret access key, region or endpoint is wrong, or something between \
                 here and the server changed the request; check --s3-secret-key, --s3-region \
                 and --s3-endpoint"
                    .to_string(),
            ),
            Some("InvalidAccessKeyId") => Some(
                "this endpoint does not know that access key id; check --s3-access-key and \
                 that --s3-endpoint belongs to the same provider"
                    .to_string(),
            ),
            Some("NoSuchBucket") => Some(
                "the bucket does not exist; create it first (github-backup never creates \
                 buckets) and check --s3-bucket, --s3-endpoint and --s3-region"
                    .to_string(),
            ),
            Some("RequestTimeTooSkewed") => Some(
                "this machine's clock differs from the server's by more than 15 minutes; \
                 fix the system time (NTP)"
                    .to_string(),
            ),
            Some("ExpiredToken" | "InvalidToken" | "TokenRefreshRequired") => Some(
                "the session token is expired or invalid; refresh AWS_SESSION_TOKEN / \
                 --s3-session-token (and the access key and secret that go with it)"
                    .to_string(),
            ),
            Some("AuthorizationHeaderMalformed" | "PermanentRedirect") => Some(match region {
                Some(r) => format!("the bucket lives in region '{r}'; pass --s3-region {r}"),
                None => "the --s3-region does not match the bucket's region; check \
                         --s3-region and --s3-endpoint"
                    .to_string(),
            }),
            Some("InvalidBucketName") => Some(
                "the bucket name is not valid for this service; check --s3-bucket".to_string(),
            ),
            Some("AccountProblem" | "AllAccessDisabled") => Some(
                "the account or bucket is disabled or has a billing problem; check with the \
                 provider"
                    .to_string(),
            ),
            Some("SlowDown" | "ServiceUnavailable" | "Throttling" | "RequestLimitExceeded") => {
                Some(
                    "the service is throttling requests; the request was retried with back-off \
                     and still failed, so lower the load or retry later"
                        .to_string(),
                )
            }
            Some("EntityTooSmall") => Some(
                "a multipart part was smaller than the service's minimum".to_string(),
            ),
            Some("KeyTooLongError") => {
                Some("the object key exceeds 1024 bytes; shorten the prefix or file name".to_string())
            }
            Some("NoSuchUpload") => {
                Some("the multipart upload no longer exists on the server".to_string())
            }
            Some("NotImplemented") => Some(
                "this S3-compatible service does not implement the request".to_string(),
            ),
            _ => None,
        };
        if by_code.is_some() {
            return by_code;
        }
        match self.status {
            301 | 307 | 308 => Some(match region {
                Some(r) => format!("the bucket lives in region '{r}'; pass --s3-region {r}"),
                None => "the endpoint redirected the request; check --s3-region and \
                         --s3-endpoint"
                    .to_string(),
            }),
            401 | 403 if self.code.is_none() => Some(
                "access denied (this response carried no details); check the credentials, the \
                 bucket policy and --s3-endpoint"
                    .to_string(),
            ),
            500 | 502 | 503 | 504 => Some(
                "the service had a server-side problem; the request was retried with back-off \
                 and still failed"
                    .to_string(),
            ),
            _ => None,
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "S3 {} failed with HTTP {}", self.operation, self.status)?;
        if let Some(code) = &self.code {
            write!(f, " {code}")?;
        }
        if let Some(message) = &self.message {
            write!(f, ": {message}")?;
        }
        if let Some(hint) = self.hint() {
            write!(f, " (hint: {hint})")?;
        }
        Ok(())
    }
}

impl std::error::Error for ApiError {}

/// Errors that can occur while reading from or writing to an S3-compatible
/// object store.
#[derive(Debug, Error)]
pub enum S3Error {
    /// The service answered with a non-success status (or a success status
    /// carrying an `<Error>` document, as `CompleteMultipartUpload` may do).
    #[error("{0}")]
    Api(Box<ApiError>),

    /// A network-level failure: connection refused or reset, TLS handshake
    /// failure, a response cut short.
    #[error("HTTP transport error: {message}")]
    Transport {
        /// The error and its causes, innermost last.
        message: String,
        /// Whether the failure happened while connecting.
        connect: bool,
    },

    /// A request made no progress for longer than the idle timeout.
    #[error("request to {url} timed out: no data moved for {idle_secs}s")]
    Timeout {
        /// URL that timed out.
        url: String,
        /// The idle timeout that expired, in seconds.
        idle_secs: u64,
    },

    /// TLS configuration failed.
    #[error("TLS error: {0}")]
    Tls(String),

    /// HTTP request construction failed.
    #[error("request build error: {0}")]
    Request(#[from] hyper::http::Error),

    /// JSON serialisation failed.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// A filesystem I/O error occurred (e.g., walking directories for sync).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Invalid S3 endpoint URL.
    #[error("invalid S3 endpoint URL: {0}")]
    InvalidEndpoint(String),

    /// The S3 settings are unusable (missing credentials, bad bucket name, …).
    #[error("invalid S3 configuration: {0}")]
    InvalidConfig(String),

    /// An object key or metadata value cannot be sent.
    #[error("invalid object key or metadata: {0}")]
    InvalidKey(String),

    /// The service answered successfully but the body was not what the
    /// protocol promises.
    #[error("unexpected S3 response: {0}")]
    InvalidResponse(String),

    /// AES-256-GCM encryption or decryption failed.
    #[error("encryption error: {0}")]
    Encrypt(String),
}

/// Renders an error and its `source()` chain as `outer: inner: innermost`.
pub(crate) fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}

impl From<hyper_util::client::legacy::Error> for S3Error {
    fn from(error: hyper_util::client::legacy::Error) -> Self {
        Self::Transport {
            connect: error.is_connect(),
            message: error_chain(&error),
        }
    }
}

impl From<hyper::Error> for S3Error {
    fn from(error: hyper::Error) -> Self {
        Self::Transport {
            connect: false,
            message: error_chain(&error),
        }
    }
}

impl S3Error {
    /// Builds an [`S3Error::Api`].
    #[must_use]
    pub fn api(error: ApiError) -> Self {
        Self::Api(Box::new(error))
    }

    /// Returns `true` when repeating the request may succeed.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Api(e) => e.is_retryable(),
            Self::Transport { .. } | Self::Timeout { .. } => true,
            _ => false,
        }
    }

    /// Returns `true` for failures that will hit every object in the run
    /// (bad credentials, missing bucket, wrong region, unreachable endpoint,
    /// unusable configuration) — there is no point trying the next file.
    #[must_use]
    pub fn is_fatal(&self) -> bool {
        match self {
            Self::Api(e) => e.is_fatal(),
            Self::Transport { connect, .. } => *connect,
            Self::Tls(_) | Self::InvalidConfig(_) | Self::InvalidEndpoint(_) => true,
            _ => false,
        }
    }

    /// The S3 `<Code>` when this is an API error carrying one.
    #[must_use]
    pub fn api_code(&self) -> Option<&str> {
        match self {
            Self::Api(e) => e.code.as_deref(),
            _ => None,
        }
    }

    /// The HTTP status when this is an API error.
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Api(e) => Some(e.status),
            _ => None,
        }
    }

    /// A one-line hint about what to check, when one applies.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        match self {
            Self::Transport { connect: true, .. } => Some(
                "cannot reach the endpoint; check --s3-endpoint, DNS and firewall (the S3 client \
                 connects directly and does not use HTTPS_PROXY)"
                    .to_string(),
            ),
            Self::Timeout { .. } => Some(
                "the connection stalled; check the network (retries with back-off were used)"
                    .to_string(),
            ),
            Self::Tls(_) => Some(
                "the system certificate store could not be loaded; install CA certificates or \
                 set SSL_CERT_FILE"
                    .to_string(),
            ),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(status: u16, body: &str) -> ApiError {
        ApiError::from_response("PutObject", status, body.as_bytes(), None, None)
    }

    const ACCESS_DENIED: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>AccessDenied</Code><Message>Access Denied</Message><RequestId>X</RequestId></Error>";

    #[test]
    fn display_carries_status_code_message_and_hint() {
        let text = api(403, ACCESS_DENIED).to_string();
        assert!(text.contains("HTTP 403"), "{text}");
        assert!(text.contains("AccessDenied"), "{text}");
        assert!(text.contains("Access Denied"), "{text}");
        assert!(text.contains("hint:"), "{text}");
        assert!(text.contains("PutObject"), "{text}");
    }

    #[test]
    fn every_documented_code_has_a_hint() {
        for code in [
            "AccessDenied",
            "SignatureDoesNotMatch",
            "InvalidAccessKeyId",
            "NoSuchBucket",
            "RequestTimeTooSkewed",
            "ExpiredToken",
            "InvalidToken",
            "AuthorizationHeaderMalformed",
            "PermanentRedirect",
            "SlowDown",
            "AccountProblem",
        ] {
            let body = format!("<Error><Code>{code}</Code><Message>m</Message></Error>");
            let e = api(400, &body);
            assert_eq!(e.code.as_deref(), Some(code));
            assert!(e.hint().is_some(), "{code} has no hint");
        }
    }

    #[test]
    fn region_hint_uses_the_header_value() {
        let e = ApiError::from_response(
            "HeadObject",
            301,
            b"",
            Some("eu-west-1".to_string()),
            None,
        );
        assert!(e.hint().unwrap().contains("--s3-region eu-west-1"));
    }

    #[test]
    fn empty_403_still_gets_a_hint_and_status() {
        let e = api(403, "");
        assert!(e.code.is_none());
        let text = e.to_string();
        assert!(text.contains("HTTP 403"), "{text}");
        assert!(text.contains("hint:"), "{text}");
    }

    #[test]
    fn retryable_classification() {
        assert!(api(500, "").is_retryable());
        assert!(api(503, "<Error><Code>SlowDown</Code></Error>").is_retryable());
        assert!(api(429, "").is_retryable());
        assert!(api(400, "<Error><Code>RequestTimeout</Code></Error>").is_retryable());
        assert!(!api(403, ACCESS_DENIED).is_retryable());
        assert!(!api(404, "<Error><Code>NoSuchBucket</Code></Error>").is_retryable());
        assert!(!api(501, "").is_retryable());
        assert!(!api(400, "<Error><Code>EntityTooSmall</Code></Error>").is_retryable());
    }

    #[test]
    fn fatal_classification() {
        assert!(api(403, ACCESS_DENIED).is_fatal());
        assert!(api(404, "<Error><Code>NoSuchBucket</Code></Error>").is_fatal());
        assert!(api(403, "<Error><Code>SignatureDoesNotMatch</Code></Error>").is_fatal());
        assert!(api(301, "").is_fatal());
        assert!(!api(404, "<Error><Code>NoSuchKey</Code></Error>").is_fatal());
        assert!(!api(503, "<Error><Code>SlowDown</Code></Error>").is_fatal());
    }

    #[test]
    fn transport_and_timeout_are_retryable() {
        let t = S3Error::Transport {
            message: "boom".to_string(),
            connect: false,
        };
        assert!(t.is_retryable());
        assert!(!t.is_fatal());
        let c = S3Error::Transport {
            message: "refused".to_string(),
            connect: true,
        };
        assert!(c.is_fatal());
        assert!(c.hint().unwrap().contains("--s3-endpoint"));
        let timeout = S3Error::Timeout {
            url: "http://x".to_string(),
            idle_secs: 60,
        };
        assert!(timeout.is_retryable());
    }

    #[test]
    fn body_is_truncated() {
        let big = "x".repeat(10_000);
        let e = api(500, &big);
        assert!(e.body.chars().count() <= MAX_BODY_CHARS + 3);
    }

    #[test]
    fn xml_entities_in_message_are_decoded() {
        let e = api(
            400,
            "<Error><Code>InvalidRequest</Code><Message>a &amp; b</Message></Error>",
        );
        assert_eq!(e.message.as_deref(), Some("a & b"));
    }

    #[test]
    fn error_chain_lists_causes_once() {
        let inner = std::io::Error::other("inner cause");
        let outer = std::io::Error::other(inner);
        let text = error_chain(&outer);
        assert!(text.contains("inner cause"), "{text}");
    }
}
