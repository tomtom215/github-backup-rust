// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! User and organisation repository listing endpoints.

use github_backup_types::Repository;

use crate::error::ClientError;

use super::super::{GitHubClient, PER_PAGE};

impl GitHubClient {
    // ── User & org repos ──────────────────────────────────────────────────

    /// Lists repositories owned by a user.
    ///
    /// Includes all repository types the credential has access to. Private
    /// repositories are returned when the token has the `repo` scope.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_user_repos(&self, username: &str) -> Result<Vec<Repository>, ClientError> {
        let api = self.api();
        let url = format!("{api}/users/{username}/repos?type=all&per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists repositories belonging to an organisation.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_org_repos(&self, org: &str) -> Result<Vec<Repository>, ClientError> {
        let api = self.api();
        let url = format!("{api}/orgs/{org}/repos?type=all&per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }
}
