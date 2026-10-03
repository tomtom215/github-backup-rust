// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Social graph and gist listing endpoints.
//!
//! Covers followers, following, starred repos, watched repos, and gists for a
//! given user.

use tracing::info;

use github_backup_types::{Gist, Page, Repository, User};

use crate::error::ClientError;

use super::super::{GitHubClient, PER_PAGE};

impl GitHubClient {
    // ── User social graph ─────────────────────────────────────────────────

    /// Returns the followers of a user.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_followers(&self, username: &str) -> Result<Page<User>, ClientError> {
        let api = self.api();
        let url = format!("{api}/users/{username}/followers?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns the users that `username` is following.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_following(&self, username: &str) -> Result<Page<User>, ClientError> {
        let api = self.api();
        let url = format!("{api}/users/{username}/following?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns repositories starred by `username`.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_starred(&self, username: &str) -> Result<Page<Repository>, ClientError> {
        let api = self.api();
        let url = format!("{api}/users/{username}/starred?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns repositories watched by `username`.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_watched(&self, username: &str) -> Result<Page<Repository>, ClientError> {
        let api = self.api();
        let url = format!("{api}/users/{username}/subscriptions?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    // ── Gists ─────────────────────────────────────────────────────────────

    /// Returns gists owned by `username`.
    ///
    /// `GET /users/{username}/gists` lists **public** gists only, so when the
    /// credential belongs to `username` (compared case-insensitively) the
    /// authenticated listing `GET /gists` is used instead; it includes the
    /// account's secret gists.  For any other user, for an anonymous client,
    /// and for tokens that cannot call `GET /user`, the public listing is used.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_gists(&self, username: &str) -> Result<Page<Gist>, ClientError> {
        let api = self.api();
        let url = if self.is_authenticated_user(username).await? {
            info!(
                username,
                "the token belongs to this account: listing its secret gists too"
            );
            format!("{api}/gists?per_page={PER_PAGE}")
        } else {
            format!("{api}/users/{username}/gists?per_page={PER_PAGE}")
        };
        self.get_all_pages(&url).await
    }

    /// Returns gists starred by the authenticated user.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_starred_gists(&self) -> Result<Page<Gist>, ClientError> {
        let api = self.api();
        let url = format!("{api}/gists/starred?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }
}
