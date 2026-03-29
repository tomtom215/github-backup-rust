// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub Actions workflow and workflow-run types.

use serde::{Deserialize, Serialize};

/// A GitHub Actions workflow defined in a repository.
///
/// Workflows are YAML files stored under `.github/workflows/` that define
/// automated CI/CD pipelines.  This struct captures the metadata exposed by
/// the REST API; the actual YAML content is preserved in the git clone.
///
/// # API reference
///
/// `GET /repos/{owner}/{repo}/actions/workflows`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    /// Numeric workflow identifier.
    pub id: u64,
    /// Short name of the workflow (from the YAML `name:` field).
    pub name: String,
    /// Repository-relative path to the workflow YAML file.
    ///
    /// Example: `.github/workflows/ci.yml`
    pub path: String,
    /// Workflow lifecycle state (`"active"`, `"disabled_manually"`,
    /// `"disabled_inactivity"`, `"deleted"`, `"disabled_fork"`).
    pub state: String,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 last-update timestamp.
    pub updated_at: String,
    /// GitHub API URL for this workflow.
    pub url: String,
    /// GitHub HTML URL for viewing this workflow in a browser.
    pub html_url: String,
    /// URL of the workflow status badge image.
    pub badge_url: String,
}

/// A single execution of a GitHub Actions workflow.
///
/// Run records are useful for auditing automation history and diagnosing
/// failures.  Storing recent run metadata alongside the repository backup
/// makes it possible to review CI status without a live GitHub connection.
///
/// # API reference
///
/// `GET /repos/{owner}/{repo}/actions/workflows/{workflow_id}/runs`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRun {
    /// Numeric run identifier (unique across the repository).
    pub id: u64,
    /// Display name of the run (may differ from the workflow name).
    #[serde(default)]
    pub name: Option<String>,
    /// The workflow run's numeric ID within the workflow (`run_number`).
    pub run_number: u64,
    /// Git branch the run targeted (e.g. `"main"`).
    #[serde(default)]
    pub head_branch: Option<String>,
    /// Git SHA of the head commit.
    pub head_sha: String,
    /// The event that triggered this run (e.g. `"push"`, `"pull_request"`).
    pub event: String,
    /// Current status (`"queued"`, `"in_progress"`, `"completed"`).
    pub status: String,
    /// Conclusion once completed (`"success"`, `"failure"`, `"cancelled"`,
    /// `"skipped"`, `"timed_out"`, `"action_required"`, or `null`).
    #[serde(default)]
    pub conclusion: Option<String>,
    /// Numeric ID of the workflow this run belongs to.
    pub workflow_id: u64,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 last-update timestamp.
    pub updated_at: String,
    /// GitHub API URL for this run.
    pub url: String,
    /// GitHub HTML URL for viewing this run in a browser.
    pub html_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_deserialise_succeeds() {
        let json = serde_json::json!({
            "id": 161335,
            "name": "CI",
            "path": ".github/workflows/ci.yml",
            "state": "active",
            "created_at": "2020-01-08T23:48:37.000-08:00",
            "updated_at": "2020-01-08T23:50:21.000-08:00",
            "url": "https://api.github.com/repos/octo-org/octo-repo/actions/workflows/161335",
            "html_url": "https://github.com/octo-org/octo-repo/blob/master/.github/workflows/161335.yml",
            "badge_url": "https://github.com/octo-org/octo-repo/workflows/build/badge.svg"
        });

        let wf: Workflow = serde_json::from_value(json).expect("deserialise");
        assert_eq!(wf.id, 161335);
        assert_eq!(wf.name, "CI");
        assert_eq!(wf.state, "active");
    }

    #[test]
    fn workflow_run_deserialise_succeeds() {
        let json = serde_json::json!({
            "id": 30433642,
            "name": "Build",
            "run_number": 562,
            "head_branch": "main",
            "head_sha": "acb5820ced9479c074f688cc328bf03f341a511d",
            "event": "push",
            "status": "completed",
            "conclusion": "success",
            "workflow_id": 161335,
            "created_at": "2020-01-22T19:33:08Z",
            "updated_at": "2020-01-22T19:33:08Z",
            "url": "https://api.github.com/repos/octo-org/octo-repo/actions/runs/30433642",
            "html_url": "https://github.com/octo-org/octo-repo/actions/runs/30433642"
        });

        let run: WorkflowRun = serde_json::from_value(json).expect("deserialise");
        assert_eq!(run.id, 30433642);
        assert_eq!(run.run_number, 562);
        assert_eq!(run.conclusion.as_deref(), Some("success"));
    }

    #[test]
    fn workflow_run_handles_null_conclusion() {
        let json = serde_json::json!({
            "id": 1,
            "run_number": 1,
            "head_sha": "abc",
            "event": "push",
            "status": "in_progress",
            "conclusion": null,
            "workflow_id": 1,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "url": "https://api.github.com/repos/o/r/actions/runs/1",
            "html_url": "https://github.com/o/r/actions/runs/1"
        });

        let run: WorkflowRun = serde_json::from_value(json).expect("deserialise");
        assert!(run.conclusion.is_none());
        assert_eq!(run.status, "in_progress");
    }
}
