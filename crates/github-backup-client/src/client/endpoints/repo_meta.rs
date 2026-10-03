// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository metadata listing and asset download endpoints.
//!
//! Covers labels, milestones, releases, hooks, security advisories, topics,
//! branches, and release asset downloads.

use bytes::Bytes;
use http_body_util::Full;
use hyper::Method;
use tracing::info;

use github_backup_types::{
    Branch, BranchProtection, Hook, Label, Milestone, Page, Raw, Release, SecurityAdvisory,
};

use crate::error::ClientError;

use super::super::{collect_body, GitHubClient, DEFAULT_TIMEOUT_SECS, PER_PAGE};

impl GitHubClient {
    // ── Repository metadata ───────────────────────────────────────────────

    /// Lists labels for a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_labels(&self, owner: &str, repo: &str) -> Result<Page<Label>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/labels?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists milestones for a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_milestones(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Page<Milestone>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/milestones?state=all&per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists releases for a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_releases(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Page<Release>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/releases?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists webhooks configured on a repository.
    ///
    /// Requires `admin` permission on the repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_hooks(&self, owner: &str, repo: &str) -> Result<Page<Hook>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/hooks?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists published security advisories for a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_security_advisories(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Page<SecurityAdvisory>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/security-advisories?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns the topics (tags) configured on a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_repo_topics(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<String>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/topics");

        let req = self
            .build_request(Method::GET, &url)?
            .header("Accept", "application/vnd.github.v3+json")
            .body(Full::new(Bytes::new()))
            .map_err(ClientError::Http)?;

        let response = tokio::time::timeout(
            std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            self.http.request(req),
        )
        .await
        .map_err(|_| ClientError::Timeout { url: url.clone() })??;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = collect_body(response.into_body()).await?;
            return Err(ClientError::ApiError {
                status,
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }

        let body = collect_body(response.into_body()).await?;
        #[derive(serde::Deserialize)]
        struct TopicsResponse {
            names: Vec<String>,
        }
        let parsed: TopicsResponse = serde_json::from_slice(&body)?;
        info!(owner, repo, count = parsed.names.len(), "fetched topics");
        Ok(parsed.names)
    }

    /// Lists all branches for a repository.
    ///
    /// Returns branch names, their tip commit SHAs, and whether each branch
    /// has protection rules enabled.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_branches(&self, owner: &str, repo: &str) -> Result<Page<Branch>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/branches?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns the detailed branch-protection rules for a single branch.
    ///
    /// Calls `GET /repos/{owner}/{repo}/branches/{branch}/protection`.
    ///
    /// Returns `Err(ClientError::ApiError { status: 403, .. })` when the
    /// authenticated token lacks admin access to the repository, and
    /// `Err(ClientError::ApiError { status: 404, .. })` when the branch does
    /// not have protection enabled.  Callers should handle both gracefully.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or other API errors.
    pub async fn get_branch_protection(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Raw<BranchProtection>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/branches/{branch}/protection");
        let (protection, _) = self
            .get_json_with_link::<Raw<BranchProtection>>(&url)
            .await?;
        Ok(protection)
    }
}
