// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Error types for the mirror crate.

use thiserror::Error;

/// Longest part of an API response body quoted in an error message.
const QUOTED_BODY_CHARS: usize = 500;

/// Errors that can occur while pushing mirrors to a remote Git host.
#[derive(Debug, Error)]
pub enum MirrorError {
    /// HTTP transport error from hyper-util.
    #[error("HTTP transport error: {0}")]
    Transport(#[from] hyper_util::client::legacy::Error),

    /// HTTP body error.
    #[error("HTTP body error: {0}")]
    Http(#[from] hyper::Error),

    /// A response body could not be read (or exceeded the size limit).
    #[error("could not read the API response: {0}")]
    Body(String),

    /// The mirror destination's API returned an unexpected status code.
    #[error("mirror API error {status}: {}", quote(body))]
    Api {
        /// HTTP status code.
        status: u16,
        /// Response body, as sent.
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

    /// The destination already holds a repository of that name that this tool
    /// did not create; pushing into it could destroy unrelated work.
    #[error(
        "repository {repo:?} already exists at the mirror destination and was not created by \
         github-backup (its description is not the mirror marker); refusing to overwrite it"
    )]
    ForeignRepository {
        /// Repository name.
        repo: String,
    },

    /// No API token was configured.
    #[error(
        "no API token for the mirror destination; set MIRROR_TOKEN (or --mirror-token, or \
         mirror_token in the config file)"
    )]
    MissingToken,

    /// The destination URL is not `https://`; the token would travel in
    /// clear text.
    #[error(
        "the mirror destination {url:?} is not an https:// URL; the API token is only sent over \
         HTTPS"
    )]
    InsecureUrl {
        /// The configured base URL.
        url: String,
    },

    /// The GitLab namespace (user or group) does not exist or the token
    /// cannot see it.
    #[error("GitLab namespace {namespace:?} was not found (or the token cannot access it)")]
    NamespaceNotFound {
        /// The configured namespace path.
        namespace: String,
    },

    /// Request timed out.
    #[error("request to {url} timed out")]
    Timeout {
        /// The URL that timed out.
        url: String,
    },
}

/// `body`, cut to [`QUOTED_BODY_CHARS`] characters.
fn quote(body: &str) -> String {
    let body = body.trim();
    match body.char_indices().nth(QUOTED_BODY_CHARS) {
        Some((cut, _)) => format!("{}…", &body[..cut]),
        None => body.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_errors_quote_the_body_up_to_a_limit() {
        let short = MirrorError::Api {
            status: 422,
            body: "{\"message\":\"name is invalid\"}".into(),
        };
        assert_eq!(
            short.to_string(),
            "mirror API error 422: {\"message\":\"name is invalid\"}"
        );
        let long = MirrorError::Api {
            status: 500,
            body: "x".repeat(10_000),
        };
        assert!(long.to_string().len() < 600, "{long}");
    }
}
