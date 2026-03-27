// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Pull request, review comment, commit, and review backup.

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
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
    client: &impl BackupClient,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::config::BackupOptions;
    use github_backup_types::pull_request::{PullRequest, PullRequestRef};
    use github_backup_types::user::User;
    use std::path::PathBuf;

    fn make_user() -> User {
        User {
            id: 1,
            login: "octocat".to_string(),
            user_type: "User".to_string(),
            avatar_url: String::new(),
            html_url: String::new(),
        }
    }

    fn make_pr_ref() -> PullRequestRef {
        PullRequestRef {
            label: "octocat:main".to_string(),
            ref_name: "main".to_string(),
            sha: "abc123".to_string(),
            repo: None,
        }
    }

    fn make_pr(number: u64) -> PullRequest {
        PullRequest {
            id: number,
            number,
            title: format!("PR #{number}"),
            body: None,
            state: "open".to_string(),
            merged: None,
            user: make_user(),
            labels: vec![],
            assignees: vec![],
            milestone: None,
            head: make_pr_ref(),
            base: make_pr_ref(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            merged_at: None,
            closed_at: None,
            html_url: format!("https://github.com/octocat/repo/pull/{number}"),
            commits: None,
            changed_files: None,
            additions: None,
            deletions: None,
        }
    }

    #[tokio::test]
    async fn backup_pull_requests_all_flags_false_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default();

        backup_pull_requests(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_pull_requests");

        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_pull_requests_flag_true_writes_pulls_json() {
        let pr = make_pr(1);
        let client = MockBackupClient::new().with_pull_requests(vec![pr]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            pulls: true,
            ..Default::default()
        };

        backup_pull_requests(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_pull_requests");

        assert!(storage.get(&PathBuf::from("/meta/pulls.json")).is_some());
    }

    #[tokio::test]
    async fn backup_pull_requests_comments_written_per_pr() {
        let pr = make_pr(5);
        let client = MockBackupClient::new().with_pull_requests(vec![pr]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            pull_comments: true,
            ..Default::default()
        };

        backup_pull_requests(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_pull_requests");

        assert!(storage
            .get(&PathBuf::from("/meta/pull_comments/5.json"))
            .is_some());
    }

    #[tokio::test]
    async fn backup_pull_requests_commits_and_reviews_written_per_pr() {
        let pr = make_pr(3);
        let client = MockBackupClient::new().with_pull_requests(vec![pr]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            pull_commits: true,
            pull_reviews: true,
            ..Default::default()
        };

        backup_pull_requests(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_pull_requests");

        assert!(storage
            .get(&PathBuf::from("/meta/pull_commits/3.json"))
            .is_some());
        assert!(storage
            .get(&PathBuf::from("/meta/pull_reviews/3.json"))
            .is_some());
    }

    #[tokio::test]
    async fn backup_pull_requests_only_pulls_no_per_pr_data() {
        let pr = make_pr(1);
        let client = MockBackupClient::new().with_pull_requests(vec![pr]);
        let storage = MemStorage::default();
        // Only pulls = true, all per-PR flags false
        let opts = BackupOptions {
            pulls: true,
            ..Default::default()
        };

        backup_pull_requests(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_pull_requests");

        // Only pulls.json, no per-PR files
        assert_eq!(storage.len(), 1);
        assert!(storage.get(&PathBuf::from("/meta/pulls.json")).is_some());
    }
}
