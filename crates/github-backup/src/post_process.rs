// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Post-processing steps that run after the primary backup completes.
//!
//! This module encapsulates the four optional post-processing phases:
//!
//! 1. **Prometheus metrics** — write backup counters in text exposition format.
//! 2. **Diff** — compare the current backup to a previous snapshot directory.
//! 3. **Mirror push** — push every cloned repository to a Gitea or GitLab instance.
//! 4. **S3 sync** — upload backup artefacts to an S3-compatible object store.
//! 5. **Retention** — delete old snapshot directories matching `YYYY-MM-DD*`.

use thiserror::Error;
use tracing::{info, warn};

use github_backup_mirror::{
    config::{GitLabConfig, GiteaConfig},
    gitlab_runner::push_mirrors_gitlab,
    runner::push_mirrors,
    GitLabClient, GiteaClient,
};
use github_backup_s3::{config::S3Config, sync::sync_to_s3, S3Client};
use github_backup_types::config::OutputConfig;

use crate::cli::Args;

/// Typed errors from the post-processing phase.
///
/// Replaces the previous `Result<(), String>` returns on public post-process
/// functions, preserving source context and enabling structured handling.
#[derive(Debug, Error)]
pub enum PostProcessError {
    /// A mirror push operation failed.
    #[error("mirror push failed: {0}")]
    Mirror(String),
    /// An S3 sync operation failed.
    #[error("S3 sync failed: {0}")]
    S3(String),
    /// The retention policy application failed.
    #[error("retention policy failed: {0}")]
    Retention(String),
    /// The backup diff operation failed.
    #[error("diff failed: {0}")]
    Diff(String),
    /// Writing Prometheus metrics failed.
    #[error("metrics write failed: {0}")]
    Metrics(String),
}

/// Mirror destination — either a Gitea-compatible host or a GitLab instance.
pub enum MirrorDest {
    Gitea(GiteaConfig),
    GitLab(GitLabConfig),
}

/// Dispatches the mirror push to the appropriate runner.
///
/// # Errors
///
/// Returns [`PostProcessError::Mirror`] if the mirror client fails to
/// initialise or the push fails.
pub async fn run_mirror_push_dest(
    dest: &MirrorDest,
    output: &OutputConfig,
    owner: &str,
) -> Result<(), PostProcessError> {
    match dest {
        MirrorDest::Gitea(config) => run_mirror_push_gitea(config, output, owner)
            .await
            .map_err(PostProcessError::Mirror),
        MirrorDest::GitLab(config) => run_mirror_push_gitlab(config, output, owner)
            .await
            .map_err(PostProcessError::Mirror),
    }
}

/// Pushes repositories to a Gitea-compatible destination.
async fn run_mirror_push_gitea(
    config: &GiteaConfig,
    output: &OutputConfig,
    owner: &str,
) -> Result<(), String> {
    let client = GiteaClient::new(config.clone()).map_err(|e| e.to_string())?;
    let repos_dir = output.repos_dir(owner);

    if !repos_dir.exists() {
        warn!(dir = %repos_dir.display(), "repos directory does not exist; skipping mirror push");
        return Ok(());
    }

    let description_prefix = format!("GitHub mirror of {owner}/");
    let stats = push_mirrors(&client, config, &repos_dir, &description_prefix)
        .await
        .map_err(|e| e.to_string())?;

    info!(
        pushed = stats.pushed,
        errored = stats.errored,
        "Gitea mirror push complete"
    );

    if stats.errored > 0 {
        warn!(
            errored = stats.errored,
            "some repositories failed to push to Gitea mirror"
        );
    }

    Ok(())
}

/// Pushes repositories to a GitLab destination.
async fn run_mirror_push_gitlab(
    config: &GitLabConfig,
    output: &OutputConfig,
    owner: &str,
) -> Result<(), String> {
    let client = GitLabClient::new(config.clone()).map_err(|e| e.to_string())?;
    let repos_dir = output.repos_dir(owner);

    if !repos_dir.exists() {
        warn!(dir = %repos_dir.display(), "repos directory does not exist; skipping GitLab mirror push");
        return Ok(());
    }

    let description_prefix = format!("GitHub mirror of {owner}/");
    let stats = push_mirrors_gitlab(&client, config, &repos_dir, &description_prefix)
        .await
        .map_err(|e| e.to_string())?;

    info!(
        pushed = stats.pushed,
        errored = stats.errored,
        "GitLab mirror push complete"
    );

    if stats.errored > 0 {
        warn!(
            errored = stats.errored,
            "some repositories failed to push to GitLab mirror"
        );
    }

    Ok(())
}

