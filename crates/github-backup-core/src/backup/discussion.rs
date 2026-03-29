// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub Discussions backup.
//!
//! Writes `discussions.json` and per-discussion comment files to the repository
//! metadata directory.  The feature must be enabled on the repository; if the
//! API returns 404 the function logs an informational message and returns
//! successfully.

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, storage::Storage};

/// Backs up GitHub Discussions for a single repository.
///
/// When `opts.discussions` is enabled:
/// - Writes `meta_dir/discussions.json` with all discussion objects.
/// - For each discussion, writes `meta_dir/discussion_comments_<number>.json`
///   with the comments thread.
///
/// Discussions must be enabled on the repository.  The API returns 404 when
/// the feature is disabled; this is treated as a non-error and the function
/// returns `Ok(0)`.
///
/// Returns the number of discussions backed up.
///
/// # Errors
///
/// Propagates [`CoreError`] from storage writes or non-404 API errors.
pub async fn backup_discussions(
    client: &impl BackupClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<u64, CoreError> {
    if !opts.discussions {
        return Ok(0);
    }

    let discussions = match client.list_discussions(owner, repo_name).await {
        Ok(d) => d,
        Err(github_backup_client::ClientError::ApiError {
            status: 404 | 410, ..
        }) => {
            info!(
                repo = format!("{owner}/{repo_name}"),
                "skipping discussions (feature not enabled on this repository)"
            );
            return Ok(0);
        }
        Err(e) => return Err(e.into()),
    };

    let count = discussions.len() as u64;
    info!(
        owner,
        repo = repo_name,
        count,
        "backing up discussions"
    );
    storage.write_json(&meta_dir.join("discussions.json"), &discussions)?;

    // Back up comments for each discussion.
    for discussion in &discussions {
        let comments = match client
            .list_discussion_comments(owner, repo_name, discussion.number)
            .await
        {
            Ok(c) => c,
            Err(github_backup_client::ClientError::ApiError {
                status: 404 | 410, ..
            }) => {
                info!(
                    repo = format!("{owner}/{repo_name}"),
                    discussion_number = discussion.number,
                    "skipping discussion comments (not available)"
                );
                continue;
            }
            Err(e) => return Err(e.into()),
        };

        let filename = format!("discussion_comments_{}.json", discussion.number);
        storage.write_json(&meta_dir.join(&filename), &comments)?;
        info!(
            owner,
            repo = repo_name,
            discussion_number = discussion.number,
            comment_count = comments.len(),
            "saved discussion comments"
        );
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::config::BackupOptions;
    use github_backup_types::discussion::{Discussion, DiscussionComment};
    use github_backup_types::user::User;
    use std::path::PathBuf;

    fn make_user() -> User {
        User {
            login: "octocat".to_string(),
            id: 1,
            avatar_url: "https://github.com/images/error/octocat_happy.gif".to_string(),
            html_url: "https://github.com/octocat".to_string(),
            user_type: "User".to_string(),
        }
    }

    fn make_discussion(number: u64, title: &str) -> Discussion {
        Discussion {
            number,
            title: title.to_string(),
            body: "Discussion body".to_string(),
            locked: false,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            html_url: format!("https://github.com/octocat/repo/discussions/{number}"),
            user: make_user(),
            comments: 0,
            category: None,
            answered: false,
        }
    }

    fn make_comment(id: u64) -> DiscussionComment {
        DiscussionComment {
            id,
            body: "A comment".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            html_url: format!("https://github.com/octocat/repo/discussions/1#comment-{id}"),
            user: make_user(),
        }
    }

    #[tokio::test]
    async fn backup_discussions_disabled_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default(); // discussions: false

        let count = backup_discussions(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_discussions");

        assert_eq!(count, 0);
        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_discussions_enabled_writes_json() {
        let d = make_discussion(1, "Welcome");
        let client = MockBackupClient::new()
            .with_discussions(vec![d])
            .with_discussion_comments(vec![make_comment(42)]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            discussions: true,
            ..Default::default()
        };

        let count = backup_discussions(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_discussions");

        assert_eq!(count, 1);
        assert!(
            storage
                .get(&PathBuf::from("/meta/discussions.json"))
                .is_some(),
            "discussions.json should be written"
        );
        assert!(
            storage
                .get(&PathBuf::from("/meta/discussion_comments_1.json"))
                .is_some(),
            "discussion_comments_1.json should be written"
        );
    }

    #[tokio::test]
    async fn backup_discussions_empty_list_writes_file() {
        let client = MockBackupClient::new(); // returns empty discussions
        let storage = MemStorage::default();
        let opts = BackupOptions {
            discussions: true,
            ..Default::default()
        };

        let count = backup_discussions(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_discussions");

        assert_eq!(count, 0);
        assert!(
            storage
                .get(&PathBuf::from("/meta/discussions.json"))
                .is_some(),
            "discussions.json should still be written for empty list"
        );
    }
}
