// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Error type for the backup core engine.

use thiserror::Error;

use github_backup_client::ClientError;

/// Errors that can occur during a backup run.
#[derive(Debug, Error)]
pub enum CoreError {
    /// An error from the GitHub API client.
    #[error("GitHub API error: {0}")]
    Client(#[from] ClientError),

    /// A filesystem I/O error.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// The path that caused the error.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// JSON serialisation or deserialisation failed.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// A `git` subprocess exited with a non-zero status.
    #[error("git {args} failed (exit {code}): {stderr}")]
    GitFailed {
        /// The git arguments (for context).
        args: String,
        /// The exit code.
        code: i32,
        /// Standard error output from git.
        stderr: String,
    },

    /// The `git` binary could not be found or launched.
    #[error("could not start git: {0}")]
    GitSpawn(std::io::Error),

    /// A path cannot be converted to UTF-8, which is required to pass it to
    /// git as a command-line argument.
    #[error("path contains non-UTF-8 bytes: {path}")]
    NonUtf8Path {
        /// The lossy string representation of the offending path.
        path: String,
    },
}

impl CoreError {
    /// Creates a [`CoreError::Io`] from an [`std::io::Error`] and a path.
    pub fn io(path: impl ToString, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_string(),
            source,
        }
    }
}