/// Syncs the local backup JSON metadata (and optionally binary assets) to S3.
///
/// When `encrypt_key` is `Some`, every file is encrypted with AES-256-GCM
/// before upload.  The key must be a 32-byte slice derived from the
/// `--encrypt-key` hex string.
///
/// # Errors
///
/// Returns [`PostProcessError::S3`] if the S3 client fails to initialise or a
/// sync error is encountered.
pub async fn run_s3_sync(
    config: &S3Config,
    output: &OutputConfig,
    owner: &str,
    include_assets: bool,
    encrypt_key: Option<&[u8; 32]>,
    delete_stale: bool,
) -> Result<(), PostProcessError> {
    let client = S3Client::new(config.clone()).map_err(|e| PostProcessError::S3(e.to_string()))?;
    let backup_root = output.owner_json_dir(owner);

    if !backup_root.exists() {
        warn!(dir = %backup_root.display(), "backup directory does not exist; skipping S3 sync");
        return Ok(());
    }

    let stats =
        sync_to_s3(&client, config, &backup_root, include_assets, encrypt_key, delete_stale)
            .await
            .map_err(|e| PostProcessError::S3(e.to_string()))?;

    info!(
        uploaded = stats.uploaded,
        skipped = stats.skipped,
        errored = stats.errored,
        deleted = stats.deleted,
        "S3 sync complete"
    );

    if stats.errored > 0 {
        warn!(errored = stats.errored, "some files failed to upload to S3");
    }
    if stats.deleted > 0 {
        info!(deleted = stats.deleted, "stale S3 objects removed");
    }

    Ok(())
}

/// Builds a [`MirrorDest`] from CLI args, or returns `None` if no mirror
/// destination is configured.
#[must_use]
pub fn build_mirror_dest(args: &Args) -> Option<MirrorDest> {
    let base_url = args.mirror_to.clone()?;
    let token = args.mirror_token.clone().unwrap_or_default();
    let owner = args
        .mirror_owner
        .clone()
        .unwrap_or_else(|| args.owner.clone().unwrap_or_default());

    match args.mirror_type.as_str() {
        "gitlab" => Some(MirrorDest::GitLab(GitLabConfig {
            base_url,
            token,
            namespace: owner,
            private: args.mirror_private,
        })),
        _ => Some(MirrorDest::Gitea(GiteaConfig {
            base_url,
            token,
            owner,
            private: args.mirror_private,
        })),
    }
}

/// Builds an [`S3Config`] from CLI args, or returns `None` if no S3 bucket
/// is configured.
#[must_use]
pub fn build_s3_config(args: &Args) -> Option<S3Config> {
    let bucket = args.s3_bucket.clone()?;
    let region = args
        .s3_region
        .clone()
        .unwrap_or_else(|| "us-east-1".to_string());
    let prefix = args.s3_prefix.clone().unwrap_or_default();
    let access_key_id = args.s3_access_key.clone().unwrap_or_default();
    let secret_access_key = args.s3_secret_key.clone().unwrap_or_default();

    Some(S3Config {
        bucket,
        region,
        prefix,
        endpoint: args.s3_endpoint.clone(),
        access_key_id,
        secret_access_key,
    })
}

/// Decodes a hex-encoded 32-byte AES-256 key from the `--encrypt-key` string.
///
/// Returns `None` if no key is set, or `Err` if the string is not exactly
/// 64 hex characters that decode to 32 bytes.
///
/// # Errors
///
/// Returns a descriptive string on invalid input.
pub fn decode_encrypt_key(hex_key: Option<&str>) -> Result<Option<Box<[u8; 32]>>, String> {
    let Some(hex) = hex_key else {
        return Ok(None);
    };
    if hex.len() != 64 {
        return Err(format!(
            "--encrypt-key must be exactly 64 hex characters (32 bytes); got {} chars",
            hex.len()
        ));
    }
    let mut key = Box::new([0u8; 32]);
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let byte_str = std::str::from_utf8(chunk)
            .map_err(|_| "--encrypt-key contains non-UTF-8 characters".to_string())?;
        key[i] = u8::from_str_radix(byte_str, 16)
            .map_err(|_| format!("--encrypt-key contains non-hex character in '{byte_str}'"))?;
    }
    Ok(Some(key))
}

