// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub community features: Discussions, Classic Projects, and Packages.
//!
//! These endpoints require specific repository settings or token scopes:
//!
//! - **Discussions** – GitHub offers Discussions through GraphQL only; the
//!   REST route called here does not exist, so it answers 404.
//! - **Classic Projects** – sunset by GitHub; the REST routes answer 404/410.
//! - **Packages** – requires the `read:packages` OAuth scope.  Callers should
//!   handle 403/404 gracefully when the user has no packages or the token lacks
//!   the required scope.

use tracing::info;

use github_backup_types::{
    ClassicProject, Discussion, DiscussionComment, Package, PackageVersion, Page, ProjectColumn,
};

use crate::error::ClientError;

use super::super::{GitHubClient, PER_PAGE};

impl GitHubClient {
    // ── Discussions ───────────────────────────────────────────────────────

    /// Lists discussions for a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_discussions(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Page<Discussion>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/discussions?per_page={PER_PAGE}");
        let all = self.get_all_pages(&url).await?;
        info!(owner, repo, count = all.len(), "fetched discussions");
        Ok(all)
    }

    /// Lists comments on a specific discussion.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_discussion_comments(
        &self,
        owner: &str,
        repo: &str,
        discussion_number: u64,
    ) -> Result<Page<DiscussionComment>, ClientError> {
        let api = self.api();
        let url = format!(
            "{api}/repos/{owner}/{repo}/discussions/{discussion_number}/comments?per_page={PER_PAGE}"
        );
        let all = self.get_all_pages(&url).await?;
        info!(
            owner,
            repo,
            discussion_number,
            count = all.len(),
            "fetched discussion comments"
        );
        Ok(all)
    }

    // ── Classic Projects ──────────────────────────────────────────────────

    /// Lists classic (v1) projects for a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_repo_projects(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Page<ClassicProject>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/projects?per_page={PER_PAGE}&state=all");
        let all = self.get_all_pages(&url).await?;
        info!(owner, repo, count = all.len(), "fetched classic projects");
        Ok(all)
    }

    /// Lists columns in a classic project.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_project_columns(
        &self,
        project_id: u64,
    ) -> Result<Page<ProjectColumn>, ClientError> {
        let api = self.api();
        let url = format!("{api}/projects/{project_id}/columns?per_page={PER_PAGE}");
        let all = self.get_all_pages(&url).await?;
        info!(project_id, count = all.len(), "fetched project columns");
        Ok(all)
    }

    // ── GitHub Packages ───────────────────────────────────────────────────

    /// Lists packages published by a user.
    ///
    /// Requires the `read:packages` OAuth scope.  Callers should handle
    /// 403/404 gracefully.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_user_packages(
        &self,
        username: &str,
        package_type: &str,
    ) -> Result<Page<Package>, ClientError> {
        let api = self.api();
        let url = format!(
            "{api}/users/{username}/packages?package_type={package_type}&per_page={PER_PAGE}"
        );
        let all = self.get_all_pages(&url).await?;
        info!(
            username,
            package_type,
            count = all.len(),
            "fetched user packages"
        );
        Ok(all)
    }

    /// Lists versions of a specific package.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_package_versions(
        &self,
        username: &str,
        package_type: &str,
        package_name: &str,
    ) -> Result<Page<PackageVersion>, ClientError> {
        let api = self.api();
        let url = format!(
            "{api}/users/{username}/packages/{package_type}/{package_name}/versions?per_page={PER_PAGE}"
        );
        let all = self.get_all_pages(&url).await?;
        info!(
            username,
            package_type,
            package_name,
            count = all.len(),
            "fetched package versions"
        );
        Ok(all)
    }
}
