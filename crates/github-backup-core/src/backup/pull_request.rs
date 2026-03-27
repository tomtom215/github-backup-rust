// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Pull request, review comment, commit, and review backup.

use std::path::Path;

use tracing::info;

use github_backup_client::GitHubClient;
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, storage::Storage};

/// Backs up all pull requests (and optionally comments, commits, and reviews)
/// for a repository.
///
/// Writes:
/// - `meta_dir/pulls.json` – all PRs
/// - `meta_dir/pull_comments/<number>.json` – review comments per PR
/// - `meta_dir/pull_commits/<number>.json` – commits per PR
/// - `meta_dir/pull_reviews/<number>.json` – reviews per PR
///
/// # Errors
///
/// Propagates [`CoreError`] from API calls or storage writes.
pub async fn backup_pull_requests(
    client: &GitHubClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<(), CoreError> {
    if !opts.pulls && !opts.pull_comments && !opts.pull_commits && !opts.pull_reviews {
        return Ok(());
    }

    info!(owner, repo = repo_name, "fetching pull requests");
    let pulls = client.list_pull_requests(owner, repo_name).await?;

    if opts.pulls {
        storage.write_json(&meta_dir.join("pulls.json"), &pulls)?;
    }

    if !opts.pull_comments && !opts.pull_commits && !opts.pull_reviews {
        return Ok(());
    }

    for pr in &pulls {
        if opts.pull_comments {
            let comments = client
                .list_pull_comments(owner, repo_name, pr.number)
                .await?;
            let path = meta_dir
                .join("pull_comments")
                .join(format!("{}.json", pr.number));
            storage.write_json(&path, &comments)?;
        }

        if opts.pull_commits {
            let commits = client
                .list_pull_commits(owner, repo_name, pr.number)
                .await?;
            let path = meta_dir
                .join("pull_commits")
                .join(format!("{}.json", pr.number));
            storage.write_json(&path, &commits)?;
        }

        if opts.pull_reviews {
            let reviews = client
                .list_pull_reviews(owner, repo_name, pr.number)
                .await?;
            let path = meta_dir
                .join("pull_reviews")
                .join(format!("{}.json", pr.number));
            storage.write_json(&path, &reviews)?;
        }
    }

    Ok(())
}
