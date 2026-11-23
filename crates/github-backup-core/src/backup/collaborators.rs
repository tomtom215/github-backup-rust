// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository collaborator backup.

use std::path::Path;

use tracing::info;

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, storage::Storage};

/// Backs up the collaborator list for a single repository.
///
/// Writes `meta_dir/collaborators.json` when `opts.collaborators` is enabled:
/// a snapshot of who has access now, replaced on every run (someone whose
/// access was removed must not look as if they still had it).
///
/// The GitHub API requires write, maintain or admin access to the
/// repository; how a refusal is reported is decided by the engine.
///
/// # Errors
///
/// Propagates [`CoreError`] from the API call (including a 403 refusal) or
/// the storage write.
pub async fn backup_collaborators(
    client: &impl BackupClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<(), CoreError> {
    if !opts.collaborators {
        return Ok(());
    }

    let collaborators = client.list_collaborators(owner, repo_name).await?;
    info!(
        owner,
        repo = repo_name,
        count = collaborators.len(),
        "fetched collaborators"
    );
    storage.write_json(&meta_dir.join("collaborators.json"), &collaborators)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::collaborator::{Collaborator, CollaboratorPermissions};
    use github_backup_types::config::BackupOptions;
    use std::path::PathBuf;

    fn make_collaborator(login: &str) -> Collaborator {
        Collaborator {
            id: 1,
            login: login.to_string(),
            user_type: "User".to_string(),
            avatar_url: "https://example.com/avatar.png".to_string(),
            html_url: format!("https://github.com/{login}"),
            role_name: Some("write".to_string()),
            permissions: Some(CollaboratorPermissions {
                pull: true,
                triage: true,
                push: true,
                maintain: false,
                admin: false,
            }),
        }
    }

    #[tokio::test]
    async fn backup_collaborators_disabled_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default(); // collaborators: false

        backup_collaborators(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_collaborators");

        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_collaborators_enabled_writes_json() {
        let collab = make_collaborator("contributor");
        let client = MockBackupClient::new().with_collaborators(vec![collab]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            collaborators: true,
            ..Default::default()
        };

        backup_collaborators(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_collaborators");

        assert!(
            storage
                .get(&PathBuf::from("/meta/collaborators.json"))
                .is_some(),
            "collaborators.json should be written"
        );
    }

    #[tokio::test]
    async fn backup_collaborators_empty_list_still_writes_file() {
        let client = MockBackupClient::new(); // returns empty list
        let storage = MemStorage::default();
        let opts = BackupOptions {
            collaborators: true,
            ..Default::default()
        };

        backup_collaborators(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_collaborators");

        assert!(storage
            .get(&PathBuf::from("/meta/collaborators.json"))
            .is_some());
    }
}
