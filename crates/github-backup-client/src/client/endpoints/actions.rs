// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub Actions and deployment environment listing endpoints.
//!
//! Covers workflow metadata, workflow run history, and deployment environment
//! configurations for a repository.

use tracing::info;

use github_backup_types::{Environment, Workflow, WorkflowRun};

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
    ) -> Result<Vec<Workflow>, ClientError> {
        // The API wraps the array under {"total_count": N, "workflows": [...]}
        #[derive(serde::Deserialize)]
        struct WorkflowsResponse {
            workflows: Vec<Workflow>,
        }

        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/actions/workflows?per_page={PER_PAGE}");
        let (resp, _) = self.get_json_with_link::<WorkflowsResponse>(&url).await?;
        info!(
            owner,
            repo,
            count = resp.workflows.len(),
            "fetched workflows"
        );
        Ok(resp.workflows)
    }

    /// Lists workflow runs for a specific workflow.
    ///
    /// Returns the most recent runs (paginated).  Callers should handle
    /// 403/404 gracefully.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_workflow_runs(
        &self,
        owner: &str,
        repo: &str,
        workflow_id: u64,
    ) -> Result<Vec<WorkflowRun>, ClientError> {
        // The API wraps runs under {"total_count": N, "workflow_runs": [...]}
        #[derive(serde::Deserialize)]
        struct RunsResponse {
            workflow_runs: Vec<WorkflowRun>,
        }

        let api = self.api();
        let url = format!(
            "{api}/repos/{owner}/{repo}/actions/workflows/{workflow_id}/runs?per_page={PER_PAGE}"
        );
        let (resp, _) = self.get_json_with_link::<RunsResponse>(&url).await?;
        info!(
            owner,
            repo,
            workflow_id,
            count = resp.workflow_runs.len(),
            "fetched workflow runs"
        );
        Ok(resp.workflow_runs)
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
    ) -> Result<Vec<Environment>, ClientError> {
        // The API wraps environments under {"total_count": N, "environments": [...]}
        #[derive(serde::Deserialize)]
        struct EnvsResponse {
            environments: Vec<Environment>,
        }

        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/environments?per_page={PER_PAGE}");
        let (resp, _) = self.get_json_with_link::<EnvsResponse>(&url).await?;
        info!(
            owner,
            repo,
            count = resp.environments.len(),
            "fetched environments"
        );
        Ok(resp.environments)
    }
}
