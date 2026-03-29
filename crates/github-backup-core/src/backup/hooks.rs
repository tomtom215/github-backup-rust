// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository webhook configuration backup.
//!
//! Saves `hooks.json` containing webhook configurations for the repository.
//! Requires admin access; 403/404 responses are treated as "no access" and
//! silently skipped rather than treated as errors.

use std::path::Path;

use tracing::info;

use github_backup_client::{BackupClient, ClientError};
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, storage::Storage};

/// Backs up repository webhooks to `meta_dir/hooks.json`.
///
/// Skipped when `opts.hooks` is `false` or `opts.dry_run` is `true`.
///
/// When the API returns 403 or 404 (missing admin access or hooks not enabled)
/// the error is logged as informational and the function returns `Ok(())`.
///
/// # Errors
///
/// Propagates [`CoreError`] for all other API or storage errors.
pub async fn backup_hooks(
    client: &impl BackupClient,
    owner: &str,
    repo: &str,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<(), CoreError> {
    if !opts.hooks || opts.dry_run {
        return Ok(());
    }

    info!(owner, repo, "fetching webhooks");
    match client.list_hooks(owner, repo).await {
        Ok(hooks) => {
            storage.write_json(&meta_dir.join("hooks.json"), &hooks)?;
        }
        Err(ClientError::ApiError {
            status: 403 | 404, ..
        }) => {
            info!(owner, repo, "skipping hooks (no admin access)");
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
    async fn backup_hooks_disabled_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default();

        backup_hooks(
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
    async fn backup_hooks_dry_run_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions {
            hooks: true,
            dry_run: true,
            ..Default::default()
        };

        backup_hooks(
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
    async fn backup_hooks_enabled_writes_json() {
        let client = MockBackupClient::new(); // default: empty hooks list
        let storage = MemStorage::default();
        let opts = BackupOptions {
            hooks: true,
            ..Default::default()
        };

        backup_hooks(
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
            .get(&PathBuf::from(format!("{META}/hooks.json")))
            .is_some());
    }
}
