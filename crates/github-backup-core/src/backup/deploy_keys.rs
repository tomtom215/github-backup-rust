// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository deploy key backup.

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, storage::Storage};

/// Backs up the deploy keys configured on a single repository.
///
/// Writes `meta_dir/deploy_keys.json` when `opts.deploy_keys` is enabled: a
/// snapshot of the keys configured now, replaced on every run (a key that was
/// removed must not look as if it were still installed).
///
/// Admin access to the repository is required by the GitHub API; how a
/// refusal is reported is decided by the engine.
///
/// # Errors
///
/// Propagates [`CoreError`] from the API call (including a 403 refusal) or
/// the storage write.
pub async fn backup_deploy_keys(
    client: &impl BackupClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<(), CoreError> {
    if !opts.deploy_keys {
        return Ok(());
    }

    let keys = client.list_deploy_keys(owner, repo_name).await?;
    info!(
        owner,
        repo = repo_name,
        count = keys.len(),
        "fetched deploy keys"
    );
    storage.write_json(&meta_dir.join("deploy_keys.json"), &keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::config::BackupOptions;
    use github_backup_types::deploy_key::DeployKey;
    use std::path::PathBuf;

    fn make_deploy_key(id: u64) -> DeployKey {
        DeployKey {
            id,
            key: format!("ssh-rsa AAAA...{id}"),
            url: format!("https://api.github.com/repos/octocat/repo/keys/{id}"),
            title: format!("key-{id}"),
            verified: true,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            read_only: true,
            added_by: None,
            last_used: None,
        }
    }

    #[tokio::test]
    async fn backup_deploy_keys_disabled_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default(); // deploy_keys: false

        backup_deploy_keys(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_deploy_keys");

        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_deploy_keys_enabled_writes_json() {
        let key = make_deploy_key(1);
        let client = MockBackupClient::new().with_deploy_keys(vec![key]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            deploy_keys: true,
            ..Default::default()
        };

        backup_deploy_keys(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_deploy_keys");

        assert!(
            storage
                .get(&PathBuf::from("/meta/deploy_keys.json"))
                .is_some(),
            "deploy_keys.json should be written"
        );
    }

    #[tokio::test]
    async fn backup_deploy_keys_empty_list_still_writes_file() {
        let client = MockBackupClient::new(); // returns empty list
        let storage = MemStorage::default();
        let opts = BackupOptions {
            deploy_keys: true,
            ..Default::default()
        };

        backup_deploy_keys(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_deploy_keys");

        assert!(storage
            .get(&PathBuf::from("/meta/deploy_keys.json"))
            .is_some());
    }
}
