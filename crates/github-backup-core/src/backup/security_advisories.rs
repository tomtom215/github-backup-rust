// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository security-advisory backup.
//!
//! Saves `security_advisories.json` containing all security advisories for
//! the repository.  Requires admin access or the repository must have
//! vulnerability alerts enabled; 403/404 responses are treated as "no access"
//! and silently skipped rather than treated as errors.

use std::path::Path;

use tracing::info;

use github_backup_client::{BackupClient, ClientError};
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, storage::Storage};

/// Backs up repository security advisories to `meta_dir/security_advisories.json`.
///
/// Skipped when `opts.security_advisories` is `false` or `opts.dry_run` is `true`.
///
/// When the API returns 403 or 404 (missing admin access or advisories not enabled)
/// the error is logged as informational and the function returns `Ok(())`.
///
/// # Errors
///
/// Propagates [`CoreError`] for all other API or storage errors.
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
    match client.list_security_advisories(owner, repo).await {
        Ok(advisories) => {
            storage.write_json(&meta_dir.join("security_advisories.json"), &advisories)?;
        }
        Err(ClientError::ApiError {
            status: 403 | 404, ..
        }) => {
            info!(owner, repo, "skipping security advisories (not available)");
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
