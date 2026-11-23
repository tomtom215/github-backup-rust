// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Commit comment backup.
//!
//! Saves `commit_comments.json`: every comment made on a commit of the
//! repository (`GET /repos/{owner}/{repo}/comments`), merged by `id` with
//! what earlier runs stored, so a deleted comment stays in the backup.

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{backup::merge::merge_and_write, error::CoreError, storage::Storage};

/// Backs up the commit comments of a repository to
/// `meta_dir/commit_comments.json`.
///
/// Skipped when `opts.commit_comments` is `false` or `opts.dry_run` is `true`.
///
/// # Errors
///
/// Propagates [`CoreError`] from the API call or the storage write.
pub async fn backup_commit_comments(
    client: &impl BackupClient,
    owner: &str,
    repo: &str,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<(), CoreError> {
    if !opts.commit_comments || opts.dry_run {
        return Ok(());
    }
    info!(owner, repo, "fetching commit comments");
    let comments = client.list_commit_comments(owner, repo).await?;
    merge_and_write(
        storage,
        &meta_dir.join("commit_comments.json"),
        &comments,
        "id",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use serde_json::{json, Value};
    use std::path::PathBuf;

    fn opts() -> BackupOptions {
        BackupOptions {
            commit_comments: true,
            ..Default::default()
        }
    }

    fn on_disk(storage: &MemStorage) -> Value {
        serde_json::from_slice(
            &storage
                .get(&PathBuf::from("/meta/commit_comments.json"))
                .expect("written"),
        )
        .expect("json")
    }

    #[tokio::test]
    async fn disabled_writes_nothing() {
        let storage = MemStorage::default();
        backup_commit_comments(
            &MockBackupClient::new(),
            "o",
            "r",
            &BackupOptions::default(),
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("ok");
        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn comments_are_stored_as_served_and_accumulate() {
        let storage = MemStorage::default();
        let first = MockBackupClient::new().with_raw(
            "commit_comments",
            vec![json!({"id": 1, "commit_id": "abc", "body": "first", "line": 3})],
        );
        backup_commit_comments(&first, "o", "r", &opts(), &PathBuf::from("/meta"), &storage)
            .await
            .expect("run 1");
        let second = MockBackupClient::new().with_raw(
            "commit_comments",
            vec![json!({"id": 2, "commit_id": "def", "body": "second"})],
        );
        backup_commit_comments(
            &second,
            "o",
            "r",
            &opts(),
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("run 2");
        assert_eq!(
            on_disk(&storage),
            json!([
                {"id": 1, "commit_id": "abc", "body": "first", "line": 3},
                {"id": 2, "commit_id": "def", "body": "second"}
            ])
        );
    }
}