/// Writes Prometheus-format metrics to `path`.
///
/// # Errors
///
/// Returns a string error on directory creation or file write failure.
pub fn write_prometheus_metrics(
    path: &std::path::Path,
    owner: &str,
    stats: &github_backup_core::BackupStats,
    started_at_unix: u64,
) -> Result<(), String> {
    let mut out = String::new();
    let label = format!("owner=\"{owner}\"");

    out.push_str(&format!(
        "# HELP github_backup_repos_backed_up Number of repositories backed up\n\
         # TYPE github_backup_repos_backed_up gauge\n\
         github_backup_repos_backed_up{{{label}}} {}\n",
        stats.repos_backed_up()
    ));
    out.push_str(&format!(
        "# HELP github_backup_repos_discovered Number of repositories discovered\n\
         # TYPE github_backup_repos_discovered gauge\n\
         github_backup_repos_discovered{{{label}}} {}\n",
        stats.repos_discovered()
    ));
    out.push_str(&format!(
        "# HELP github_backup_repos_errored Repositories with backup errors\n\
         # TYPE github_backup_repos_errored gauge\n\
         github_backup_repos_errored{{{label}}} {}\n",
        stats.repos_errored()
    ));
    out.push_str(&format!(
        "# HELP github_backup_issues_fetched Total issues fetched\n\
         # TYPE github_backup_issues_fetched counter\n\
         github_backup_issues_fetched{{{label}}} {}\n",
        stats.issues_fetched()
    ));
    out.push_str(&format!(
        "# HELP github_backup_prs_fetched Total pull requests fetched\n\
         # TYPE github_backup_prs_fetched counter\n\
         github_backup_prs_fetched{{{label}}} {}\n",
        stats.prs_fetched()
    ));
    out.push_str(&format!(
        "# HELP github_backup_duration_seconds Duration of the last backup run in seconds\n\
         # TYPE github_backup_duration_seconds gauge\n\
         github_backup_duration_seconds{{{label}}} {:.3}\n",
        stats.elapsed_secs()
    ));
    out.push_str(&format!(
        "# HELP github_backup_last_success_timestamp_seconds Unix timestamp of the last successful backup start\n\
         # TYPE github_backup_last_success_timestamp_seconds gauge\n\
         github_backup_last_success_timestamp_seconds{{{label}}} {started_at_unix}\n"
    ));
    out.push_str(&format!(
        "# HELP github_backup_success Whether the last backup succeeded (1 = success, 0 = failure)\n\
         # TYPE github_backup_success gauge\n\
         github_backup_success{{{label}}} {}\n",
        if stats.repos_errored() == 0 { 1 } else { 0 }
    ));

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create metrics dir: {e}"))?;
    }
    std::fs::write(path, out).map_err(|e| format!("write metrics: {e}"))
}

/// Compares two backup JSON directories and returns a human-readable summary.
///
/// Reads `repos.json` from both directories and reports added/removed repos.
///
/// # Errors
///
/// Returns a string error if the repos.json files cannot be read or parsed.
pub fn run_diff(prev_dir: &std::path::Path, curr_dir: &std::path::Path) -> Result<String, String> {
    let prev_repos = read_repo_names(&prev_dir.join("repos.json"))?;
    let curr_repos = read_repo_names(&curr_dir.join("repos.json"))?;

    let added: Vec<_> = curr_repos
        .iter()
        .filter(|r| !prev_repos.contains(*r))
        .cloned()
        .collect();
    let removed: Vec<_> = prev_repos
        .iter()
        .filter(|r| !curr_repos.contains(*r))
        .cloned()
        .collect();

    let mut summary = format!(
        "repositories: {} → {} ({} added, {} removed)",
        prev_repos.len(),
        curr_repos.len(),
        added.len(),
        removed.len()
    );

    if !added.is_empty() {
        summary.push_str(&format!("\n  added:   {}", added.join(", ")));
    }
    if !removed.is_empty() {
        summary.push_str(&format!("\n  removed: {}", removed.join(", ")));
    }

    Ok(summary)
}

/// Reads repository names from a `repos.json` file.
///
/// Returns an empty vec if the file does not exist.
///
/// # Errors
///
/// Returns a string error if the file exists but cannot be read or parsed.
pub fn read_repo_names(path: &std::path::Path) -> Result<Vec<String>, String> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let repos: Vec<serde_json::Value> =
        serde_json::from_str(&content).map_err(|e| format!("parse repos.json: {e}"))?;
    Ok(repos
        .iter()
        .filter_map(|r| r.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .collect())
}

