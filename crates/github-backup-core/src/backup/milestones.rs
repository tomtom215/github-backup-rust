// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository milestone backup.
//!
//! Saves `milestones.json` containing all milestones (open and closed) defined
//! on the repository.

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, storage::Storage};

/// Backs up repository milestones to `meta_dir/milestones.json`.
///
/// Skipped when `opts.milestones` is `false` or `opts.dry_run` is `true`.
///
/// # Errors
///
/// Propagates [`CoreError`] from API calls or storage writes.
pub async fn backup_milestones(
    client: &impl BackupClient,
    owner: &str,
    repo: &str,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<(), CoreError> {
    if !opts.milestones || opts.dry_run {
        return Ok(());
    }

    info!(owner, repo, "fetching milestones");
    let milestones = client.list_milestones(owner, repo).await?;
    storage.write_json(&meta_dir.join("milestones.json"), &milestones)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use std::path::PathBuf;

    const META: &str = "/meta";

    #[tokio::test]
    async fn backup_milestones_disabled_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default();

        backup_milestones(
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
    async fn backup_milestones_dry_run_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions {
            milestones: true,
            dry_run: true,
            ..Default::default()
        };

        backup_milestones(
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
    async fn backup_milestones_enabled_writes_json() {
        let client = MockBackupClient::new(); // empty milestones list
        let storage = MemStorage::default();
        let opts = BackupOptions {
            milestones: true,
            ..Default::default()
        };

        backup_milestones(
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
            .get(&PathBuf::from(format!("{META}/milestones.json")))
            .is_some());
    }
}
