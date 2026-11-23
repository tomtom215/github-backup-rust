// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Error type for the GitHub API client.

use thiserror::Error;

/// Errors that can occur when interacting with the GitHub API.
#[derive(Debug, Error)]
pub enum ClientError {
    /// An HTTP transport error from hyper.
    #[error("HTTP transport error: {0}")]
    Transport(#[from] hyper_util::client::legacy::Error),

    /// An error building or encoding an HTTP request.
    #[error("HTTP error: {0}")]
    Http(#[from] hyper::http::Error),

    /// An error reading or assembling the response body.
    #[error("body error: {0}")]
    Body(#[from] hyper::Error),

    /// The response body could not be deserialised as JSON.
    #[error("JSON deserialisation error: {0}")]
    Json(#[from] serde_json::Error),

    /// Storing a streamed download at its local destination failed.
    #[error("local write error: {0}")]
    Io(#[from] std::io::Error),

    /// GitHub returned a non-success HTTP status code.
    ///
    /// The `status` field contains the numeric code; `body` contains the
    /// raw response text for diagnostic purposes.
    #[error("GitHub API error {status}: {body}")]
    ApiError {
        /// HTTP status code.
        status: u16,
        /// Raw response body.
        body: String,
    },

    /// A response body was larger than the client accepts for an API
    /// response.
    #[error("response body exceeds the {} MiB cap", limit_bytes / (1024 * 1024))]
    BodyTooLarge {
        /// The cap, in bytes.
        limit_bytes: usize,
    },

    /// The `Link` header of a paginated list cannot be followed safely: it
    /// points back to a page already fetched, or to another origin.
    #[error("pagination error: {0}")]
    Pagination(String),

    /// A GraphQL response carried an `errors` array.
    #[error("GraphQL error: {message}")]
    GraphQl {
        /// The `message` of every error, joined with `; `.
        message: String,
        /// The `type` of every error that has one (for example `NOT_FOUND`
        /// or `FORBIDDEN`), so that callers can tell "absent" from "failed".
        kinds: Vec<String>,
    },

    /// The OAuth server answered with an error the device flow cannot
    /// recover from (RFC 8628 section 3.5 or RFC 6749 section 5.2).
    #[error("OAuth error {error}{}", description.as_deref().map(|d| format!(": {d}")).unwrap_or_default())]
    OAuth {
        /// The `error` code, for example `incorrect_client_credentials`.
        error: String,
        /// The server's `error_description`, if any.
        description: Option<String>,
    },

    /// A URL could not be parsed.
    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),

    /// The configured API base URL cannot be used.
    #[error("invalid API URL: {0}")]
    InvalidApiUrl(String),

    /// TLS configuration failed.
    #[error("TLS error: {0}")]
    Tls(String),

    /// The rate limit was exceeded and the retry limit was reached.
    #[error("rate limit exceeded; retry after {retry_after_secs}s")]
    RateLimitExceeded {
        /// Suggested wait before retrying (seconds).
        retry_after_secs: u64,
    },

    /// The request did not complete within the configured timeout.
    #[error("request timed out: {url}")]
    Timeout {
        /// The URL that timed out.
        url: String,
    },

    /// The OAuth device flow session expired before the user authorised.
    #[error("OAuth device code expired; run again to get a new code")]
    OAuthExpired,

    /// The user explicitly denied the OAuth authorisation request.
    #[error("OAuth authorisation was denied by the user")]
    OAuthDenied,

    /// A GitHub App credential cannot be used: its private key is unusable,
    /// or the installation to act as is unknown or missing.
    #[error("GitHub App: {0}")]
    App(String),
}
