// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Issue, issue comment, and issue event backup.

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
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
    client: &impl BackupClient,
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
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_issues");

        assert!(storage
            .get(&PathBuf::from("/meta/issue_events/7.json"))
            .is_some());
    }
}
