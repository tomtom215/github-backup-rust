// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Issue, issue comment, and issue event backup.

use std::path::Path;

use tracing::info;

use github_backup_client::GitHubClient;
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, storage::Storage};

/// Backs up all issues (and optionally comments and events) for a repository.
///
/// Writes:
/// - `meta_dir/issues.json` – all issues
/// - `meta_dir/issue_comments/<number>.json` – comments per issue
/// - `meta_dir/issue_events/<number>.json` – events per issue
///
/// # Errors
///
/// Propagates [`CoreError`] from API calls or storage writes.
pub async fn backup_issues(
    client: &GitHubClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<(), CoreError> {
    if !opts.issues && !opts.issue_comments && !opts.issue_events {
        return Ok(());
    }

    info!(owner, repo = repo_name, "fetching issues");
    let issues = client.list_issues(owner, repo_name).await?;

    if opts.issues {
        storage.write_json(&meta_dir.join("issues.json"), &issues)?;
    }

    if !opts.issue_comments && !opts.issue_events {
        return Ok(());
    }

    for issue in &issues {
        // The GitHub Issues API returns PRs too; skip them for issue-specific
        // per-issue data (they are handled in the PR backup path).
        if issue.is_pull_request() {
            continue;
        }

        if opts.issue_comments {
            let comments = client
                .list_issue_comments(owner, repo_name, issue.number)
                .await?;
            let path = meta_dir
                .join("issue_comments")
                .join(format!("{}.json", issue.number));
            storage.write_json(&path, &comments)?;
        }

        if opts.issue_events {
            let events = client
                .list_issue_events(owner, repo_name, issue.number)
                .await?;
            let path = meta_dir
                .join("issue_events")
                .join(format!("{}.json", issue.number));
            storage.write_json(&path, &events)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    // Issue backup integration tests require a live API or a mock HTTP server.
    // Unit-level tests are in the backup/repository module; the backup logic
    // here is validated via the engine integration tests.
}