/// Applies the retention policy by deleting old snapshot directories.
///
/// Snapshots are detected as date-stamped directories matching `YYYY-MM-DD*`
/// directly under `output_root`.  Both `keep_last` and `max_age_days` can be
/// combined; whichever deletes more snapshots wins.
///
/// # Errors
///
/// Returns [`PostProcessError::Retention`] if the output directory cannot be
/// read or a snapshot directory cannot be deleted.
pub fn apply_retention(
    output_root: &std::path::Path,
    keep_last: Option<usize>,
    max_age_days: Option<u64>,
) -> Result<(), PostProcessError> {
    let entries = std::fs::read_dir(output_root)
        .map_err(|e| PostProcessError::Retention(format!("read output dir: {e}")))?;

    let mut snapshots: Vec<std::path::PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            // Match YYYY-MM-DD prefix
            if name.len() >= 10 && name.as_bytes()[4] == b'-' && name.as_bytes()[7] == b'-' {
                Some(e.path())
            } else {
                None
            }
        })
        .collect();

    snapshots.sort();

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut to_delete: std::collections::HashSet<std::path::PathBuf> = Default::default();

    if let Some(keep) = keep_last {
        if snapshots.len() > keep {
            let delete_count = snapshots.len() - keep;
            for path in snapshots.iter().take(delete_count) {
                to_delete.insert(path.clone());
            }
        }
    }

    if let Some(max_age) = max_age_days {
        let cutoff_secs = now_secs.saturating_sub(max_age * 86_400);
        for path in &snapshots {
            if let Ok(meta) = std::fs::metadata(path) {
                if let Ok(modified) = meta.modified() {
                    let mod_secs = modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(u64::MAX);
                    if mod_secs < cutoff_secs {
                        to_delete.insert(path.clone());
                    }
                }
            }
        }
    }

    for path in &to_delete {
        info!(path = %path.display(), "applying retention: deleting old snapshot");
        std::fs::remove_dir_all(path).map_err(|e| {
            PostProcessError::Retention(format!("delete snapshot {}: {e}", path.display()))
        })?;
    }

    if !to_delete.is_empty() {
        info!(deleted = to_delete.len(), "retention policy applied");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn decode_encrypt_key_none_returns_none() {
        assert!(decode_encrypt_key(None).unwrap().is_none());
    }

    #[test]
    fn decode_encrypt_key_valid_32_bytes() {
        let hex = "a".repeat(64);
        let key = decode_encrypt_key(Some(&hex)).unwrap().unwrap();
        assert_eq!(key.len(), 32);
        assert!(key.iter().all(|&b| b == 0xaa));
    }

    #[test]
    fn decode_encrypt_key_wrong_length_errors() {
        assert!(decode_encrypt_key(Some("aabb")).is_err());
    }

    #[test]
    fn decode_encrypt_key_non_hex_errors() {
        let hex = "zz".repeat(32);
        assert!(decode_encrypt_key(Some(&hex)).is_err());
    }

    #[test]
    fn run_diff_empty_dirs_summary() {
        let dir1 = tempdir().unwrap();
        let dir2 = tempdir().unwrap();
        let result = run_diff(dir1.path(), dir2.path()).unwrap();
        assert!(result.contains("0 added, 0 removed"));
    }

    #[test]
    fn apply_retention_no_snapshots_is_ok() {
        let dir = tempdir().unwrap();
        // Create a non-snapshot directory — should be left untouched.
        fs::create_dir(dir.path().join("config")).unwrap();
        apply_retention(dir.path(), Some(5), None).unwrap();
        assert!(dir.path().join("config").exists());
    }

    #[test]
    fn apply_retention_deletes_oldest_when_over_limit() {
        let dir = tempdir().unwrap();
        for name in &["2025-01-01", "2025-02-01", "2025-03-01", "2025-04-01"] {
            fs::create_dir(dir.path().join(name)).unwrap();
        }
        apply_retention(dir.path(), Some(2), None).unwrap();
        let remaining: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().into_string().unwrap())
            .collect();
        assert_eq!(remaining.len(), 2, "should keep only 2 snapshots");
        assert!(remaining.contains(&"2025-03-01".to_string()));
        assert!(remaining.contains(&"2025-04-01".to_string()));
    }
}
