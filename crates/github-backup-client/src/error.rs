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

    /// A URL could not be parsed.
    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),

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
}
