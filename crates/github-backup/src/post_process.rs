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
use github_backup_s3::{
    config::S3Config,
    sync::{sync_to_s3, SyncOptions, SyncReport},
    S3Client,
};
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

/// Names of the repositories the backup's `repos.json` lists as public.
///
/// Mirrors are created private unless the source is known to be public, so a
/// missing or unreadable listing means "everything private", never the reverse.
fn public_repo_names(repos_json: &std::path::Path) -> std::collections::HashSet<String> {
    let Ok(text) = std::fs::read_to_string(repos_json) else {
        return Default::default();
    };
    let Ok(serde_json::Value::Array(repos)) = serde_json::from_str(&text) else {
        return Default::default();
    };
    repos
        .iter()
        .filter(|r| r.get("private").and_then(serde_json::Value::as_bool) == Some(false))
        .filter_map(|r| r.get("name").and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .collect()
}

/// Turns the per-repository results of a mirror push into the run's outcome:
/// any failed repository makes the push an error (the CLI maps it to a
/// non-zero exit status), with every failure named.
fn mirror_outcome(stats: &github_backup_mirror::runner::MirrorStats) -> Result<(), String> {
    if stats.errored == 0 {
        return Ok(());
    }
    Err(format!(
        "{} of {} repositories failed to push: {}",
        stats.errored,
        stats.errored + stats.pushed,
        stats.failures.join("; ")
    ))
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
    let public_repos = public_repo_names(&output.owner_json(owner, "repos.json"));
    let stats = push_mirrors(
        &client,
        config,
        &repos_dir,
        &description_prefix,
        &public_repos,
    )
    .await
    .map_err(|e| e.to_string())?;

    info!(
        pushed = stats.pushed,
        errored = stats.errored,
        "Gitea mirror push complete"
    );

    mirror_outcome(&stats)
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
    let public_repos = public_repo_names(&output.owner_json(owner, "repos.json"));
    let stats = push_mirrors_gitlab(
        &client,
        config,
        &repos_dir,
        &description_prefix,
        &public_repos,
    )
    .await
    .map_err(|e| e.to_string())?;

    info!(
        pushed = stats.pushed,
        errored = stats.errored,
        "GitLab mirror push complete"
    );

    mirror_outcome(&stats)
}

/// Options for [`run_s3_sync_with`].
#[derive(Debug, Clone, Copy, Default)]
pub struct S3RunOptions<'a> {
    /// Also upload release assets.
    pub include_assets: bool,
    /// Encrypt every file with this AES-256 key.
    pub encrypt_key: Option<&'a [u8; 32]>,
    /// Delete remote objects whose local file is gone (`--s3-delete-stale`).
    pub delete_stale: bool,
    /// Master switch for deletion.  Pass `false` when the backup run had
    /// failures: an incomplete local copy must never remove a good remote one.
    pub allow_delete: bool,
    /// `--dry-run`: list what would be uploaded or deleted, write nothing.
    pub dry_run: bool,
}

