// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub Actions workflow backup.
//!
//! Backs up the list of workflows defined in a repository and, optionally,
//! the run history for each workflow.

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, storage::Storage};

/// Backs up GitHub Actions workflow metadata for a single repository.
///
/// When `opts.actions` is enabled, writes `meta_dir/workflows.json` containing
/// an array of workflow objects (id, name, path, state, badge URL, …).
///
/// When `opts.action_runs` is *also* enabled, writes one additional file per
/// workflow: `meta_dir/workflow_runs_<id>.json`.  This can produce large files
/// for active repositories; opt in deliberately.
///
/// The actual YAML workflow files are already preserved by the git clone.
/// This function captures the API-level metadata which is not part of the
/// repository tree.
///
/// Admin access is **not** required — any token with `repo` scope (or a
/// fine-grained token with `actions:read`) can read workflow data.  When the
/// API returns 403 or 404 (Actions disabled or token lacks permissions) the
/// function logs a message and returns `Ok(())`.
///
/// # Errors
///
/// Propagates [`CoreError`] from storage writes or non-403/404 API errors.
pub async fn backup_actions(
    client: &impl BackupClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<u64, CoreError> {
    if !opts.actions {
        return Ok(0);
    }

    let workflows = match client.list_workflows(owner, repo_name).await {
        Ok(wf) => wf,
        Err(github_backup_client::ClientError::ApiError {
            status: 403 | 404, ..
        }) => {
            info!(
                repo = format!("{owner}/{repo_name}"),
                "skipping actions workflows (not available or insufficient permissions)"
            );
            return Ok(0);
        }
        Err(e) => return Err(e.into()),
    };

    let count = workflows.len() as u64;
    info!(
        owner,
        repo = repo_name,
        count,
        "backing up actions workflows"
    );
    storage.write_json(&meta_dir.join("workflows.json"), &workflows)?;

    // Optionally back up workflow run history.
    if opts.action_runs {
        for workflow in &workflows {
            let runs = match client
                .list_workflow_runs(owner, repo_name, workflow.id)
                .await
            {
                Ok(r) => r,
                Err(github_backup_client::ClientError::ApiError {
                    status: 403 | 404, ..
                }) => {
                    info!(
                        repo = format!("{owner}/{repo_name}"),
                        workflow_id = workflow.id,
                        "skipping workflow runs (not available)"
                    );
                    continue;
                }
                Err(e) => return Err(e.into()),
            };

            let filename = format!("workflow_runs_{}.json", workflow.id);
            storage.write_json(&meta_dir.join(&filename), &runs)?;
            info!(
                owner,
                repo = repo_name,
                workflow_id = workflow.id,
                runs = runs.len(),
                "saved workflow runs"
            );
        }
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::config::BackupOptions;
    use github_backup_types::workflow::Workflow;
    use std::path::PathBuf;

    fn make_workflow(id: u64, name: &str) -> Workflow {
        Workflow {
            id,
            name: name.to_string(),
            path: format!(".github/workflows/{name}.yml"),
            state: "active".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            url: format!("https://api.github.com/repos/octocat/repo/actions/workflows/{id}"),
            html_url: format!(
                "https://github.com/octocat/repo/blob/main/.github/workflows/{name}.yml"
            ),
            badge_url: format!("https://github.com/octocat/repo/workflows/{name}/badge.svg"),
        }
    }

    #[tokio::test]
    async fn backup_actions_disabled_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default(); // actions: false

        let count = backup_actions(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_actions");

        assert_eq!(count, 0);
        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_actions_enabled_writes_workflows_json() {
        let wf = make_workflow(161335, "ci");
        let client = MockBackupClient::new().with_workflows(vec![wf]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            actions: true,
            ..Default::default()
        };

        let count = backup_actions(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_actions");

        assert_eq!(count, 1);
        assert!(
            storage
                .get(&PathBuf::from("/meta/workflows.json"))
                .is_some(),
            "workflows.json should be written"
        );
    }

    #[tokio::test]
    async fn backup_actions_empty_list_writes_file() {
        let client = MockBackupClient::new(); // returns empty list
        let storage = MemStorage::default();
        let opts = BackupOptions {
            actions: true,
            ..Default::default()
        };

        let count = backup_actions(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_actions");

        assert_eq!(count, 0);
        assert!(storage
            .get(&PathBuf::from("/meta/workflows.json"))
            .is_some());
    }
}
