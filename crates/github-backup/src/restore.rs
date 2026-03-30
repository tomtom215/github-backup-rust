// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! `--restore` mode: re-create backed-up metadata in a target GitHub
//! organisation or repository.
//!
//! # What is restored
//!
//! | Artefact | Source JSON | Target API |
//! |----------|-------------|------------|
//! | Labels   | `json/repos/<repo>/labels.json` | `POST /repos/{org}/{repo}/labels` |
//! | Milestones | `json/repos/<repo>/milestones.json` | `POST /repos/{org}/{repo}/milestones` |
//!
//! Issues and pull requests are **informational** — GitHub does not expose a
//! public bulk-import API.  Restore those via
//! [`gh import`](https://cli.github.com/manual/gh_issue_import) or the GitHub
//! Enterprise Migrations API.
//!
//! # Non-destructive
//!
//! The restore operation is **additive only**.  It never deletes or modifies
//! existing labels or milestones in the target.  If a resource already exists
//! (HTTP 422 "already exists"), it is silently skipped.
//!
//! # Usage
//!
//! ```text
//! github-backup octocat --token ghp_xxx --output /backup \
//!     --restore --restore-target-org new-org
//! ```

use std::path::Path;

use tracing::{info, warn};

use github_backup_client::GitHubClient;
use github_backup_types::{Label, Milestone, OutputConfig};

/// Statistics collected during a restore operation.
#[derive(Debug, Default)]
pub struct RestoreStats {
    /// Labels successfully created.
    pub labels_created: usize,
    /// Labels skipped (already existed).
    pub labels_skipped: usize,
    /// Labels that failed with an unexpected error.
    pub labels_errored: usize,
    /// Milestones successfully created.
    pub milestones_created: usize,
    /// Milestones skipped (already existed).
    pub milestones_skipped: usize,
    /// Milestones that failed with an unexpected error.
    pub milestones_errored: usize,
}

impl std::fmt::Display for RestoreStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "labels: {} created, {} skipped, {} errored | \
             milestones: {} created, {} skipped, {} errored",
            self.labels_created,
            self.labels_skipped,
            self.labels_errored,
            self.milestones_created,
            self.milestones_skipped,
            self.milestones_errored,
        )
    }
}

/// Runs the restore operation.
///
/// Reads backed-up JSON from the `source_owner` backup directory and recreates
/// labels and milestones for every repository in `target_org`.
///
/// # Errors
///
/// Returns a string error if the backup directory cannot be read.  Per-repo
/// or per-resource errors are logged as warnings and counted in
/// [`RestoreStats`] rather than aborting the restore.
pub async fn run_restore(
    client: &GitHubClient,
    output: &OutputConfig,
    source_owner: &str,
    target_org: &str,
    _api_url: Option<&str>,
) -> Result<(), String> {
    info!(
        source_owner,
        target_org, "starting restore of labels and milestones"
    );

    let repos_meta_dir = output.owner_json_dir(source_owner).join("repos");
    if !repos_meta_dir.exists() {
        warn!(
            dir = %repos_meta_dir.display(),
            "no repos metadata directory found; nothing to restore"
        );
        return Ok(());
    }

    let repo_entries = std::fs::read_dir(&repos_meta_dir)
        .map_err(|e| format!("read repos dir {}: {e}", repos_meta_dir.display()))?;

    let mut total = RestoreStats::default();

    for entry in repo_entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let repo_name = entry.file_name().into_string().unwrap_or_default();
        if repo_name.is_empty() {
            continue;
        }

        let meta_dir = entry.path();
        let stats = restore_repo(client, &meta_dir, target_org, &repo_name).await;

        total.labels_created += stats.labels_created;
        total.labels_skipped += stats.labels_skipped;
        total.labels_errored += stats.labels_errored;
        total.milestones_created += stats.milestones_created;
        total.milestones_skipped += stats.milestones_skipped;
        total.milestones_errored += stats.milestones_errored;
    }

    info!(%total, "restore complete");
    Ok(())
}

