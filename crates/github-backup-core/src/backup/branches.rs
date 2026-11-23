// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository branch-list, branch-protection and ruleset backup.
//!
//! Saves `branches.json` containing the name and SHA of every branch in the
//! repository.  When any branch has `protected: true`, also fetches the
//! detailed protection rules for each protected branch and saves them to
//! `branch_protections.json`.  The repository rulesets — where GitHub keeps
//! modern branch and tag protection, including rules an organisation
//! applies — are saved to `rulesets.json`.  All three are snapshots of the
//! current configuration, replaced on every run.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;
use tracing::{debug, info};

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{
    backup::{is_forbidden, is_gone, Outcome},
    error::CoreError,
    storage::Storage,
};

/// Backs up repository branches, branch-protection rules and rulesets.
///
/// Saves `branches.json`.  When any branch has `protected: true`, also saves
/// `branch_protections.json` mapping branch name → protection rules, each
/// exactly as the API returned it (sorted by branch name, so the file is
/// deterministic).  A protected branch whose rules answer 404 (protection
/// that comes from a ruleset) is left out.  Then saves `rulesets.json`: every
/// ruleset that applies to the repository, in full, ordered by `id`; a server
/// without the rulesets API (404) is skipped.
///
/// Reading protection rules needs admin access.  A 403 refusal is returned
/// as the error of the category after the other files are written, so the
/// engine reports it like any other refusal.  A ruleset that cannot be
/// fetched is recorded in [`Outcome::failed`].
///
/// Skipped when `opts.branches` is `false` or `opts.dry_run` is `true`.
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
) -> Result<Outcome, CoreError> {
    let mut outcome = Outcome::default();
    if !opts.branches || opts.dry_run {
        return Ok(outcome);
    }

    info!(owner, repo, "fetching branches");
    let branches = client.list_branches(owner, repo).await?;
    storage.write_json(&meta_dir.join("branches.json"), &branches)?;

    let mut refused: Option<CoreError> = None;
    let protected: Vec<&str> = branches
        .iter()
        .filter(|b| b.protected)
        .map(|b| b.name.as_str())
        .collect();
    if !protected.is_empty() {
        debug!(
            owner,
            repo,
            count = protected.len(),
            "fetching branch protection rules"
        );
        let mut protections = BTreeMap::new();
        for branch_name in protected {
            match client.get_branch_protection(owner, repo, branch_name).await {
                Ok(rules) => {
                    protections.insert(branch_name.to_string(), rules);
                }
                Err(e) => {
                    let e = CoreError::from(e);
                    if is_forbidden(&e) {
                        // Every further branch would be refused the same way.
                        refused = Some(e);
                        break;
                    }
                    outcome.item_error(format!("branch {branch_name} protection"), e)?;
                }
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
    }

    backup_rulesets(client, owner, repo, meta_dir, storage, &mut outcome).await?;

    match refused {
        Some(e) => Err(e),
        None => Ok(outcome),
    }
}

/// Writes `rulesets.json`: the full object of every ruleset that applies to
/// the repository.
async fn backup_rulesets(
    client: &impl BackupClient,
    owner: &str,
    repo: &str,
    meta_dir: &Path,
    storage: &impl Storage,
    outcome: &mut Outcome,
) -> Result<(), CoreError> {
    let rulesets = match client.list_rulesets(owner, repo).await {
        Ok(rulesets) => rulesets,
        Err(e) => {
            let e = CoreError::from(e);
            if is_gone(&e) {
                debug!(owner, repo, "no rulesets API; skipping rulesets");
                return Ok(());
            }
            return Err(e);
        }
    };
    let mut full: BTreeMap<u64, Value> = BTreeMap::new();
    for ruleset in &rulesets {
        match client.get_ruleset(owner, repo, ruleset.id).await {
            Ok(detail) => {
                full.insert(ruleset.id, detail);
            }
            Err(e) => outcome.item_error(format!("ruleset {}", ruleset.id), e)?,
        }
    }
    // A summary the typed model could not read is kept as listed.
    let mut written: Vec<Value> = full.into_values().collect();
    written.extend(rulesets.unparsed().iter().cloned());
    storage.write_json(&meta_dir.join("rulesets.json"), &written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::{Branch, BranchCommit};
    use serde_json::json;
    use std::collections::HashMap;
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

    /// Any shape GitHub sends is stored: nothing about it is modelled.
    fn make_protection(branch: &str) -> Value {
        json!({
            "url": format!("https://api.github.com/repos/owner/repo/branches/{branch}/protection"),
            "required_linear_history": {"enabled": true},
            "a_property_added_next_year": [1, 2]
        })
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
        let client = MockBackupClient::new().with_branches(vec![make_branch("main", false)]);

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

    fn opts() -> BackupOptions {
        BackupOptions {
            branches: true,
            ..Default::default()
        }
    }

    fn read(storage: &MemStorage, name: &str) -> Value {
        serde_json::from_slice(
            &storage
                .get(&PathBuf::from(format!("{META}/{name}")))
                .unwrap_or_else(|| panic!("{name} written")),
        )
        .expect("json")
    }

    #[tokio::test]
    async fn rulesets_are_saved_in_full_in_id_order() {
        let client = MockBackupClient::new().with_raw(
            "rulesets",
            vec![
                json!({"id": 9, "name": "tags", "source_type": "Organization"}),
                json!({"id": 3, "name": "main"}),
            ],
        );
        let storage = MemStorage::default();
        let outcome = backup_branches(&client, "o", "r", &opts(), &PathBuf::from(META), &storage)
            .await
            .expect("ok");
        assert!(outcome.failed.is_empty());
        let rulesets = read(&storage, "rulesets.json");
        assert_eq!(rulesets[0]["id"], 3);
        assert_eq!(rulesets[1]["source_type"], "Organization");
        assert!(
            rulesets[0].get("rules").is_some(),
            "the detail, not the summary"
        );
    }

    #[tokio::test]
    async fn a_server_without_rulesets_is_not_a_failure() {
        let client = MockBackupClient::new().with_error("rulesets", 404);
        let storage = MemStorage::default();
        backup_branches(&client, "o", "r", &opts(), &PathBuf::from(META), &storage)
            .await
            .expect("404 means no rulesets API");
        assert!(storage
            .get(&PathBuf::from(format!("{META}/rulesets.json")))
            .is_none());
    }

    #[tokio::test]
    async fn a_ruleset_that_fails_is_recorded_and_the_others_kept() {
        let client = MockBackupClient::new()
            .with_raw("rulesets", vec![json!({"id": 1}), json!({"id": 2})])
            .with_error("ruleset:1", 500);
        let storage = MemStorage::default();
        let outcome = backup_branches(&client, "o", "r", &opts(), &PathBuf::from(META), &storage)
            .await
            .expect("ok");
        assert_eq!(outcome.failed.len(), 1);
        assert_eq!(outcome.failed[0].0, "ruleset 1");
        assert_eq!(read(&storage, "rulesets.json")[0]["id"], 2);
    }

    /// Without admin access the protection rules are refused: the branches
    /// and rulesets are still written, and the refusal is the category's
    /// error for the engine to judge.
    #[tokio::test]
    async fn refused_protection_is_returned_after_the_rest_is_written() {
        let client = MockBackupClient::new()
            .with_branches(vec![make_branch("main", true), make_branch("dev", true)])
            .with_error("branch_protection", 403);
        let storage = MemStorage::default();
        let err = backup_branches(&client, "o", "r", &opts(), &PathBuf::from(META), &storage)
            .await
            .expect_err("refused");
        assert!(crate::backup::is_forbidden(&err));
        assert!(storage
            .get(&PathBuf::from(format!("{META}/branches.json")))
            .is_some());
        assert!(storage
            .get(&PathBuf::from(format!("{META}/rulesets.json")))
            .is_some());
        let asked = client
            .calls()
            .iter()
            .filter(|c| c.starts_with("branch_protection:"))
            .count();
        assert_eq!(asked, 1, "stops asking once refused");
    }

    /// A protection object of any shape is stored as is; a strict typed model
    /// used to fail the whole step on any property it did not expect.
    #[tokio::test]
    async fn protection_rules_are_stored_verbatim() {
        let mut protections = HashMap::new();
        protections.insert("main".to_string(), make_protection("main"));
        let client = MockBackupClient::new()
            .with_branches(vec![make_branch("main", true)])
            .with_branch_protections(protections);
        let storage = MemStorage::default();
        backup_branches(&client, "o", "r", &opts(), &PathBuf::from(META), &storage)
            .await
            .expect("ok");
        assert_eq!(
            read(&storage, "branch_protections.json")["main"],
            make_protection("main")
        );
    }
}
