// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository label backup.
//!
//! Saves `labels.json` containing all labels defined on the repository.

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, storage::Storage};

/// Backs up repository labels to `meta_dir/labels.json`.
///
/// Skipped when `opts.labels` is `false` or `opts.dry_run` is `true`.
///
/// # Errors
///
/// Propagates [`CoreError`] from API calls or storage writes.
pub async fn backup_labels(
    client: &impl BackupClient,
    owner: &str,
    repo: &str,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<(), CoreError> {
    if !opts.labels || opts.dry_run {
        return Ok(());
    }

    info!(owner, repo, "fetching labels");
    let labels = client.list_labels(owner, repo).await?;
    storage.write_json(&meta_dir.join("labels.json"), &labels)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::Label;
    use std::path::PathBuf;

    const META: &str = "/meta";

    #[tokio::test]
    async fn backup_labels_disabled_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default();

        backup_labels(
            &client,
            "owner",
            "repo",
            &opts,
            &PathBuf::from(META),
            &storage,
        )
        .await
        .expect("ok");

        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_labels_dry_run_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions {
            labels: true,
            dry_run: true,
            ..Default::default()
        };

        backup_labels(
            &client,
            "owner",
            "repo",
            &opts,
            &PathBuf::from(META),
            &storage,
        )
        .await
        .expect("ok");

        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_labels_enabled_writes_json() {
        let label = Label {
            id: 1,
            name: "bug".to_string(),
            color: "d73a4a".to_string(),
            description: None,
            default: true,
        };
        let client = MockBackupClient::new().with_labels(vec![label]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            labels: true,
            ..Default::default()
        };

        backup_labels(
            &client,
            "owner",
            "repo",
            &opts,
            &PathBuf::from(META),
            &storage,
        )
        .await
        .expect("ok");

        assert!(storage
            .get(&PathBuf::from(format!("{META}/labels.json")))
            .is_some());
    }
}