/// Restores labels and milestones for a single repository.
async fn restore_repo(
    client: &GitHubClient,
    meta_dir: &Path,
    target_org: &str,
    repo_name: &str,
) -> RestoreStats {
    let mut stats = RestoreStats::default();

    // ── Labels ─────────────────────────────────────────────────────────────
    let labels_path = meta_dir.join("labels.json");
    if labels_path.exists() {
        match load_json::<Vec<Label>>(&labels_path) {
            Ok(labels) => {
                info!(
                    repo = %repo_name,
                    count = labels.len(),
                    "restoring labels"
                );
                for label in &labels {
                    match client
                        .create_label(
                            target_org,
                            repo_name,
                            &label.name,
                            &label.color,
                            label.description.as_deref(),
                        )
                        .await
                    {
                        Ok(_) => {
                            stats.labels_created += 1;
                        }
                        Err(github_backup_client::ClientError::ApiError {
                            status: 422, ..
                        }) => {
                            // 422 = Unprocessable Entity = already exists
                            stats.labels_skipped += 1;
                        }
                        Err(e) => {
                            warn!(
                                repo = %repo_name,
                                label = %label.name,
                                error = %e,
                                "failed to restore label"
                            );
                            stats.labels_errored += 1;
                        }
                    }
                }
            }
            Err(e) => {
                warn!(repo = %repo_name, error = %e, "failed to read labels.json");
            }
        }
    }

    // ── Milestones ─────────────────────────────────────────────────────────
    let milestones_path = meta_dir.join("milestones.json");
    if milestones_path.exists() {
        match load_json::<Vec<Milestone>>(&milestones_path) {
            Ok(milestones) => {
                info!(
                    repo = %repo_name,
                    count = milestones.len(),
                    "restoring milestones"
                );
                for ms in &milestones {
                    match client
                        .create_milestone(
                            target_org,
                            repo_name,
                            &ms.title,
                            ms.description.as_deref(),
                            Some(ms.state.as_str()),
                            ms.due_on.as_deref(),
                        )
                        .await
                    {
                        Ok(_) => {
                            stats.milestones_created += 1;
                        }
                        Err(github_backup_client::ClientError::ApiError {
                            status: 422, ..
                        }) => {
                            stats.milestones_skipped += 1;
                        }
                        Err(e) => {
                            warn!(
                                repo = %repo_name,
                                milestone = %ms.title,
                                error = %e,
                                "failed to restore milestone"
                            );
                            stats.milestones_errored += 1;
                        }
                    }
                }
            }
            Err(e) => {
                warn!(repo = %repo_name, error = %e, "failed to read milestones.json");
            }
        }
    }

    stats
}

/// Reads and deserialises a JSON file.
fn load_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_str(&content).map_err(|e| format!("parse {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_stats_display_format() {
        let stats = RestoreStats {
            labels_created: 5,
            labels_skipped: 2,
            labels_errored: 1,
            milestones_created: 3,
            milestones_skipped: 0,
            milestones_errored: 0,
        };
        let s = stats.to_string();
        assert!(s.contains("5 created"));
        assert!(s.contains("2 skipped"));
        assert!(s.contains("1 errored"));
        assert!(s.contains("milestones: 3 created"));
    }

    #[test]
    fn restore_stats_default_is_zero() {
        let stats = RestoreStats::default();
        assert_eq!(stats.labels_created, 0);
        assert_eq!(stats.milestones_created, 0);
    }

    #[test]
    fn load_json_valid() {
        use std::io::Write as _;
        use tempfile::NamedTempFile;
        let mut f = NamedTempFile::new().unwrap();
        write!(f, r#"["hello","world"]"#).unwrap();
        let v: Vec<String> = load_json(f.path()).unwrap();
        assert_eq!(v, vec!["hello", "world"]);
    }

    #[test]
    fn load_json_invalid_returns_error() {
        use std::io::Write as _;
        use tempfile::NamedTempFile;
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "not json").unwrap();
        let result = load_json::<Vec<String>>(f.path());
        assert!(result.is_err());
    }
}
