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
    Branch, BranchProtection, Hook, Label, Milestone, Release, SecurityAdvisory,
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
    pub async fn list_labels(&self, owner: &str, repo: &str) -> Result<Vec<Label>, ClientError> {
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
    ) -> Result<Vec<Milestone>, ClientError> {
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
    ) -> Result<Vec<Release>, ClientError> {
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
    pub async fn list_hooks(&self, owner: &str, repo: &str) -> Result<Vec<Hook>, ClientError> {
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
    ) -> Result<Vec<SecurityAdvisory>, ClientError> {
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
    pub async fn list_branches(&self, owner: &str, repo: &str) -> Result<Vec<Branch>, ClientError> {
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
    ) -> Result<BranchProtection, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/branches/{branch}/protection");
        let (protection, _) = self.get_json_with_link::<BranchProtection>(&url).await?;
        Ok(protection)
    }

    // ── Assets ────────────────────────────────────────────────────────────

    /// Downloads a release asset and returns the raw bytes.
    ///
    /// Uses the `application/octet-stream` accept header required by GitHub.
    /// Follows up to 3 HTTP redirects (GitHub redirects asset downloads to S3).
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn download_release_asset(&self, asset_url: &str) -> Result<Bytes, ClientError> {
        let mut url = asset_url.to_string();
        let mut remaining_redirects: u8 = 3;

        loop {
            let req = self
                .build_request(Method::GET, &url)?
                .header("Accept", "application/octet-stream")
                .body(Full::new(Bytes::new()))
                .map_err(ClientError::Http)?;

            let response = tokio::time::timeout(
                std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS),
                self.http.request(req),
            )
            .await
            .map_err(|_| ClientError::Timeout { url: url.clone() })??;

            let status = response.status();

            if status.is_redirection() {
                if remaining_redirects == 0 {
                    return Err(ClientError::ApiError {
                        status: status.as_u16(),
                        body: "too many redirects".to_string(),
                    });
                }
                remaining_redirects -= 1;
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| ClientError::ApiError {
                        status: status.as_u16(),
                        body: "redirect with no Location header".to_string(),
                    })?
                    .to_string();
                url = location;
                continue;
            }

            if !status.is_success() {
                let body = collect_body(response.into_body()).await?;
                return Err(ClientError::ApiError {
                    status: status.as_u16(),
                    body: String::from_utf8_lossy(&body).into_owned(),
                });
            }

            return collect_body(response.into_body()).await;
        }
    }
}
