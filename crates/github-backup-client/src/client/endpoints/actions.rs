// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub Actions and deployment environment listing endpoints.
//!
//! Covers workflow metadata, workflow run history, and deployment environment
//! configurations for a repository.  All three endpoints wrap their list in an
//! object (`{"total_count": n, "<key>": [...]}`) and paginate through the
//! `Link` header like every other list; every page is followed and merged.

use tracing::info;

use github_backup_types::{Environment, Page, Workflow, WorkflowRun};

use crate::error::ClientError;

use super::super::{GitHubClient, PER_PAGE};

impl GitHubClient {
    // ── GitHub Actions ────────────────────────────────────────────────────

    /// Lists GitHub Actions workflows defined in a repository.
    ///
    /// Returns workflow metadata (ID, name, path, state, badge URL, …).
    /// The actual YAML content is captured by the git clone.
    ///
    /// Requires the token to have `actions:read` permission (or the repository
    /// to have Actions enabled).  Callers should handle 403/404 gracefully.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_workflows(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Page<Workflow>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/actions/workflows?per_page={PER_PAGE}");
        let workflows = self.get_all_wrapped_pages(&url, "workflows").await?;
        info!(owner, repo, count = workflows.len(), "fetched workflows");
        Ok(workflows)
    }

    /// Lists every run of a specific workflow, following all pages.
    ///
    /// Callers should handle 403/404 gracefully.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_workflow_runs(
        &self,
        owner: &str,
        repo: &str,
        workflow_id: u64,
    ) -> Result<Page<WorkflowRun>, ClientError> {
        let api = self.api();
        let url = format!(
            "{api}/repos/{owner}/{repo}/actions/workflows/{workflow_id}/runs?per_page={PER_PAGE}"
        );
        let runs = self.get_all_wrapped_pages(&url, "workflow_runs").await?;
        info!(
            owner,
            repo,
            workflow_id,
            count = runs.len(),
            "fetched workflow runs"
        );
        Ok(runs)
    }

    // ── Deployment environments ───────────────────────────────────────────

    /// Lists deployment environments configured on a repository.
    ///
    /// Environments model deployment targets such as `staging` or `production`
    /// and may have protection rules and branch policies.
    ///
    /// Callers should handle 403/404 gracefully (not all repositories have
    /// environments configured, and the API returns 404 in that case).
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_environments(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Page<Environment>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/environments?per_page={PER_PAGE}");
        let envs = self.get_all_wrapped_pages(&url, "environments").await?;
        info!(owner, repo, count = envs.len(), "fetched environments");
        Ok(envs)
    }
}
