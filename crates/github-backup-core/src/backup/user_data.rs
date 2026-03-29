// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! User-level and org-level owner data backup.
//!
//! Handles starred repos, watched repos, followers, following (all targets),
//! and org members / org teams (org targets only).

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
use github_backup_types::config::{BackupOptions, BackupTarget};

use crate::{error::CoreError, storage::Storage};

/// Backs up owner-level relationship and social data.
///
/// Writes JSON files into `owner_json_dir`:
///
/// For **all** targets:
/// - `starred.json`   – repositories starred by the owner
/// - `watched.json`   – repositories watched by the owner
/// - `followers.json` – users who follow the owner
/// - `following.json` – users the owner follows
///
/// For **organisation** targets only (when `opts.target == BackupTarget::Org`):
/// - `org_members.json` – organisation member list
/// - `org_teams.json`   – organisation team list
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
    if opts.dry_run {
        info!(username, "dry-run: skipping owner data backup");
        return Ok(());
    }

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

    // Org-specific data — only fetched when target is an organisation.
    if opts.target == BackupTarget::Org {
        if opts.org_members {
            info!(org = username, "fetching org members");
            let members = client.list_org_members(username).await?;
            storage.write_json(&owner_json_dir.join("org_members.json"), &members)?;
        }

        if opts.org_teams {
            info!(org = username, "fetching org teams");
            let teams = client.list_org_teams(username).await?;
            storage.write_json(&owner_json_dir.join("org_teams.json"), &teams)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::config::{BackupOptions, BackupTarget};
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
    async fn backup_user_data_dry_run_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions {
            starred: true,
            watched: true,
            followers: true,
            following: true,
            dry_run: true,
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
        .expect("backup_user_data dry_run");

        assert_eq!(storage.len(), 0, "dry-run must write nothing");
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

    #[tokio::test]
    async fn backup_user_data_org_members_written_for_org_target() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions {
            target: BackupTarget::Org,
            org_members: true,
            ..Default::default()
        };

        backup_user_data(&client, "my-org", &opts, &PathBuf::from(JSON_DIR), &storage)
            .await
            .expect("backup_user_data org_members");

        assert!(
            storage
                .get(&PathBuf::from(format!("{JSON_DIR}/org_members.json")))
                .is_some(),
            "org_members.json should be written for org target"
        );
    }

    #[tokio::test]
    async fn backup_user_data_org_members_not_written_for_user_target() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions {
            target: BackupTarget::User,
            org_members: true, // flag set but target is User
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

        assert!(
            storage
                .get(&PathBuf::from(format!("{JSON_DIR}/org_members.json")))
                .is_none(),
            "org_members.json must not be written for a user target"
        );
    }

    #[tokio::test]
    async fn backup_user_data_org_teams_written_for_org_target() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions {
            target: BackupTarget::Org,
            org_teams: true,
            ..Default::default()
        };

        backup_user_data(&client, "my-org", &opts, &PathBuf::from(JSON_DIR), &storage)
            .await
            .expect("backup_user_data org_teams");

        assert!(
            storage
                .get(&PathBuf::from(format!("{JSON_DIR}/org_teams.json")))
                .is_some(),
            "org_teams.json should be written for org target"
        );
    }
}
