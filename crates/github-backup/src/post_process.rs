// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Post-processing steps that run after the primary backup completes.
//!
//! This module encapsulates the optional post-processing phases:
//!
//! 1. **Diff** — compare the current backup to a previous snapshot directory.
//! 2. **Mirror push** — push every cloned repository to a Gitea or GitLab instance.
//! 3. **S3 sync** — upload backup artefacts to an S3-compatible object store.
//!
//! (Prometheus metrics live in `metrics`.)

use thiserror::Error;
use tracing::{info, warn};
use zeroize::Zeroizing;

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
#[derive(Debug, Error)]
pub enum PostProcessError {
    /// A mirror push operation failed.
    #[error("mirror push failed: {0}")]
    Mirror(String),
    /// An S3 sync operation failed.
    #[error("S3 sync failed: {0}")]
    S3(String),
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

    let stats = sync_to_s3(
        &client,
        config,
        &backup_root,
        include_assets,
        encrypt_key,
        delete_stale,
    )
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
/// The returned key bytes are wrapped in [`Zeroizing`] so that they are
/// securely erased from memory when dropped, preventing the key from
/// lingering in process memory.
///
/// # Errors
///
/// Returns a descriptive string on invalid input.
pub fn decode_encrypt_key(hex_key: Option<&str>) -> Result<Option<Zeroizing<[u8; 32]>>, String> {
    let Some(hex) = hex_key else {
        return Ok(None);
    };
    if hex.len() != 64 {
        return Err(format!(
            "--encrypt-key must be exactly 64 hex characters (32 bytes); got {} chars",
            hex.len()
        ));
    }
    let mut key = Zeroizing::new([0u8; 32]);
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let byte_str = std::str::from_utf8(chunk)
            .map_err(|_| "--encrypt-key contains non-UTF-8 characters".to_string())?;
        key[i] = u8::from_str_radix(byte_str, 16)
            .map_err(|_| format!("--encrypt-key contains non-hex character in '{byte_str}'"))?;
    }
    Ok(Some(key))
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
    fn run_diff_reports_what_the_engine_writes_to_repos_json() {
        // The engine writes `repos.json` as an array of repository objects.
        let prev = tempdir().unwrap();
        let curr = tempdir().unwrap();
        fs::write(
            prev.path().join("repos.json"),
            r#"[{"id":1,"name":"kept"},{"id":2,"name":"gone"}]"#,
        )
        .unwrap();
        fs::write(
            curr.path().join("repos.json"),
            r#"[{"id":1,"name":"kept"},{"id":3,"name":"new"}]"#,
        )
        .unwrap();
        let summary = run_diff(prev.path(), curr.path()).unwrap();
        assert!(summary.contains("1 added, 1 removed"), "{summary}");
        assert!(summary.contains("added:   new"), "{summary}");
        assert!(summary.contains("removed: gone"), "{summary}");
    }
}
