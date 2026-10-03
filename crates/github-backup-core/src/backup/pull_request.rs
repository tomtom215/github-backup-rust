// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Pull request, review comment, commit, and review backup.

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{
    backup::merge::{merge_list, reusable},
    error::CoreError,
    storage::Storage,
};

/// Backs up all pull requests (and optionally comments, commits, and reviews)
/// for a repository.
///
/// Writes:
/// - `meta_dir/pulls.json` – every PR, merged with what earlier runs stored
/// - `meta_dir/pull_comments/<number>.json` – review comments per PR
/// - `meta_dir/pull_commits/<number>.json` – commits per PR
/// - `meta_dir/pull_reviews/<number>.json` – reviews per PR
///
/// Like [`backup_issues`](super::issue::backup_issues), the PR list is
/// **always fetched in full** and merged into the existing `pulls.json`, so an
/// incremental run can never shrink the file.  `since` is the repository's
/// incremental watermark; it only decides which PRs get their comment, commit
/// and review files re-fetched — those are skipped for PRs not updated since
/// the watermark and whose files are already on disk.
///
/// Returns the total number of pull requests fetched.
///
/// # Errors
///
/// Propagates [`CoreError`] from API calls or storage writes.
pub async fn backup_pull_requests(
    client: &impl BackupClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    since: Option<&str>,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<u64, CoreError> {
    if !opts.pulls && !opts.pull_comments && !opts.pull_commits && !opts.pull_reviews {
        return Ok(0);
    }

    info!(owner, repo = repo_name, "fetching pull requests");
    // Never ask the API for a delta: the response is persisted wholesale.
    let pulls = client.list_pull_requests(owner, repo_name, None).await?;
    let count = pulls.len() as u64;

    if opts.pulls {
        let path = meta_dir.join("pulls.json");
        let merged = merge_list(storage, &path, &pulls, "number")?;
        storage.write_json(&path, &merged)?;
    }

    if !opts.pull_comments && !opts.pull_commits && !opts.pull_reviews {
        return Ok(count);
    }

    for pr in &pulls {
        let file = |dir: &str| meta_dir.join(dir).join(format!("{}.json", pr.number));
        let comments_path = file("pull_comments");
        let commits_path = file("pull_commits");
        let reviews_path = file("pull_reviews");

        let mut wanted: Vec<&Path> = Vec::new();
        if opts.pull_comments {
            wanted.push(&comments_path);
        }
        if opts.pull_commits {
            wanted.push(&commits_path);
        }
        if opts.pull_reviews {
            wanted.push(&reviews_path);
        }
        if reusable(storage, &pr.updated_at, since, &wanted) {
            continue;
        }

        if opts.pull_comments {
            let comments = client
                .list_pull_comments(owner, repo_name, pr.number)
                .await?;
            storage.write_json(&comments_path, &comments)?;
        }

        if opts.pull_commits {
            let commits = client
                .list_pull_commits(owner, repo_name, pr.number)
                .await?;
            storage.write_json(&commits_path, &commits)?;
        }

        if opts.pull_reviews {
            let reviews = client
                .list_pull_reviews(owner, repo_name, pr.number)
                .await?;
            storage.write_json(&reviews_path, &reviews)?;
        }
    }

    Ok(count)
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_pull_requests");

        // Only pulls.json, no per-PR files
        assert_eq!(storage.len(), 1);
        assert!(storage.get(&PathBuf::from("/meta/pulls.json")).is_some());
    }

    fn pr_updated(number: u64, updated_at: &str) -> PullRequest {
        PullRequest {
            updated_at: updated_at.to_string(),
            ..make_pr(number)
        }
    }

    fn stored_numbers(storage: &MemStorage) -> Vec<u64> {
        let bytes = storage
            .get(&PathBuf::from("/meta/pulls.json"))
            .expect("pulls.json written");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("valid JSON");
        value
            .as_array()
            .expect("array")
            .iter()
            .map(|p| p["number"].as_u64().expect("number"))
            .collect()
    }

    /// Same data-loss bug as issues: an automatic `since` made every later run
    /// overwrite `pulls.json` with only the changed pull requests.
    #[tokio::test]
    async fn incremental_run_never_shrinks_pulls_json() {
        let storage = MemStorage::default();
        let opts = BackupOptions {
            pulls: true,
            ..Default::default()
        };
        let meta = PathBuf::from("/meta");

        let first = MockBackupClient::new().with_pull_requests(vec![
            pr_updated(1, "2024-01-01T00:00:00Z"),
            pr_updated(2, "2024-01-02T00:00:00Z"),
        ]);
        backup_pull_requests(&first, "o", "r", &opts, None, &meta, &storage)
            .await
            .expect("run 1");
        assert_eq!(stored_numbers(&storage), vec![1, 2]);

        let second =
            MockBackupClient::new().with_pull_requests(vec![pr_updated(2, "2024-06-01T00:00:00Z")]);
        backup_pull_requests(
            &second,
            "o",
            "r",
            &opts,
            Some("2024-03-01T00:00:00Z"),
            &meta,
            &storage,
        )
        .await
        .expect("run 2");
        assert_eq!(stored_numbers(&storage), vec![1, 2]);

        let third = MockBackupClient::new();
        backup_pull_requests(
            &third,
            "o",
            "r",
            &opts,
            Some("2024-07-01T00:00:00Z"),
            &meta,
            &storage,
        )
        .await
        .expect("run 3");
        assert_eq!(
            stored_numbers(&storage),
            vec![1, 2],
            "an empty listing must not erase the backup"
        );
    }

    #[tokio::test]
    async fn watermark_skips_unchanged_prs_whose_files_exist() {
        let storage = MemStorage::default();
        for dir in ["pull_comments", "pull_commits", "pull_reviews"] {
            storage
                .write_bytes(&PathBuf::from(format!("/meta/{dir}/1.json")), b"kept")
                .expect("seed");
        }
        let client = MockBackupClient::new().with_pull_requests(vec![
            pr_updated(1, "2024-01-01T00:00:00Z"),
            pr_updated(2, "2024-06-01T00:00:00Z"),
        ]);
        let opts = BackupOptions {
            pull_comments: true,
            pull_commits: true,
            pull_reviews: true,
            ..Default::default()
        };
        backup_pull_requests(
            &client,
            "o",
            "r",
            &opts,
            Some("2024-03-01T00:00:00Z"),
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup");

        for dir in ["pull_comments", "pull_commits", "pull_reviews"] {
            assert_eq!(
                storage
                    .get(&PathBuf::from(format!("/meta/{dir}/1.json")))
                    .as_deref(),
                Some(b"kept".as_slice()),
                "{dir}/1.json: unchanged PR must be left alone"
            );
            assert!(
                storage
                    .get(&PathBuf::from(format!("/meta/{dir}/2.json")))
                    .is_some_and(|b| b != b"kept"),
                "{dir}/2.json: changed PR must be fetched"
            );
        }
    }

    #[tokio::test]
    async fn unchanged_pr_with_a_missing_file_is_still_fetched() {
        let storage = MemStorage::default();
        storage
            .write_bytes(&PathBuf::from("/meta/pull_commits/1.json"), b"kept")
            .expect("seed");
        let client =
            MockBackupClient::new().with_pull_requests(vec![pr_updated(1, "2024-01-01T00:00:00Z")]);
        let opts = BackupOptions {
            pull_commits: true,
            pull_reviews: true,
            ..Default::default()
        };
        backup_pull_requests(
            &client,
            "o",
            "r",
            &opts,
            Some("2024-03-01T00:00:00Z"),
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup");

        assert!(storage
            .get(&PathBuf::from("/meta/pull_reviews/1.json"))
            .is_some());
    }
}
