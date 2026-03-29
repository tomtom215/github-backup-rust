// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Social graph and gist listing endpoints.
//!
//! Covers followers, following, starred repos, watched repos, and gists for a
//! given user.

use github_backup_types::{Gist, Repository, User};

use crate::error::ClientError;

use super::super::{GitHubClient, PER_PAGE};

impl GitHubClient {
    // ── User social graph ─────────────────────────────────────────────────

    /// Returns the followers of a user.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_followers(&self, username: &str) -> Result<Vec<User>, ClientError> {
        let api = self.api();
        let url = format!("{api}/users/{username}/followers?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns the users that `username` is following.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_following(&self, username: &str) -> Result<Vec<User>, ClientError> {
        let api = self.api();
        let url = format!("{api}/users/{username}/following?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns repositories starred by `username`.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_starred(&self, username: &str) -> Result<Vec<Repository>, ClientError> {
        let api = self.api();
        let url = format!("{api}/users/{username}/starred?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns repositories watched by `username`.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_watched(&self, username: &str) -> Result<Vec<Repository>, ClientError> {
        let api = self.api();
        let url = format!("{api}/users/{username}/subscriptions?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    // ── Gists ─────────────────────────────────────────────────────────────

    /// Returns gists owned by `username`.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_gists(&self, username: &str) -> Result<Vec<Gist>, ClientError> {
        let api = self.api();
        let url = format!("{api}/users/{username}/gists?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns gists starred by the authenticated user.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_starred_gists(&self) -> Result<Vec<Gist>, ClientError> {
        let api = self.api();
        let url = format!("{api}/gists/starred?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }
}
