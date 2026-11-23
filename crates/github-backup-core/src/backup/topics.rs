// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository topic backup.
//!
//! Saves `topics.json`: the list of topic names assigned to the repository,
//! taken from the repository object (which carries `topics`), so no request
//! of its own is needed.  The file keeps the shape it always had, a JSON array
//! of strings, and is a snapshot replaced on every run.

use std::path::Path;

use serde_json::Value;
use tracing::debug;

use github_backup_types::{config::BackupOptions, Raw, Repository};

use crate::{error::CoreError, storage::Storage};

/// Writes the `topics` of `repo` to `meta_dir/topics.json`.
///
/// Skipped when `opts.topics` is `false` or `opts.dry_run` is `true`, and
/// when the repository object has no `topics` (an old GitHub Enterprise
/// Server).
///
/// # Errors
///
/// Propagates [`CoreError`] from the storage write.
pub fn backup_topics(
    repo: &Raw<Repository>,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<(), CoreError> {
    if !opts.topics || opts.dry_run {
        return Ok(());
    }
    match repo.json().get("topics") {
        Some(topics @ Value::Array(_)) => storage.write_json(&meta_dir.join("topics.json"), topics),
        _ => {
            debug!(repo = %repo.full_name, "the repository object has no topics");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::test_support::MemStorage;
    use std::path::PathBuf;

    const META: &str = "/meta";

    fn repo(topics: Option<Value>) -> Raw<Repository> {
        let mut json = serde_json::json!({
            "id": 1, "name": "repo", "full_name": "owner/repo",
            "owner": {"id": 1, "login": "owner", "type": "User", "avatar_url": "", "html_url": ""},
            "private": false, "fork": false, "clone_url": "", "ssh_url": "", "html_url": ""
        });
        if let Some(topics) = topics {
            json["topics"] = topics;
        }
        Raw::from_value(json).expect("repository")
    }

    fn topics_on_disk(storage: &MemStorage) -> Option<Value> {
        storage
            .get(&PathBuf::from(format!("{META}/topics.json")))
            .map(|bytes| serde_json::from_slice(&bytes).expect("json"))
    }

    #[test]
    fn disabled_or_dry_run_writes_nothing() {
        let storage = MemStorage::default();
        let r = repo(Some(serde_json::json!(["rust"])));
        backup_topics(
            &r,
            &BackupOptions::default(),
            &PathBuf::from(META),
            &storage,
        )
        .expect("ok");
        let dry = BackupOptions {
            topics: true,
            dry_run: true,
            ..Default::default()
        };
        backup_topics(&r, &dry, &PathBuf::from(META), &storage).expect("ok");
        assert_eq!(storage.len(), 0);
    }

    #[test]
    fn topics_come_from_the_repository_object_as_a_list_of_names() {
        let storage = MemStorage::default();
        let opts = BackupOptions {
            topics: true,
            ..Default::default()
        };
        let r = repo(Some(serde_json::json!(["rust", "backup"])));
        backup_topics(&r, &opts, &PathBuf::from(META), &storage).expect("ok");
        assert_eq!(
            topics_on_disk(&storage),
            Some(serde_json::json!(["rust", "backup"]))
        );

        let empty = MemStorage::default();
        backup_topics(
            &repo(Some(serde_json::json!([]))),
            &opts,
            &PathBuf::from(META),
            &empty,
        )
        .expect("ok");
        assert_eq!(topics_on_disk(&empty), Some(serde_json::json!([])));

        let missing = MemStorage::default();
        backup_topics(&repo(None), &opts, &PathBuf::from(META), &missing).expect("ok");
        assert_eq!(topics_on_disk(&missing), None);
    }
}
