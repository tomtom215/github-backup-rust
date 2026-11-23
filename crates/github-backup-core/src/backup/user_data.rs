// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! User-level and org-level owner data backup.
//!
//! Handles starred repos, watched repos, followers, following (all targets),
//! and org members / org teams (org targets only).

use std::path::Path;

use serde::Serialize;
use tracing::info;

use github_backup_client::{BackupClient, ClientError};
use github_backup_types::config::{BackupOptions, BackupTarget};

use crate::{backup::Outcome, error::CoreError, storage::Storage};

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
/// Each file is a snapshot of the current relationship (who follows the
/// owner *now*), replaced on every run.  Each list is independent: one that
/// cannot be fetched is recorded in [`Outcome::failed`] and the others are
/// still backed up.
///
/// # Errors
///
/// Propagates storage errors and any fatal error.
pub async fn backup_user_data(
    client: &impl BackupClient,
    username: &str,
    opts: &BackupOptions,
    owner_json_dir: &Path,
    storage: &impl Storage,
) -> Result<Outcome, CoreError> {
    let mut outcome = Outcome::default();
    if opts.dry_run {
        info!(username, "dry-run: skipping owner data backup");
        return Ok(outcome);
    }
    let org = opts.target == BackupTarget::Org;
    let save = |file: &str| owner_json_dir.join(file);

    if opts.starred {
        info!(username, "fetching starred repositories");
        let result = client.list_starred(username).await;
        store(
            storage,
            &save("starred.json"),
            "starred repositories",
            result,
            &mut outcome,
        )?;
    }
    if opts.watched {
        info!(username, "fetching watched repositories");
        let result = client.list_watched(username).await;
        store(
            storage,
            &save("watched.json"),
            "watched repositories",
            result,
            &mut outcome,
        )?;
    }
    if opts.followers {
        info!(username, "fetching followers");
        let result = client.list_followers(username).await;
        store(
            storage,
            &save("followers.json"),
            "followers",
            result,
            &mut outcome,
        )?;
    }
    if opts.following {
        info!(username, "fetching following");
        let result = client.list_following(username).await;
        store(
            storage,
            &save("following.json"),
            "following",
            result,
            &mut outcome,
        )?;
    }
    if org && opts.org_members {
        info!(org = username, "fetching org members");
        let result = client.list_org_members(username).await;
        store(
            storage,
            &save("org_members.json"),
            "organisation members",
            result,
            &mut outcome,
        )?;
    }
    if org && opts.org_teams {
        info!(org = username, "fetching org teams");
        let result = client.list_org_teams(username).await;
        store(
            storage,
            &save("org_teams.json"),
            "organisation teams",
            result,
            &mut outcome,
        )?;
    }

    Ok(outcome)
}

/// Writes a fetched list, or records why it could not be fetched.
fn store<T: Serialize>(
    storage: &impl Storage,
    path: &Path,
    what: &str,
    result: Result<T, ClientError>,
    outcome: &mut Outcome,
) -> Result<(), CoreError> {
    match result {
        Ok(list) => storage.write_json(path, &list),
        Err(e) => outcome.item_error(what, e),
    }
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

    /// The owner-level lists used to be one step: a failing starred list cost
    /// the followers too.
    #[tokio::test]
    async fn one_failing_list_does_not_cost_the_others() {
        let client = MockBackupClient::new().with_error("starred", 500);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            starred: true,
            followers: true,
            ..Default::default()
        };
        let outcome = backup_user_data(
            &client,
            "octocat",
            &opts,
            &PathBuf::from(JSON_DIR),
            &storage,
        )
        .await
        .expect("not fatal");
        let whats: Vec<&str> = outcome.failed.iter().map(|(w, _)| w.as_str()).collect();
        assert_eq!(whats, ["starred repositories"]);
        assert!(storage
            .get(&PathBuf::from(format!("{JSON_DIR}/followers.json")))
            .is_some());
    }
}
