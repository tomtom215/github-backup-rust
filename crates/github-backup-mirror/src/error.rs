// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Error types for the mirror crate.

use thiserror::Error;

/// Errors that can occur while pushing mirrors to a remote Git host.
#[derive(Debug, Error)]
pub enum MirrorError {
    /// HTTP transport error from hyper-util.
    #[error("HTTP transport error: {0}")]
    Transport(#[from] hyper_util::client::legacy::Error),

    /// HTTP body error.
    #[error("HTTP body error: {0}")]
    Http(#[from] hyper::Error),

    /// The Gitea API returned a non-success status code.
    #[error("Gitea API error {status}: {body}")]
    Api {
        /// HTTP status code.
        status: u16,
        /// Response body for debugging.
        body: String,
    },

    /// TLS configuration failed to load.
    #[error("TLS error: {0}")]
    Tls(String),

    /// Request construction failed.
    #[error("request error: {0}")]
    Request(#[from] hyper::http::Error),

    /// JSON serialisation or deserialisation failed.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// The source repository path is not valid UTF-8.
    #[error("invalid path (non-UTF-8): {path}")]
    NonUtf8Path {
        /// The offending path, lossily converted.
        path: String,
    },

    /// A git subprocess failed.
    #[error("git {args:?} exited with code {code}: {stderr}")]
    GitFailed {
        /// The git arguments that were passed.
        args: String,
        /// Exit code (−1 if unavailable).
        code: i32,
        /// Standard error output from git.
        stderr: String,
    },

    /// Could not spawn the git subprocess.
    #[error("could not spawn git: {0}")]
    GitSpawn(#[from] std::io::Error),

    /// Request timed out.
    #[error("request to {url} timed out")]
    Timeout {
        /// The URL that timed out.
        url: String,
    },
}
