// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository topic backup.
//!
//! Saves `topics.json` containing the list of topics assigned to the
//! repository.  403/404 responses (private repo without access, or feature not
//! enabled) are silently skipped rather than treated as errors.

use std::path::Path;

use tracing::info;

use github_backup_client::{BackupClient, ClientError};
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, storage::Storage};

/// Backs up repository topics to `meta_dir/topics.json`.
///
/// Skipped when `opts.topics` is `false` or `opts.dry_run` is `true`.
///
/// When the API returns 403 or 404 the error is logged as informational and
/// the function returns `Ok(())`.
///
/// # Errors
///
/// Propagates [`CoreError`] for all other API or storage errors.
pub async fn backup_topics(
    client: &impl BackupClient,
    owner: &str,
    repo: &str,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<(), CoreError> {
    if !opts.topics || opts.dry_run {
        return Ok(());
    }

    info!(owner, repo, "fetching topics");
    match client.list_repo_topics(owner, repo).await {
        Ok(topics) => {
            storage.write_json(&meta_dir.join("topics.json"), &topics)?;
        }
        Err(ClientError::ApiError {
            status: 403 | 404, ..
        }) => {
            info!(owner, repo, "skipping topics (not available)");
        }
        Err(e) => return Err(e.into()),
    }
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
    async fn backup_topics_disabled_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default();

        backup_topics(
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
    async fn backup_topics_dry_run_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions {
            topics: true,
            dry_run: true,
            ..Default::default()
        };

        backup_topics(
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
    async fn backup_topics_enabled_writes_json() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions {
            topics: true,
            ..Default::default()
        };

        backup_topics(
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
            .get(&PathBuf::from(format!("{META}/topics.json")))
            .is_some());
    }
}