/// Syncs the local JSON metadata (and optionally release assets) to S3 and
/// returns the full [`SyncReport`].
///
/// Objects are stored as `<prefix>/<owner>/json/<relative path>`.  Unchanged
/// files are detected by a content digest stored with each object.
///
/// # Errors
///
/// Returns [`PostProcessError::S3`] if the settings are unusable (for
/// example missing credentials), the client cannot be created, **or any
/// upload, listing or deletion failed**.  The message names the failed keys,
/// the HTTP status, the S3 error code and a hint.
pub async fn run_s3_sync_with(
    config: &S3Config,
    output: &OutputConfig,
    owner: &str,
    options: &S3RunOptions<'_>,
) -> Result<SyncReport, PostProcessError> {
    check_s3_config(config).map_err(PostProcessError::S3)?;
    let client =
        S3Client::new(config.clone()).map_err(|e| PostProcessError::S3(s3_error_text(&e)))?;
    let backup_root = output.owner_json_dir(owner);

    if !backup_root.exists() {
        warn!(dir = %backup_root.display(), "backup directory does not exist; skipping S3 sync");
        return Ok(SyncReport::default());
    }

    info!("S3 sync uploads the JSON metadata (and release assets with --s3-include-assets); repository clones are not uploaded");
    let key_root = format!("{owner}/json");
    let sync_options = SyncOptions::new(&backup_root, &key_root)
        .include_binary_assets(options.include_assets)
        .encrypt_key(options.encrypt_key)
        .delete_stale(options.delete_stale)
        .allow_delete(options.allow_delete)
        .dry_run(options.dry_run);
    let report = sync_to_s3(&client, config, &sync_options)
        .await
        .map_err(|e| PostProcessError::S3(s3_error_text(&e)))?;

    if report.dry_run {
        info!(
            would_upload = report.would_upload.len(),
            skipped = report.stats.skipped,
            would_delete = report.would_delete.len(),
            "S3 dry run complete (nothing was written)"
        );
    } else {
        info!(
            uploaded = report.stats.uploaded,
            skipped = report.stats.skipped,
            errored = report.stats.errored,
            deleted = report.stats.deleted,
            "S3 sync complete"
        );
    }
    if let Some(reason) = &report.deletion_skipped {
        warn!(reason = %reason, "--s3-delete-stale did nothing");
    }
    if report.is_success() {
        Ok(report)
    } else {
        Err(PostProcessError::S3(summarize_failures(&report)))
    }
}

/// Syncs the local backup JSON metadata (and optionally release assets) to S3.
///
/// Convenience form of [`run_s3_sync_with`] for a real (non-dry) run with
/// deletion allowed.
///
/// # Errors
///
/// Returns [`PostProcessError::S3`] on any failure, including a single failed
/// upload or deletion.
#[cfg(test)]
pub async fn run_s3_sync(
    config: &S3Config,
    output: &OutputConfig,
    owner: &str,
    include_assets: bool,
    encrypt_key: Option<&[u8; 32]>,
    delete_stale: bool,
) -> Result<(), PostProcessError> {
    run_s3_sync_with(
        config,
        output,
        owner,
        &S3RunOptions {
            include_assets,
            encrypt_key,
            delete_stale,
            allow_delete: true,
            dry_run: false,
        },
    )
    .await
    .map(drop)
}

/// Renders an [`S3Error`](github_backup_s3::S3Error) with its hint.
fn s3_error_text(error: &github_backup_s3::S3Error) -> String {
    match error.hint() {
        Some(hint) => format!("{error} (hint: {hint})"),
        None => error.to_string(),
    }
}

/// One-paragraph description of everything that failed in `report`.
fn summarize_failures(report: &SyncReport) -> String {
    const SHOWN: usize = 5;
    let mut text = match &report.aborted {
        Some(reason) => format!(
            "aborted: {reason}; {} file(s) were not attempted",
            report.not_attempted
        ),
        None => format!(
            "{} operation(s) failed ({})",
            report.failures.len(),
            report.stats
        ),
    };
    for failure in report.failures.iter().take(SHOWN) {
        text.push_str(&format!("; {failure}"));
    }
    if report.failures.len() > SHOWN {
        text.push_str(&format!("; and {} more", report.failures.len() - SHOWN));
    }
    text
}

/// Whether every mirror is forced private: unless `--mirror-public` was given,
/// and always when `--mirror-private` was (a config file may set both).
fn mirror_forced_private(args: &Args) -> bool {
    args.mirror_private || !args.mirror_public
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
            private: mirror_forced_private(args),
        })),
        _ => Some(MirrorDest::Gitea(GiteaConfig {
            base_url,
            token,
            owner,
            private: mirror_forced_private(args),
        })),
    }
}

