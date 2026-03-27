// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! User-level data backup: starred repos, watched repos, followers, following.

use std::path::Path;

use tracing::info;

use github_backup_client::GitHubClient;
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
    client: &GitHubClient,
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
