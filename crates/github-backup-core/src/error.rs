// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Error type for the backup core engine.

use thiserror::Error;

use github_backup_client::ClientError;

/// Errors that can occur during a backup run.
#[derive(Debug, Error)]
pub enum CoreError {
    /// An error from the GitHub API client.
    #[error(transparent)]
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
    #[error("git {} failed (exit {code}): {stderr}", git_operation(.args))]
    GitFailed {
        /// The git arguments (for context).
        args: String,
        /// The exit code.
        code: i32,
        /// Standard error output from git.
        stderr: String,
    },

    /// A `git` subprocess was stopped because it made no progress (produced no
    /// output) for longer than the stall limit.
    #[error("git {} made no progress for {timeout_secs}s and was stopped", git_operation(.args))]
    GitTimeout {
        /// The git arguments (for context).
        args: String,
        /// The stall limit that was exceeded, in seconds.
        timeout_secs: u64,
    },

    /// The operation was abandoned because the process is shutting down
    /// (SIGINT/SIGTERM).
    #[error("interrupted: the process is shutting down")]
    Interrupted,

    /// A background task panicked or was cancelled before finishing.
    #[error("background task failed: {0}")]
    TaskFailed(String),

    /// The `git` binary could not be found or launched.
    #[error("could not start git: {0}")]
    GitSpawn(std::io::Error),

    /// `git fsck` reported repository corruption after a fresh clone.
    ///
    /// This is treated as a warning — the backup continues — but is surfaced
    /// as an error so callers can decide whether to abort or log and proceed.
    #[error("git fsck reported issues in {repo}: {output}")]
    GitFsckFailed {
        /// The repository path that was checked.
        repo: String,
        /// The fsck output (first 512 bytes).
        output: String,
    },

    /// A path cannot be converted to UTF-8, which is required to pass it to
    /// git as a command-line argument.
    #[error("path contains non-UTF-8 bytes: {path}")]
    NonUtf8Path {
        /// The lossy string representation of the offending path.
        path: String,
    },
}

/// The git operation (`clone`, `fetch`, `push`, ...) out of a full argument
/// list.  Messages name the operation only: the arguments carry URLs and local
/// paths that bury the reason, and the log already says which repository.
fn git_operation(args: &str) -> &str {
    args.split_whitespace()
        .find(|word| !word.starts_with('-'))
        .unwrap_or("command")
}

impl CoreError {
    /// Returns `true` if continuing the run is pointless.
    ///
    /// Everything else is *isolated*: it is recorded as a failure of one
    /// repository or category and the run carries on, so that one bad object
    /// never costs the backup of everything else.  Fatal errors are those
    /// that would simply repeat for every remaining item:
    ///
    /// * the process is shutting down ([`CoreError::Interrupted`]);
    /// * GitHub rejects the credentials (HTTP 401);
    /// * the rate-limit retry budget is exhausted;
    /// * the output disk is full.
    #[must_use]
    pub fn is_fatal(&self) -> bool {
        match self {
            Self::Interrupted => true,
            Self::Client(ClientError::ApiError { status: 401, .. }) => true,
            Self::Client(ClientError::RateLimitExceeded { .. }) => true,
            Self::Io { source, .. } => source.kind() == std::io::ErrorKind::StorageFull,
            _ => false,
        }
    }

    /// `true` for a git failure that says the remote repository does not exist
    /// (or is invisible to the credential): GitHub answers `Repository not
    /// found` both for a wiki that was never created and for a repository the
    /// token cannot see.
    ///
    /// This is deliberately narrower than "git exited with 128": 128 is also
    /// what git returns for rejected credentials, DNS and TLS failures and
    /// ownership errors, none of which may be mistaken for "nothing to back up".
    #[must_use]
    pub fn is_remote_missing(&self) -> bool {
        match self {
            Self::GitFailed { stderr, .. } => {
                let stderr = stderr.to_ascii_lowercase();
                stderr.contains("not found")
                    || stderr.contains("does not appear to be a git repository")
                    || stderr.contains("returned error: 404")
            }
            _ => false,
        }
    }

    /// Creates a [`CoreError::Io`] from an [`std::io::Error`] and a path.
    pub fn io(path: impl ToString, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_string(),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(status: u16) -> CoreError {
        CoreError::Client(ClientError::ApiError {
            status,
            body: String::new(),
        })
    }

    #[test]
    fn fatal_errors_stop_the_run() {
        assert!(CoreError::Interrupted.is_fatal());
        assert!(api(401).is_fatal());
        assert!(CoreError::Client(ClientError::RateLimitExceeded {
            retry_after_secs: 60
        })
        .is_fatal());
        let full = std::io::Error::from(std::io::ErrorKind::StorageFull);
        assert!(CoreError::io("/backup/x.json", full).is_fatal());
    }

    #[test]
    fn isolated_errors_do_not_stop_the_run() {
        for status in [403, 404, 410, 451, 500, 502] {
            assert!(!api(status).is_fatal(), "HTTP {status} must be isolated");
        }
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(!CoreError::io("/backup/x.json", denied).is_fatal());
        assert!(!CoreError::GitFailed {
            args: "clone".into(),
            code: 128,
            stderr: "fatal: repository not found".into()
        }
        .is_fatal());
        assert!(!CoreError::GitTimeout {
            args: "clone".into(),
            timeout_secs: 600
        }
        .is_fatal());
    }

    fn git(code: i32, stderr: &str) -> CoreError {
        CoreError::GitFailed {
            args: "clone --mirror".into(),
            code,
            stderr: stderr.into(),
        }
    }

    #[test]
    fn only_a_not_found_message_means_the_remote_is_missing() {
        assert!(git(
            128,
            "remote: Repository not found.\nfatal: repository 'https://github.com/o/r.wiki.git/' not found"
        )
        .is_remote_missing());
        assert!(git(
            128,
            "fatal: '/srv/o/r.wiki.git' does not appear to be a git repository"
        )
        .is_remote_missing());
        assert!(git(128, "fatal: unable to access 'https://h/r.wiki.git/': The requested URL returned error: 404").is_remote_missing());

        for stderr in [
            "fatal: Authentication failed for 'https://github.com/o/r.wiki.git/'",
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
            "fatal: unable to access 'https://github.com/o/r.wiki.git/': Could not resolve host: github.com",
            "fatal: detected dubious ownership in repository at '/backup/r.wiki.git'",
            "",
        ] {
            assert!(
                !git(128, stderr).is_remote_missing(),
                "exit 128 with {stderr:?} is a real failure, not an empty wiki"
            );
        }
        assert!(!CoreError::Interrupted.is_remote_missing());
    }

    #[test]
    fn git_errors_name_the_operation_not_the_whole_command_line() {
        let e = CoreError::GitFailed {
            args: "clone --progress --mirror file:///very/long/origin.git /very/long/dest.git"
                .into(),
            code: 128,
            stderr: "fatal: repository not found".into(),
        };
        assert_eq!(
            e.to_string(),
            "git clone failed (exit 128): fatal: repository not found"
        );
        let t = CoreError::GitTimeout {
            args: "fetch --progress --all".into(),
            timeout_secs: 60,
        };
        assert_eq!(
            t.to_string(),
            "git fetch made no progress for 60s and was stopped"
        );
        assert_eq!(git_operation("--progress"), "command");
    }
}
