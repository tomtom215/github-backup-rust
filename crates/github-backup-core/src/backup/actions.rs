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

use crate::{
    backup::{all_json, describe_unparsed, json_u64, merge::merge_and_write, Outcome},
    error::CoreError,
    storage::Storage,
};

/// Backs up GitHub Actions workflow metadata for a single repository.
///
/// When `opts.actions` is enabled, writes `meta_dir/workflows.json` containing
/// an array of workflow objects (id, name, path, state, badge URL, …).
///
/// When `opts.action_runs` is *also* enabled, writes one additional file per
/// workflow: `meta_dir/workflow_runs_<id>.json`.  This can produce large files
/// for active repositories; opt in deliberately.
///
/// Both files **accumulate**: they are merged by `id` with what earlier runs
/// stored.  GitHub deletes workflow runs after the retention period and a
/// workflow disappears with its file, and that history is exactly what a
/// backup should keep.
///
/// The actual YAML workflow files are already preserved by the git clone.
/// This function captures the API-level metadata which is not part of the
/// repository tree.
///
/// A workflow whose runs cannot be fetched is recorded in
/// [`Outcome::failed`]; the other workflows are still backed up.
///
/// Returns the number of workflows.
///
/// # Errors
///
/// Propagates [`CoreError`] from the workflow listing (including a 403 or 404
/// answer, which the engine interprets) or storage writes, and any fatal
/// error.
pub async fn backup_actions(
    client: &impl BackupClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<Outcome, CoreError> {
    let mut outcome = Outcome::default();
    if !opts.actions {
        return Ok(outcome);
    }

    let workflows = client.list_workflows(owner, repo_name).await?;
    outcome.count = (workflows.len() + workflows.unparsed_count()) as u64;
    info!(
        owner,
        repo = repo_name,
        count = outcome.count,
        "backing up actions workflows"
    );
    merge_and_write(storage, &meta_dir.join("workflows.json"), &workflows, "id")?;

    if !opts.action_runs {
        return Ok(outcome);
    }

    for workflow in all_json(&workflows) {
        let Some(id) = json_u64(workflow, "id") else {
            outcome.fail(
                format!("workflow ({})", describe_unparsed(workflow)),
                "the API object has no `id`, so its runs cannot be fetched",
            );
            continue;
        };
        match client.list_workflow_runs(owner, repo_name, id).await {
            Ok(runs) => {
                let path = meta_dir.join(format!("workflow_runs_{id}.json"));
                merge_and_write(storage, &path, &runs, "id")?;
                info!(
                    owner,
                    repo = repo_name,
                    workflow_id = id,
                    runs = runs.len(),
                    "saved workflow runs"
                );
            }
            Err(e) => outcome.item_error(format!("workflow {id} runs"), e)?,
        }
    }

    Ok(outcome)
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
        .expect("backup_actions")
        .count;

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
        .expect("backup_actions")
        .count;

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
        .expect("backup_actions")
        .count;

        assert_eq!(count, 0);
        assert!(storage
            .get(&PathBuf::from("/meta/workflows.json"))
            .is_some());
    }
}
