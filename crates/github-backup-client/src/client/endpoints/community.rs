// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub community features: Discussions, Classic Projects, and Packages.
//!
//! These endpoints require specific repository settings or token scopes:
//!
//! - **Discussions** – the repository must have Discussions enabled; otherwise
//!   the API returns 404.  Callers should handle 404 gracefully.
//! - **Classic Projects** – the Projects feature must be enabled on the repo.
//!   The API returns 404 when the feature is disabled.
//! - **Packages** – requires the `read:packages` OAuth scope.  Callers should
//!   handle 403/404 gracefully when the user has no packages or the token lacks
//!   the required scope.

use tracing::info;

use github_backup_types::{
    ClassicProject, Discussion, DiscussionComment, Package, PackageVersion, ProjectColumn,
};

use crate::error::ClientError;

use super::super::{GitHubClient, PER_PAGE};

impl GitHubClient {
    // ── Discussions ───────────────────────────────────────────────────────

    /// Lists discussions for a repository.
    ///
    /// GitHub Discussions must be enabled on the repository.  If the feature
    /// is disabled the API returns 404; callers should handle this gracefully.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_discussions(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<Discussion>, ClientError> {
        let api = self.api();
        let mut url = format!("{api}/repos/{owner}/{repo}/discussions?per_page={PER_PAGE}");
        let mut all: Vec<Discussion> = Vec::new();

        loop {
            let (page, link) = self.get_json_with_link::<Vec<Discussion>>(&url).await?;
            all.extend(page);
            match link.as_deref().and_then(crate::pagination::parse_next_link) {
                Some(next) => url = next,
                None => break,
            }
        }

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
    ) -> Result<Vec<DiscussionComment>, ClientError> {
        let api = self.api();
        let mut url = format!(
            "{api}/repos/{owner}/{repo}/discussions/{discussion_number}/comments?per_page={PER_PAGE}"
        );
        let mut all: Vec<DiscussionComment> = Vec::new();

        loop {
            let (page, link) = self
                .get_json_with_link::<Vec<DiscussionComment>>(&url)
                .await?;
            all.extend(page);
            match link.as_deref().and_then(crate::pagination::parse_next_link) {
                Some(next) => url = next,
                None => break,
            }
        }

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
    /// Classic Projects must be enabled on the repository; if not, the API
    /// returns 404.  Callers should handle 404 gracefully.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_repo_projects(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<ClassicProject>, ClientError> {
        let api = self.api();
        let mut url = format!("{api}/repos/{owner}/{repo}/projects?per_page={PER_PAGE}&state=all");
        let mut all: Vec<ClassicProject> = Vec::new();

        loop {
            let (page, link) = self.get_json_with_link::<Vec<ClassicProject>>(&url).await?;
            all.extend(page);
            match link.as_deref().and_then(crate::pagination::parse_next_link) {
                Some(next) => url = next,
                None => break,
            }
        }

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
    ) -> Result<Vec<ProjectColumn>, ClientError> {
        let api = self.api();
        let mut url = format!("{api}/projects/{project_id}/columns?per_page={PER_PAGE}");
        let mut all: Vec<ProjectColumn> = Vec::new();

        loop {
            let (page, link) = self.get_json_with_link::<Vec<ProjectColumn>>(&url).await?;
            all.extend(page);
            match link.as_deref().and_then(crate::pagination::parse_next_link) {
                Some(next) => url = next,
                None => break,
            }
        }

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
    ) -> Result<Vec<Package>, ClientError> {
        let api = self.api();
        let mut url = format!(
            "{api}/users/{username}/packages?package_type={package_type}&per_page={PER_PAGE}"
        );
        let mut all: Vec<Package> = Vec::new();

        loop {
            let (page, link) = self.get_json_with_link::<Vec<Package>>(&url).await?;
            all.extend(page);
            match link.as_deref().and_then(crate::pagination::parse_next_link) {
                Some(next) => url = next,
                None => break,
            }
        }

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
    ) -> Result<Vec<PackageVersion>, ClientError> {
        let api = self.api();
        let mut url = format!(
            "{api}/users/{username}/packages/{package_type}/{package_name}/versions?per_page={PER_PAGE}"
        );
        let mut all: Vec<PackageVersion> = Vec::new();

        loop {
            let (page, link) = self.get_json_with_link::<Vec<PackageVersion>>(&url).await?;
            all.extend(page);
            match link.as_deref().and_then(crate::pagination::parse_next_link) {
                Some(next) => url = next,
                None => break,
            }
        }

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
