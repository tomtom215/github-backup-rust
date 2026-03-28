// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Gist metadata and git clone backup.

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{
    error::CoreError,
    git::{CloneOptions, GitRunner},
    storage::Storage,
};

/// Backs up gists owned by `username` and optionally starred gists.
///
/// For each owned gist:
/// - Writes `gists_meta_dir/<id>.json` with gist metadata.
/// - Clones `gists_git_dir/<id>.git` as a bare mirror.
///
/// Returns the total number of gists backed up (owned + starred).
///
/// # Errors
///
/// Propagates [`CoreError`] from API calls, storage writes, or git operations.
pub async fn backup_gists(
    client: &impl BackupClient,
    username: &str,
    opts: &BackupOptions,
    gists_git_dir: &Path,
    gists_meta_dir: &Path,
    storage: &impl Storage,
    git: &impl GitRunner,
    clone_opts: &CloneOptions,
) -> Result<u64, CoreError> {
    if !opts.gists && !opts.starred_gists {
        return Ok(0);
    }

    if opts.dry_run {
        info!(username, "dry-run: skipping gist backup");
        return Ok(0);
    }

    let mut count = 0u64;

    if opts.gists {
        info!(username, "fetching gists");
        let gists = client.list_gists(username).await?;
        for gist in &gists {
            let meta_path = gists_meta_dir.join(format!("{}.json", gist.id));
            storage.write_json(&meta_path, gist)?;

            let dest = gists_git_dir.join(format!("{}.git", gist.id));
            git.mirror_clone(&gist.git_pull_url, &dest, clone_opts)?;
            count += 1;
        }
        storage.write_json(&gists_meta_dir.join("index.json"), &gists)?;
    }

    if opts.starred_gists {
        // NOTE: /gists/starred returns gists starred by the *authenticated user*,
        // not the `username` argument being backed up. This is correct behaviour
        // for a backup tool but differs from other user-scoped calls.
        info!("fetching starred gists for authenticated user");
        let starred = client.list_starred_gists().await?;
        for gist in &starred {
            let meta_path = gists_meta_dir.join(format!("{}.starred.json", gist.id));
            storage.write_json(&meta_path, gist)?;
            count += 1;
        }
        storage.write_json(&gists_meta_dir.join("starred_index.json"), &starred)?;
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::git::test_support::SpyGitRunner;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::config::BackupOptions;
    use std::path::PathBuf;

    const GIST_DIR: &str = "/git/gists";
    const META_DIR: &str = "/json/gists";

    #[tokio::test]
    async fn backup_gists_disabled_returns_zero_and_no_io() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let git = SpyGitRunner::default();
        let opts = BackupOptions::default(); // gists = false, starred_gists = false

        let count = backup_gists(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(GIST_DIR),
            &PathBuf::from(META_DIR),
            &storage,
            &git,
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("backup_gists");

        assert_eq!(count, 0);
        assert_eq!(git.recorded_calls().len(), 0, "no git calls expected");
        assert_eq!(storage.len(), 0, "no storage writes expected");
    }

    #[tokio::test]
    async fn backup_gists_empty_list_writes_index_only() {
        // MockBackupClient returns empty gists by default
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let git = SpyGitRunner::default();
        let opts = BackupOptions {
            gists: true,
            ..Default::default()
        };

        let count = backup_gists(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(GIST_DIR),
            &PathBuf::from(META_DIR),
            &storage,
            &git,
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("backup_gists");

        assert_eq!(count, 0);
        assert!(
            storage
                .get(&PathBuf::from(format!("{META_DIR}/index.json")))
                .is_some(),
            "index.json should be written even for empty list"
        );
        assert_eq!(git.recorded_calls().len(), 0);
    }

    #[tokio::test]
    async fn backup_starred_gists_disabled_skips() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let git = SpyGitRunner::default();
        let opts = BackupOptions {
            gists: false,
            starred_gists: false,
            ..Default::default()
        };

        let count = backup_gists(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(GIST_DIR),
            &PathBuf::from(META_DIR),
            &storage,
            &git,
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("backup_gists");

        assert_eq!(count, 0);
        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_gists_dry_run_returns_zero_and_no_io() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let git = SpyGitRunner::default();
        let opts = BackupOptions {
            gists: true,
            starred_gists: true,
            dry_run: true,
            ..Default::default()
        };

        let count = backup_gists(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(GIST_DIR),
            &PathBuf::from(META_DIR),
            &storage,
            &git,
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("backup_gists dry_run");

        assert_eq!(count, 0, "dry-run must return 0");
        assert_eq!(
            git.recorded_calls().len(),
            0,
            "dry-run must make no git calls"
        );
        assert_eq!(storage.len(), 0, "dry-run must write nothing");
    }

    #[tokio::test]
    async fn backup_starred_gists_only_writes_starred_index() {
        let client = MockBackupClient::new(); // starred_gists = empty
        let storage = MemStorage::default();
        let git = SpyGitRunner::default();
        let opts = BackupOptions {
            gists: false,
            starred_gists: true,
            ..Default::default()
        };

        backup_gists(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(GIST_DIR),
            &PathBuf::from(META_DIR),
            &storage,
            &git,
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("backup_gists");

        assert!(
            storage
                .get(&PathBuf::from(format!("{META_DIR}/starred_index.json")))
                .is_some(),
            "starred_index.json should be written"
        );
        // No index.json when only starred_gists is enabled
        assert!(
            storage
                .get(&PathBuf::from(format!("{META_DIR}/index.json")))
                .is_none(),
            "index.json should not be written when only starred_gists is enabled"
        );
    }
}
