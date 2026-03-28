// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Error types for the S3 storage backend.

use thiserror::Error;

/// Errors that can occur while reading from or writing to an S3-compatible
/// object store.
#[derive(Debug, Error)]
pub enum S3Error {
    /// An HTTP transport error from hyper-util.
    #[error("HTTP transport error: {0}")]
    Transport(#[from] hyper_util::client::legacy::Error),

    /// An HTTP body error.
    #[error("HTTP body error: {0}")]
    Http(#[from] hyper::Error),

    /// The S3 service returned a non-success response.
    #[error("S3 error {status}: {body}")]
    Api {
        /// HTTP status code returned by the service.
        status: u16,
        /// Response body (may be XML error details).
        body: String,
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

    /// A request timed out.
    #[error("request to {url} timed out")]
    Timeout {
        /// URL that timed out.
        url: String,
    },

    /// Invalid S3 endpoint URL.
    #[error("invalid S3 endpoint URL: {0}")]
    InvalidEndpoint(String),
}
