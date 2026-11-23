// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! The token a `git` process is given.
//!
//! A personal access token stays the same for the whole run, but a GitHub App
//! installation token expires after an hour, and a backup can run for longer.
//! The git runner therefore asks a [`GitToken`] for the token each time it
//! starts a `git` process, and a refreshed token reaches every later process.

use std::fmt;
use std::sync::Arc;

use crate::api_client::BoxFuture;
use crate::error::ClientError;

/// Supplies the current access token, renewing it when needed.
///
/// Implementations must be single-flight: when several callers ask while the
/// token is being renewed, one renewal serves them all.
pub trait TokenSource: Send + Sync {
    /// The token to use now.
    ///
    /// # Errors
    ///
    /// When a new token is needed and cannot be obtained.
    fn token(&self) -> BoxFuture<'_, Result<String, ClientError>>;
}

/// The credential handed to `git` for HTTPS remotes.
#[derive(Clone)]
pub enum GitToken {
    /// A token that does not change during the run (a personal access or
    /// OAuth token).
    Fixed(String),
    /// A token that is renewed while the run lasts (a GitHub App installation
    /// token); asked for at every `git` start.
    Refreshed(Arc<dyn TokenSource>),
}

impl GitToken {
    /// The token to give a `git` process that is about to start.
    ///
    /// # Errors
    ///
    /// When a [`GitToken::Refreshed`] token cannot be renewed.
    pub async fn current(&self) -> Result<String, ClientError> {
        match self {
            GitToken::Fixed(token) => Ok(token.clone()),
            GitToken::Refreshed(source) => source.token().await,
        }
    }
}

impl From<String> for GitToken {
    fn from(token: String) -> Self {
        GitToken::Fixed(token)
    }
}

impl fmt::Debug for GitToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never the token itself.
        match self {
            GitToken::Fixed(_) => f.write_str("GitToken::Fixed([redacted])"),
            GitToken::Refreshed(_) => f.write_str("GitToken::Refreshed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_the_token() {
        let token = GitToken::from("ghp_secretvalue123".to_string());
        assert!(!format!("{token:?}").contains("secretvalue"));
    }

    #[tokio::test]
    async fn a_fixed_token_is_returned_as_is() {
        let token = GitToken::Fixed("abc".into());
        assert_eq!(token.current().await.unwrap(), "abc");
    }
}
