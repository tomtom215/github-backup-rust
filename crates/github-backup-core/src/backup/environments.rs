// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository deployment environment backup.

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, storage::Storage};

/// Backs up deployment environment configurations for a single repository.
///
/// When `opts.environments` is enabled, writes `meta_dir/environments.json`
/// containing an array of environment objects with their protection rules and
/// branch policies.
///
/// Environments model deployment targets such as `staging` or `production`.
/// Backing up their metadata makes it possible to audit and reproduce
/// deployment gate configurations without a live GitHub connection.
///
/// The file is a snapshot of the current configuration, replaced on every
/// run.  How a 404 (nothing to back up) or a 403 refusal is reported is
/// decided by the engine.
///
/// # Errors
///
/// Propagates [`CoreError`] from the API call or the storage write.
pub async fn backup_environments(
    client: &impl BackupClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<(), CoreError> {
    if !opts.environments {
        return Ok(());
    }

    let envs = client.list_environments(owner, repo_name).await?;
    info!(
        owner,
        repo = repo_name,
        count = envs.len(),
        "backed up deployment environments"
    );
    storage.write_json(&meta_dir.join("environments.json"), &envs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::config::BackupOptions;
    use github_backup_types::environment::{DeploymentBranchPolicy, Environment};
    use std::path::PathBuf;

    fn make_environment(id: u64, name: &str) -> Environment {
        Environment {
            id,
            node_id: format!("EN_{id}"),
            name: name.to_string(),
            url: format!("https://api.github.com/repos/octocat/repo/environments/{name}"),
            html_url: format!("https://github.com/octocat/repo/deployments/activity_log?environments_filter={name}"),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            protection_rules: vec![],
            deployment_branch_policy: Some(DeploymentBranchPolicy {
                protected_branches: true,
                custom_branch_policies: false,
            }),
        }
    }

    #[tokio::test]
    async fn backup_environments_disabled_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default(); // environments: false

        backup_environments(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_environments");

        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_environments_enabled_writes_json() {
        let env = make_environment(1, "production");
        let client = MockBackupClient::new().with_environments(vec![env]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            environments: true,
            ..Default::default()
        };

        backup_environments(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_environments");

        assert!(
            storage
                .get(&PathBuf::from("/meta/environments.json"))
                .is_some(),
            "environments.json should be written"
        );
    }

    #[tokio::test]
    async fn backup_environments_empty_list_writes_file() {
        let client = MockBackupClient::new(); // returns empty list
        let storage = MemStorage::default();
        let opts = BackupOptions {
            environments: true,
            ..Default::default()
        };

        backup_environments(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_environments");

        assert!(storage
            .get(&PathBuf::from("/meta/environments.json"))
            .is_some());
    }
}
