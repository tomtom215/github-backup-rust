// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Deploy key and collaborator listing endpoints.

use github_backup_types::{Collaborator, DeployKey};

use crate::error::ClientError;

use super::super::{GitHubClient, PER_PAGE};

impl GitHubClient {
    // ── Deploy keys ───────────────────────────────────────────────────────

    /// Lists deploy keys configured on a repository.
    ///
    /// Requires admin access to the repository; callers should handle
    /// [`ClientError::ApiError`] with status 403 or 404 gracefully.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_deploy_keys(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<DeployKey>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/keys?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    // ── Collaborators ─────────────────────────────────────────────────────

    /// Lists collaborators on a repository.
    ///
    /// Returns users who have been explicitly granted access to the repository.
    /// Requires admin access; callers should handle 403/404 gracefully.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_collaborators(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<Collaborator>, ClientError> {
        let api = self.api();
        let url =
            format!("{api}/repos/{owner}/{repo}/collaborators?per_page={PER_PAGE}&affiliation=all");
        self.get_all_pages(&url).await
    }
}
