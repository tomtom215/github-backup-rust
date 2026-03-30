// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository branch-list and branch-protection backup.
//!
//! Saves `branches.json` containing the name and SHA of every branch in the
//! repository.  When any branch has `protected: true`, also fetches the
//! detailed protection rules for each protected branch and saves them to
//! `branch_protections.json`.  Requires admin access; 403/404 responses per
//! branch are silently skipped.

use std::collections::HashMap;
use std::path::Path;

use tracing::{debug, info};

use github_backup_client::{BackupClient, ClientError};
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, storage::Storage};

/// Backs up repository branches and branch-protection rules.
///
/// Saves `branches.json`.  When any branch has `protected: true` and the
/// token has admin access, also saves `branch_protections.json` mapping
/// branch name → protection rules.
///
/// Skipped when `opts.branches` is `false` or `opts.dry_run` is `true`.
/// Individual protection-rule fetches that return 403/404 are skipped
/// silently (requires admin access; not all branches may be protected).
///
/// # Errors
///
/// Propagates [`CoreError`] from API calls or storage writes.
pub async fn backup_branches(
    client: &impl BackupClient,
    owner: &str,
    repo: &str,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<(), CoreError> {
    if !opts.branches || opts.dry_run {
        return Ok(());
    }

    info!(owner, repo, "fetching branches");
    let branches = client.list_branches(owner, repo).await?;
    storage.write_json(&meta_dir.join("branches.json"), &branches)?;

    // Fetch detailed protection rules for every protected branch.
    let protected: Vec<&str> = branches
        .iter()
        .filter(|b| b.protected)
        .map(|b| b.name.as_str())
        .collect();

    if protected.is_empty() {
        return Ok(());
    }

    debug!(
        owner,
        repo,
        count = protected.len(),
        "fetching branch protection rules"
    );

    let mut protections = HashMap::new();
    for branch_name in protected {
        match client.get_branch_protection(owner, repo, branch_name).await {
            Ok(rules) => {
                protections.insert(branch_name.to_string(), rules);
            }
            Err(ClientError::ApiError {
                status: 403 | 404, ..
            }) => {
                debug!(owner, repo, branch = branch_name, "skipping branch protection (no admin access or not protected)");
            }
            Err(e) => return Err(e.into()),
        }
    }

    if !protections.is_empty() {
        storage.write_json(&meta_dir.join("branch_protections.json"), &protections)?;
        info!(
            owner,
            repo,
            count = protections.len(),
            "saved branch protection rules"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::{Branch, BranchCommit, BranchProtection, SimpleEnabled};
    use std::path::PathBuf;

    const META: &str = "/meta";

    fn make_branch(name: &str, protected: bool) -> Branch {
        Branch {
            name: name.to_string(),
            protected,
            commit: BranchCommit {
                sha: "abc1234".to_string(),
                url: "https://api.github.com/commits/abc1234".to_string(),
            },
        }
    }

    fn make_protection(branch: &str) -> BranchProtection {
        BranchProtection {
            url: format!("https://api.github.com/repos/owner/repo/branches/{branch}/protection"),
            required_status_checks: None,
            enforce_admins: None,
            required_pull_request_reviews: None,
            restrictions: None,
            required_linear_history: Some(SimpleEnabled { enabled: true }),
            allow_force_pushes: Some(SimpleEnabled { enabled: false }),
            allow_deletions: Some(SimpleEnabled { enabled: false }),
            block_creations: None,
            required_conversation_resolution: None,
            lock_branch: None,
            allow_fork_syncing: None,
        }
    }

    #[tokio::test]
    async fn backup_branches_disabled_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default();

        backup_branches(
            &client,
            "owner",
            "repo",
            &opts,
            &PathBuf::from(META),
            &storage,
        )
        .await
        .expect("ok");

        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_branches_dry_run_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions {
            branches: true,
            dry_run: true,
            ..Default::default()
        };

        backup_branches(
            &client,
            "owner",
            "repo",
            &opts,
            &PathBuf::from(META),
            &storage,
        )
        .await
        .expect("ok");

        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_branches_enabled_writes_json() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions {
            branches: true,
            ..Default::default()
        };

        backup_branches(
            &client,
            "owner",
            "repo",
            &opts,
            &PathBuf::from(META),
            &storage,
        )
        .await
        .expect("ok");

        assert!(storage
            .get(&PathBuf::from(format!("{META}/branches.json")))
            .is_some());
    }

    #[tokio::test]
    async fn backup_branches_saves_protection_for_protected_branches() {
        let mut protections = HashMap::new();
        protections.insert("main".to_string(), make_protection("main"));

        let client = MockBackupClient::new()
            .with_branches(vec![
                make_branch("main", true),
                make_branch("feature-x", false),
            ])
            .with_branch_protections(protections);

        let storage = MemStorage::default();
        let opts = BackupOptions {
            branches: true,
            ..Default::default()
        };

        backup_branches(
            &client,
            "owner",
            "repo",
            &opts,
            &PathBuf::from(META),
            &storage,
        )
        .await
        .expect("ok");

        // Should have both branches.json and branch_protections.json.
        assert!(storage
            .get(&PathBuf::from(format!("{META}/branches.json")))
            .is_some());
        assert!(storage
            .get(&PathBuf::from(format!("{META}/branch_protections.json")))
            .is_some());
    }

    #[tokio::test]
    async fn backup_branches_no_protection_for_unprotected_branches() {
        let client = MockBackupClient::new()
            .with_branches(vec![make_branch("main", false)]);

        let storage = MemStorage::default();
        let opts = BackupOptions {
            branches: true,
            ..Default::default()
        };

        backup_branches(
            &client,
            "owner",
            "repo",
            &opts,
            &PathBuf::from(META),
            &storage,
        )
        .await
        .expect("ok");

        // Only branches.json; no protection file for unprotected branches.
        assert!(storage
            .get(&PathBuf::from(format!("{META}/branches.json")))
            .is_some());
        assert!(storage
            .get(&PathBuf::from(format!("{META}/branch_protections.json")))
            .is_none());
    }
}
