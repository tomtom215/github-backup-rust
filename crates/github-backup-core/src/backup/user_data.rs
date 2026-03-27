// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! User-level data backup: starred repos, watched repos, followers, following.

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, storage::Storage};

/// Backs up user-level relationship data for `username`.
///
/// Writes JSON files into `owner_json_dir`:
/// - `starred.json`   – repositories starred by the user
/// - `watched.json`   – repositories watched by the user
/// - `followers.json` – users who follow the user
/// - `following.json` – users the user follows
///
/// # Errors
///
/// Propagates [`CoreError`] from API calls or storage writes.
pub async fn backup_user_data(
    client: &impl BackupClient,
    username: &str,
    opts: &BackupOptions,
    owner_json_dir: &Path,
    storage: &impl Storage,
) -> Result<(), CoreError> {
    if opts.starred {
        info!(username, "fetching starred repositories");
        let starred = client.list_starred(username).await?;
        storage.write_json(&owner_json_dir.join("starred.json"), &starred)?;
    }

    if opts.watched {
        info!(username, "fetching watched repositories");
        let watched = client.list_watched(username).await?;
        storage.write_json(&owner_json_dir.join("watched.json"), &watched)?;
    }

    if opts.followers {
        info!(username, "fetching followers");
        let followers = client.list_followers(username).await?;
        storage.write_json(&owner_json_dir.join("followers.json"), &followers)?;
    }

    if opts.following {
        info!(username, "fetching following");
        let following = client.list_following(username).await?;
        storage.write_json(&owner_json_dir.join("following.json"), &following)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::config::BackupOptions;
    use std::path::PathBuf;

    const JSON_DIR: &str = "/json";

    #[tokio::test]
    async fn backup_user_data_all_disabled_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default(); // all false

        backup_user_data(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(JSON_DIR),
            &storage,
        )
        .await
        .expect("backup_user_data");

        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_user_data_starred_writes_starred_json() {
        let client = MockBackupClient::new(); // returns empty starred
        let storage = MemStorage::default();
        let opts = BackupOptions {
            starred: true,
            ..Default::default()
        };

        backup_user_data(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(JSON_DIR),
            &storage,
        )
        .await
        .expect("backup_user_data");

        assert!(storage
            .get(&PathBuf::from(format!("{JSON_DIR}/starred.json")))
            .is_some());
        // watched/followers/following should NOT be written
        assert!(storage
            .get(&PathBuf::from(format!("{JSON_DIR}/watched.json")))
            .is_none());
    }

    #[tokio::test]
    async fn backup_user_data_watched_writes_watched_json() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions {
            watched: true,
            ..Default::default()
        };

        backup_user_data(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(JSON_DIR),
            &storage,
        )
        .await
        .expect("backup_user_data");

        assert!(storage
            .get(&PathBuf::from(format!("{JSON_DIR}/watched.json")))
            .is_some());
    }

    #[tokio::test]
    async fn backup_user_data_followers_and_following_written_independently() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions {
            followers: true,
            following: true,
            ..Default::default()
        };

        backup_user_data(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(JSON_DIR),
            &storage,
        )
        .await
        .expect("backup_user_data");

        assert!(storage
            .get(&PathBuf::from(format!("{JSON_DIR}/followers.json")))
            .is_some());
        assert!(storage
            .get(&PathBuf::from(format!("{JSON_DIR}/following.json")))
            .is_some());
        // starred/watched should NOT be written
        assert!(storage
            .get(&PathBuf::from(format!("{JSON_DIR}/starred.json")))
            .is_none());
    }

    #[tokio::test]
    async fn backup_user_data_all_enabled_writes_four_files() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions {
            starred: true,
            watched: true,
            followers: true,
            following: true,
            ..Default::default()
        };

        backup_user_data(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(JSON_DIR),
            &storage,
        )
        .await
        .expect("backup_user_data");

        assert_eq!(
            storage.len(),
            4,
            "all four user-data files should be written"
        );
    }
}
