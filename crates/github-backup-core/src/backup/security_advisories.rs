// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository security-advisory backup.
//!
//! Saves `security_advisories.json`: the repository's security advisories,
//! merged by `ghsa_id` with what earlier runs stored, so an advisory that is
//! withdrawn or deleted stays in the backup.  Unpublished advisories are only
//! visible to security managers and administrators.

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{backup::merge::merge_and_write, error::CoreError, storage::Storage};

/// Backs up repository security advisories to `meta_dir/security_advisories.json`.
///
/// Skipped when `opts.security_advisories` is `false` or `opts.dry_run` is `true`.
///
/// # Errors
///
/// Propagates [`CoreError`] from the API call (including a 403 refusal) or
/// the storage write.
pub async fn backup_security_advisories(
    client: &impl BackupClient,
    owner: &str,
    repo: &str,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<(), CoreError> {
    if !opts.security_advisories || opts.dry_run {
        return Ok(());
    }

    info!(owner, repo, "fetching security advisories");
    let advisories = client.list_security_advisories(owner, repo).await?;
    merge_and_write(
        storage,
        &meta_dir.join("security_advisories.json"),
        &advisories,
        "ghsa_id",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use std::path::PathBuf;

    const META: &str = "/meta";

    #[tokio::test]
    async fn backup_security_advisories_disabled_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default();

        backup_security_advisories(
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
    async fn backup_security_advisories_dry_run_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions {
            security_advisories: true,
            dry_run: true,
            ..Default::default()
        };

        backup_security_advisories(
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
    async fn backup_security_advisories_enabled_writes_json() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions {
            security_advisories: true,
            ..Default::default()
        };

        backup_security_advisories(
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
            .get(&PathBuf::from(format!("{META}/security_advisories.json")))
            .is_some());
    }
}
