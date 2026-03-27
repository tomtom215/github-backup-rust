// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Gist metadata and git clone backup.

use std::path::Path;

use tracing::info;

use github_backup_client::GitHubClient;
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, git::GitRunner, storage::Storage};

/// Backs up gists owned by `username` and optionally starred gists.
///
/// For each gist:
/// - Writes `gists_meta_dir/<id>.json` with gist metadata
/// - Clones `gists_git_dir/<id>.git` as a bare mirror
///
/// # Errors
///
/// Propagates [`CoreError`] from API calls, storage writes, or git operations.
pub async fn backup_gists(
    client: &GitHubClient,
    username: &str,
    opts: &BackupOptions,
    gists_git_dir: &Path,
    gists_meta_dir: &Path,
    storage: &impl Storage,
    git: &impl GitRunner,
) -> Result<(), CoreError> {
    if !opts.gists && !opts.starred_gists {
        return Ok(());
    }

    if opts.gists {
        info!(username, "fetching gists");
        let gists = client.list_gists(username).await?;
        for gist in &gists {
            let meta_path = gists_meta_dir.join(format!("{}.json", gist.id));
            storage.write_json(&meta_path, gist)?;

            let dest = gists_git_dir.join(format!("{}.git", gist.id));
            git.mirror_clone(&gist.git_pull_url, &dest)?;
        }
        storage.write_json(&gists_meta_dir.join("index.json"), &gists)?;
    }

    if opts.starred_gists {
        info!(username, "fetching starred gists");
        let starred = client.list_starred_gists().await?;
        for gist in &starred {
            let meta_path = gists_meta_dir.join(format!("{}.starred.json", gist.id));
            storage.write_json(&meta_path, gist)?;
        }
        storage.write_json(&gists_meta_dir.join("starred_index.json"), &starred)?;
    }

    Ok(())
}
