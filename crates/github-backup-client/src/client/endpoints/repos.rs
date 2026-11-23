// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! User and organisation repository listing endpoints.

use tracing::info;

use github_backup_types::{Page, Raw, Repository};

use crate::error::ClientError;

use super::super::{GitHubClient, PER_PAGE};
use super::{strip_clone_token, without_clone_tokens};

impl GitHubClient {
    // ── User & org repos ──────────────────────────────────────────────────

    /// Lists repositories owned by a user.
    ///
    /// `GET /users/{username}/repos` lists **public** repositories only, so
    /// when the credential belongs to `username` (compared case-insensitively)
    /// the authenticated listing `GET /user/repos?affiliation=owner&visibility=all`
    /// is used instead: it returns the repositories the account owns,
    /// private ones included.  For any other user, for an anonymous client,
    /// and for tokens that cannot call `GET /user` (GitHub App tokens, with a
    /// warning), the public listing is used.  A GitHub App credential lists
    /// the repositories its installation can access, private ones included.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_user_repos(&self, username: &str) -> Result<Page<Repository>, ClientError> {
        if self.is_app() {
            return self.list_installation_repos(username).await;
        }
        let api = self.api();
        let url = if self.is_authenticated_user(username).await? {
            info!(
                username,
                "the token belongs to this account: listing its private repositories too"
            );
            format!("{api}/user/repos?affiliation=owner&visibility=all&per_page={PER_PAGE}")
        } else {
            format!("{api}/users/{username}/repos?type=all&per_page={PER_PAGE}")
        };
        Ok(without_clone_tokens(self.get_all_pages(&url).await?))
    }

    /// Lists repositories belonging to an organisation.  A GitHub App
    /// credential lists the repositories its installation can access.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_org_repos(&self, org: &str) -> Result<Page<Repository>, ClientError> {
        if self.is_app() {
            return self.list_installation_repos(org).await;
        }
        let api = self.api();
        let url = format!("{api}/orgs/{org}/repos?type=all&per_page={PER_PAGE}");
        Ok(without_clone_tokens(self.get_all_pages(&url).await?))
    }

    /// Returns one repository in full (`GET /repos/{owner}/{repo}`): unlike
    /// the listings it has `parent`, `source`, `template_repository`, the
    /// merge settings and `security_and_analysis`.  `temp_clone_token`, a
    /// short-lived credential, is removed.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn get_repository(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Raw<Repository>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}");
        let (value, _) = self.get_json_with_link::<serde_json::Value>(&url).await?;
        Ok(Raw::from_value(strip_clone_token(value))?)
    }
}
