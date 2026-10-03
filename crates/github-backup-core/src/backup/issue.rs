// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Issue, issue comment, and issue event backup.

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{
    backup::merge::{merge_list, reusable},
    error::CoreError,
    storage::Storage,
};

/// Backs up all issues (and optionally comments and events) for a repository.
///
/// Writes:
/// - `meta_dir/issues.json` – every issue, merged with what earlier runs
///   stored (see below)
/// - `meta_dir/issue_comments/<number>.json` – comments per issue
/// - `meta_dir/issue_events/<number>.json` – events per issue
///
/// The issue list is **always fetched in full** and merged into the existing
/// `issues.json`: an issue that has since been deleted on GitHub stays in the
/// backup, and an incremental run can never shrink the file.  `since` is the
/// repository's incremental watermark; it only decides which issues get their
/// comment and event files re-fetched — those are skipped for issues not
/// updated since the watermark and whose files are already on disk.  With
/// `None` every issue is fetched.
///
/// Returns the total number of issues fetched (including PR-linked ones, which
/// are skipped for per-issue sub-resources).
///
/// # Errors
///
/// Propagates [`CoreError`] from API calls or storage writes.
pub async fn backup_issues(
    client: &impl BackupClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    since: Option<&str>,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<u64, CoreError> {
    if !opts.issues && !opts.issue_comments && !opts.issue_events {
        return Ok(0);
    }

    info!(owner, repo = repo_name, "fetching issues");
    // Never ask the API for a delta: the response is persisted wholesale.
    let issues = client.list_issues(owner, repo_name, None).await?;
    let count = issues.len() as u64;

    if opts.issues {
        let path = meta_dir.join("issues.json");
        let merged = merge_list(storage, &path, &issues, "number")?;
        storage.write_json(&path, &merged)?;
    }

    if !opts.issue_comments && !opts.issue_events {
        return Ok(count);
    }

    for issue in &issues {
        // The GitHub Issues API returns PRs too; skip them for issue-specific
        // per-issue data (they are handled in the PR backup path).
        if issue.is_pull_request() {
            continue;
        }

        let comments_path = meta_dir
            .join("issue_comments")
            .join(format!("{}.json", issue.number));
        let events_path = meta_dir
            .join("issue_events")
            .join(format!("{}.json", issue.number));

        // Unchanged since the last complete run and its files are still on
        // disk: what that run stored is still current.
        let mut wanted: Vec<&Path> = Vec::new();
        if opts.issue_comments {
            wanted.push(&comments_path);
        }
        if opts.issue_events {
            wanted.push(&events_path);
        }
        if reusable(storage, &issue.updated_at, since, &wanted) {
            continue;
        }

        if opts.issue_comments {
            let comments = client
                .list_issue_comments(owner, repo_name, issue.number)
                .await?;
            storage.write_json(&comments_path, &comments)?;
        }

        if opts.issue_events {
            let events = client
                .list_issue_events(owner, repo_name, issue.number)
                .await?;
            storage.write_json(&events_path, &events)?;
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
    use github_backup_types::issue::{Issue, IssuePullRequestRef};
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

    fn make_issue(number: u64, is_pr: bool) -> Issue {
        Issue {
            id: number,
            number,
            title: format!("Issue #{number}"),
            body: None,
            state: "open".to_string(),
            user: make_user(),
            labels: vec![],
            assignees: vec![],
            milestone: None,
            pull_request: if is_pr {
                Some(IssuePullRequestRef {
                    url: format!("https://api.github.com/repos/octocat/repo/pulls/{number}"),
                    html_url: format!("https://github.com/octocat/repo/pull/{number}"),
                })
            } else {
                None
            },
            comments: 0,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            closed_at: None,
            html_url: format!("https://github.com/octocat/repo/issues/{number}"),
        }
    }

    #[tokio::test]
    async fn backup_issues_all_flags_false_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default();

        backup_issues(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_issues");

        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_issues_flag_true_writes_issues_json() {
        let issue = make_issue(1, false);
        let client = MockBackupClient::new().with_issues(vec![issue]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            issues: true,
            ..Default::default()
        };

        backup_issues(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_issues");

        assert!(storage.get(&PathBuf::from("/meta/issues.json")).is_some());
    }

    #[tokio::test]
    async fn backup_issues_comments_written_per_issue() {
        let issue = make_issue(42, false);
        let client = MockBackupClient::new().with_issues(vec![issue]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            issue_comments: true,
            ..Default::default()
        };

        backup_issues(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_issues");

        assert!(storage
            .get(&PathBuf::from("/meta/issue_comments/42.json"))
            .is_some());
        // issues.json not written since `issues` flag is false
        assert!(storage.get(&PathBuf::from("/meta/issues.json")).is_none());
    }

    #[tokio::test]
    async fn backup_issues_pr_linked_issues_skipped_for_per_issue_data() {
        let pr_issue = make_issue(1, true);
        let real_issue = make_issue(2, false);
        let client = MockBackupClient::new().with_issues(vec![pr_issue, real_issue]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            issue_comments: true,
            issue_events: true,
            ..Default::default()
        };

        backup_issues(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_issues");

        assert!(
            storage
                .get(&PathBuf::from("/meta/issue_comments/1.json"))
                .is_none(),
            "PR-linked issue #1 must not produce comment file"
        );
        assert!(
            storage
                .get(&PathBuf::from("/meta/issue_comments/2.json"))
                .is_some(),
            "regular issue #2 must produce comment file"
        );
    }

    #[tokio::test]
    async fn backup_issues_events_flag_writes_events_json() {
        let issue = make_issue(7, false);
        let client = MockBackupClient::new().with_issues(vec![issue]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            issue_events: true,
            ..Default::default()
        };

        backup_issues(
            &client,
            "octocat",
            "Hello-World",
            &opts,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_issues");

        assert!(storage
            .get(&PathBuf::from("/meta/issue_events/7.json"))
            .is_some());
    }

    fn issue_updated(number: u64, updated_at: &str) -> Issue {
        Issue {
            updated_at: updated_at.to_string(),
            ..make_issue(number, false)
        }
    }

    fn stored_numbers(storage: &MemStorage) -> Vec<u64> {
        let bytes = storage
            .get(&PathBuf::from("/meta/issues.json"))
            .expect("issues.json written");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("valid JSON");
        value
            .as_array()
            .expect("array")
            .iter()
            .map(|i| i["number"].as_u64().expect("number"))
            .collect()
    }

    /// Regression for the data-loss bug: every run after the first used to
    /// overwrite `issues.json` with only the issues GitHub returned for the
    /// automatic `since` filter, so the file usually ended up as `[]`.
    #[tokio::test]
    async fn incremental_run_never_shrinks_issues_json() {
        let storage = MemStorage::default();
        let opts = BackupOptions {
            issues: true,
            ..Default::default()
        };

        // Run 1: three issues.
        let first = MockBackupClient::new().with_issues(vec![
            issue_updated(1, "2024-01-01T00:00:00Z"),
            issue_updated(2, "2024-01-02T00:00:00Z"),
            issue_updated(3, "2024-01-03T00:00:00Z"),
        ]);
        backup_issues(
            &first,
            "o",
            "r",
            &opts,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("run 1");
        assert_eq!(stored_numbers(&storage), vec![1, 2, 3]);

        // Run 2 (incremental): only #2 changed.
        let second =
            MockBackupClient::new().with_issues(vec![issue_updated(2, "2024-06-01T00:00:00Z")]);
        backup_issues(
            &second,
            "o",
            "r",
            &opts,
            Some("2024-03-01T00:00:00Z"),
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("run 2");
        assert_eq!(
            stored_numbers(&storage),
            vec![1, 2, 3],
            "older issues must survive"
        );

        // Run 3: GitHub returns nothing at all (everything deleted / empty delta).
        let third = MockBackupClient::new();
        backup_issues(
            &third,
            "o",
            "r",
            &opts,
            Some("2024-07-01T00:00:00Z"),
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("run 3");
        assert_eq!(
            stored_numbers(&storage),
            vec![1, 2, 3],
            "an empty listing must not erase the backup"
        );
    }

    #[tokio::test]
    async fn changed_issue_replaces_its_stored_copy() {
        let storage = MemStorage::default();
        let opts = BackupOptions {
            issues: true,
            ..Default::default()
        };
        let mut old = issue_updated(1, "2024-01-01T00:00:00Z");
        old.title = "before".into();
        backup_issues(
            &MockBackupClient::new().with_issues(vec![old]),
            "o",
            "r",
            &opts,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("first");

        let mut new = issue_updated(1, "2024-02-01T00:00:00Z");
        new.title = "after".into();
        backup_issues(
            &MockBackupClient::new().with_issues(vec![new]),
            "o",
            "r",
            &opts,
            Some("2024-01-15T00:00:00Z"),
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("second");

        let text =
            String::from_utf8(storage.get(&PathBuf::from("/meta/issues.json")).unwrap()).unwrap();
        assert!(text.contains("after") && !text.contains("before"), "{text}");
    }

    #[tokio::test]
    async fn watermark_skips_sub_resources_of_unchanged_issues_only() {
        let storage = MemStorage::default();
        // Files an earlier run left for issue 1 (sentinel content proves they
        // are neither refetched nor rewritten).
        for dir in ["issue_comments", "issue_events"] {
            storage
                .write_bytes(&PathBuf::from(format!("/meta/{dir}/1.json")), b"kept")
                .expect("seed");
        }
        let client = MockBackupClient::new().with_issues(vec![
            issue_updated(1, "2024-01-01T00:00:00Z"), // before the watermark
            issue_updated(2, "2024-06-01T00:00:00Z"), // after the watermark
        ]);
        let opts = BackupOptions {
            issue_comments: true,
            issue_events: true,
            ..Default::default()
        };
        backup_issues(
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

        for dir in ["issue_comments", "issue_events"] {
            assert_eq!(
                storage
                    .get(&PathBuf::from(format!("/meta/{dir}/1.json")))
                    .as_deref(),
                Some(b"kept".as_slice()),
                "{dir}/1.json: unchanged issue must be left alone"
            );
            let two = storage
                .get(&PathBuf::from(format!("/meta/{dir}/2.json")))
                .unwrap_or_else(|| panic!("{dir}/2.json: changed issue must be fetched"));
            assert_ne!(two, b"kept");
        }
    }

    /// The shortcut must never leave a hole: an unchanged issue whose file is
    /// absent (hand-cleaned directory, category enabled later) is fetched.
    #[tokio::test]
    async fn unchanged_issue_with_a_missing_file_is_still_fetched() {
        let storage = MemStorage::default();
        // Comments exist from an earlier run; events were never fetched.
        storage
            .write_bytes(&PathBuf::from("/meta/issue_comments/1.json"), b"kept")
            .expect("seed");
        let client =
            MockBackupClient::new().with_issues(vec![issue_updated(1, "2024-01-01T00:00:00Z")]);
        let opts = BackupOptions {
            issue_comments: true,
            issue_events: true,
            ..Default::default()
        };
        backup_issues(
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

        assert!(
            storage
                .get(&PathBuf::from("/meta/issue_events/1.json"))
                .is_some(),
            "the missing events file must be fetched"
        );
    }

    #[tokio::test]
    async fn without_a_watermark_every_issue_gets_its_sub_resources() {
        let storage = MemStorage::default();
        let client = MockBackupClient::new().with_issues(vec![
            issue_updated(1, "2020-01-01T00:00:00Z"),
            issue_updated(2, "2020-01-02T00:00:00Z"),
        ]);
        let opts = BackupOptions {
            issue_comments: true,
            ..Default::default()
        };
        backup_issues(
            &client,
            "o",
            "r",
            &opts,
            None,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup");
        assert!(storage
            .get(&PathBuf::from("/meta/issue_comments/1.json"))
            .is_some());
        assert!(storage
            .get(&PathBuf::from("/meta/issue_comments/2.json"))
            .is_some());
    }
}