/// Builds an [`S3Config`] from CLI args, or returns `None` if no S3 bucket
/// is configured.
///
/// Blank values (an empty `AWS_SESSION_TOKEN` forwarded by a container
/// launcher, for instance) count as unset.  Missing credentials are *not*
/// rejected here; [`check_s3_config`] (called by [`run_s3_sync_with`]) does
/// that with an actionable message.
#[must_use]
pub fn build_s3_config(args: &Args) -> Option<S3Config> {
    let bucket = args.s3_bucket.clone()?;
    let region = args
        .s3_region
        .clone()
        .filter(|r| !r.trim().is_empty())
        .unwrap_or_else(|| "us-east-1".to_string());
    let prefix = args.s3_prefix.clone().unwrap_or_default();
    let access_key_id = args.s3_access_key.clone().unwrap_or_default();
    let secret_access_key = args.s3_secret_key.clone().unwrap_or_default();
    let session_token = args
        .s3_session_token
        .clone()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());

    Some(S3Config {
        bucket,
        region,
        prefix,
        endpoint: args.s3_endpoint.clone().filter(|e| !e.trim().is_empty()),
        access_key_id,
        secret_access_key,
        session_token,
    })
}

/// Validates an [`S3Config`] before any request is made.
///
/// # Errors
///
/// Returns a message that says which setting is missing or malformed and how
/// to supply it.
pub fn check_s3_config(config: &S3Config) -> Result<(), String> {
    let missing_key = config.access_key_id.trim().is_empty();
    let missing_secret = config.secret_access_key.trim().is_empty();
    if missing_key || missing_secret {
        let what = match (missing_key, missing_secret) {
            (true, true) => "an access key id and a secret access key",
            (true, false) => "an access key id",
            _ => "a secret access key",
        };
        return Err(format!(
            "--s3-bucket is set but {what} is missing; provide AWS_ACCESS_KEY_ID and \
             AWS_SECRET_ACCESS_KEY (environment variables are safer than flags), or \
             --s3-access-key / --s3-secret-key, or s3_access_key / s3_secret_key in the config file"
        ));
    }
    config.validate().map_err(|e| s3_error_text(&e))
}

/// Decodes a hex-encoded 32-byte AES-256 key from the `--encrypt-key` string.
///
/// Returns `None` if no key is set, or `Err` if the string is not exactly
/// 64 hex characters that decode to 32 bytes.
///
/// The returned key bytes are wrapped in [`Zeroizing`], so that buffer is
/// overwritten when dropped.  This does not reach copies the process cannot
/// control: the hex string held by the argument parser, the environment and
/// the command line.  Error messages never contain any character of the key.
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
            "--encrypt-key must be exactly 64 hex characters (32 bytes); got {} bytes",
            hex.len()
        ));
    }
    fn nibble(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }
    let mut key = Zeroizing::new([0u8; 32]);
    for (i, pair) in hex.as_bytes().chunks(2).enumerate() {
        let (hi, lo) = (nibble(pair[0]), nibble(pair[1]));
        match (hi, lo) {
            (Some(hi), Some(lo)) => key[i] = hi * 16 + lo,
            _ => {
                return Err(format!(
                    "--encrypt-key must contain only the hex digits 0-9 and a-f \
                     (a different character was found near position {})",
                    i * 2 + 1
                ))
            }
        }
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

    fn forced_private(extra: &[&str]) -> bool {
        let mut argv = vec![
            "github-backup",
            "octocat",
            "--token",
            "t",
            "--mirror-to",
            "https://codeberg.example",
        ];
        argv.extend(extra);
        let args = crate::cli::test_support::parse(&argv);
        match build_mirror_dest(&args).expect("a destination") {
            MirrorDest::Gitea(c) => c.private,
            MirrorDest::GitLab(c) => c.private,
        }
    }

    /// Mirrors are private unless `--mirror-public` is given; and even then the
    /// runner only publishes repositories known to be public (`wants_private`).
    #[test]
    fn mirrors_are_forced_private_unless_mirror_public_is_given() {
        assert!(forced_private(&[]), "default");
        assert!(forced_private(&["--mirror-private"]), "explicit");
        assert!(!forced_private(&["--mirror-public"]), "opt-in");
    }

    #[test]
    fn mirror_public_and_mirror_private_cannot_be_combined_on_the_command_line() {
        assert!(crate::cli::test_support::try_parse(&[
            "github-backup",
            "octocat",
            "--mirror-to",
            "https://codeberg.example",
            "--mirror-public",
            "--mirror-private",
        ])
        .is_err());
    }

    /// A config file that (wrongly) sets both resolves to private.
    #[test]
    fn a_config_that_sets_both_means_private() {
        let mut args = crate::cli::test_support::parse(&[
            "github-backup",
            "octocat",
            "--mirror-to",
            "https://codeberg.example",
        ]);
        let cfg = github_backup_types::config::ConfigFile::from_toml_str(
            "mirror_public = true\nmirror_private = true\n",
        )
        .expect("parses");
        args.merge_config(&cfg);
        assert!(build_mirror_dest(&args).is_some());
        assert!(mirror_forced_private(&args), "private wins");
    }

    /// The command line beats the file: `--mirror-private` ignores
    /// `mirror_public = true`, and `--mirror-public` ignores `mirror_private`.
    #[test]
    fn the_command_line_beats_the_config_file_for_mirror_visibility() {
        let cfg = github_backup_types::config::ConfigFile::from_toml_str(
            "mirror_public = true\nmirror_private = true\n",
        )
        .expect("parses");
        let mut private = crate::cli::test_support::parse(&[
            "github-backup",
            "octocat",
            "--mirror-to",
            "https://codeberg.example",
            "--mirror-private",
        ]);
        private.merge_config(&cfg);
        assert!(!private.mirror_public);
        assert!(mirror_forced_private(&private));

        let mut public = crate::cli::test_support::parse(&[
            "github-backup",
            "octocat",
            "--mirror-to",
            "https://codeberg.example",
            "--mirror-public",
        ]);
        public.merge_config(&cfg);
        assert!(!public.mirror_private);
        assert!(!mirror_forced_private(&public));
    }

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
    fn decode_encrypt_key_accepts_upper_case() {
        let hex = "AB".repeat(32);
        assert!(decode_encrypt_key(Some(&hex))
            .unwrap()
            .unwrap()
            .iter()
            .all(|&b| b == 0xab));
    }

    #[test]
    fn decode_encrypt_key_rejects_plus_and_whitespace_signs() {
        let hex = format!("+a{}", "aa".repeat(31));
        assert!(decode_encrypt_key(Some(&hex)).is_err());
        let hex = format!("-a{}", "aa".repeat(31));
        assert!(decode_encrypt_key(Some(&hex)).is_err());
    }

    #[test]
    fn decode_encrypt_key_errors_never_echo_key_characters() {
        // Distinct marker characters at known positions.
        let mut hex = "0123456789abcdef".repeat(4);
        hex.replace_range(20..22, "Zq");
        let err = decode_encrypt_key(Some(&hex)).unwrap_err();
        for fragment in ["Zq", "Z", "q", "0123", "abcdef", &hex] {
            assert!(!err.contains(fragment), "{err:?} leaks {fragment:?}");
        }
        let short = decode_encrypt_key(Some("deadbeefSECRET")).unwrap_err();
        assert!(
            !short.contains("deadbeef") && !short.contains("SECRET"),
            "{short}"
        );
    }

    fn s3_args(extra: &[&str]) -> Args {
        let mut argv = vec!["github-backup", "octocat", "--s3-bucket", "b"];
        argv.extend_from_slice(extra);
        crate::cli::test_support::parse(&argv)
    }

    #[test]
    fn build_s3_config_none_without_bucket() {
        let args = crate::cli::test_support::parse(&["github-backup", "octocat"]);
        assert!(build_s3_config(&args).is_none());
    }

    #[test]
    fn build_s3_config_carries_session_token_and_trims_blanks() {
        let args = s3_args(&["--s3-session-token", "  TOK  "]);
        assert_eq!(
            build_s3_config(&args).unwrap().session_token.as_deref(),
            Some("TOK")
        );
        let args = s3_args(&["--s3-session-token", "   "]);
        assert!(build_s3_config(&args).unwrap().session_token.is_none());
    }

    #[test]
    fn check_s3_config_explains_missing_credentials() {
        let args = s3_args(&[]);
        let err = check_s3_config(&build_s3_config(&args).unwrap()).unwrap_err();
        assert!(
            err.contains("AWS_ACCESS_KEY_ID") && err.contains("secret access key"),
            "{err}"
        );
        let args = s3_args(&["--s3-access-key", "AK"]);
        let err = check_s3_config(&build_s3_config(&args).unwrap()).unwrap_err();
        assert!(err.contains("a secret access key is missing"), "{err}");
        let args = s3_args(&["--s3-access-key", "AK", "--s3-secret-key", "SK"]);
        check_s3_config(&build_s3_config(&args).unwrap()).unwrap();
    }

    #[test]
    fn check_s3_config_rejects_bucket_urls() {
        let mut args = s3_args(&["--s3-access-key", "AK", "--s3-secret-key", "SK"]);
        args.s3_bucket = Some("s3://my-bucket".to_string());
        let err = check_s3_config(&build_s3_config(&args).unwrap()).unwrap_err();
        assert!(err.contains("bucket"), "{err}");
    }

    #[tokio::test]
    async fn run_s3_sync_without_credentials_is_an_error_before_any_request() {
        let dir = tempdir().unwrap();
        let output = OutputConfig::new(dir.path());
        fs::create_dir_all(output.owner_json_dir("octocat")).unwrap();
        let args = s3_args(&["--s3-endpoint", "http://127.0.0.1:9"]);
        let cfg = build_s3_config(&args).unwrap();
        let err = run_s3_sync(&cfg, &output, "octocat", false, None, false)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("AWS_ACCESS_KEY_ID"), "{err}");
    }

    #[tokio::test]
    async fn run_s3_sync_reports_a_failed_upload_as_an_error() {
        // Nothing listens on port 9: every upload fails, so the run must
        // return Err (the CLI turns that into a non-zero exit).
        let dir = tempdir().unwrap();
        let output = OutputConfig::new(dir.path());
        let json = output.owner_json_dir("octocat");
        fs::create_dir_all(&json).unwrap();
        fs::write(json.join("a.json"), b"{}").unwrap();
        let args = s3_args(&[
            "--s3-endpoint",
            "http://127.0.0.1:9",
            "--s3-access-key",
            "AK",
            "--s3-secret-key",
            "SK",
        ]);
        let cfg = build_s3_config(&args).unwrap();
        let err = run_s3_sync(&cfg, &output, "octocat", false, None, false)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("octocat/json/a.json") || err.contains("aborted"),
            "{err}"
        );
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

    #[test]
    fn only_repositories_listed_as_public_are_public() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("repos.json");
        fs::write(
            &path,
            r#"[{"name":"open","private":false},{"name":"secret","private":true},{"name":"odd"}]"#,
        )
        .unwrap();
        let public = public_repo_names(&path);
        assert!(public.contains("open"));
        assert!(!public.contains("secret"));
        assert!(!public.contains("odd"), "unknown visibility is not public");
    }

    #[test]
    fn missing_or_corrupt_listing_means_nothing_is_public() {
        let dir = tempdir().unwrap();
        assert!(public_repo_names(&dir.path().join("absent.json")).is_empty());
        let bad = dir.path().join("bad.json");
        fs::write(&bad, "not json").unwrap();
        assert!(public_repo_names(&bad).is_empty());
    }

    #[test]
    fn a_failed_repository_makes_the_mirror_push_an_error() {
        let ok = github_backup_mirror::runner::MirrorStats {
            pushed: 2,
            ..Default::default()
        };
        assert!(mirror_outcome(&ok).is_ok());
        let bad = github_backup_mirror::runner::MirrorStats {
            pushed: 1,
            errored: 1,
            failures: vec!["r2: boom".to_owned()],
        };
        let err = mirror_outcome(&bad).expect_err("must fail");
        assert!(err.contains("1 of 2") && err.contains("r2: boom"), "{err}");
    }
}
