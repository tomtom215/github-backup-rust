// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! `github-backup` binary entry point.

use std::io;
use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use clap_complete::{generate, Shell};
use tracing::{error, info, warn};

use github_backup_client::{oauth::device_flow, GitHubClient};
use github_backup_core::{
    verify_manifest, write_manifest, BackupEngine, FsStorage, ProcessGitRunner,
};
use github_backup_mirror::{
    config::{GitLabConfig, GiteaConfig},
    gitlab_runner::push_mirrors_gitlab,
    runner::push_mirrors,
    GitLabClient, GiteaClient,
};
use github_backup_s3::{config::S3Config, sync::sync_to_s3, S3Client};
use github_backup_tui::InitialConfig;
use github_backup_types::backup_state::BackupState;
use github_backup_types::config::{ConfigFile, Credential, OutputConfig};

mod cli;
mod report;

use cli::Args;
use report::{is_valid_iso8601, write_report};

#[tokio::main]
async fn main() -> ExitCode {
    // Check for --completions <shell> before full arg parsing so it works
    // even when required args (token, owner) are absent.
    if let Some(shell) = detect_completions_request() {
        generate(
            shell,
            &mut Args::command(),
            "github-backup",
            &mut io::stdout(),
        );
        return ExitCode::SUCCESS;
    }

    let mut args = Args::parse();

    // ── TUI mode ──────────────────────────────────────────────────────────────
    // When `--tui` is passed (or when no other meaningful flags are given and
    // we appear to be running in an interactive terminal), launch the TUI.
    if args.tui {
        let initial = InitialConfig {
            token: args.token.clone(),
            owner: args.owner.clone(),
            output: args.output.as_ref().map(|p| p.display().to_string()),
            api_url: args.api_url.clone(),
        };
        return github_backup_tui::run_tui(initial).await;
    }

    // Initialise structured logging early so config-file errors are logged.
    init_tracing(args.quiet, args.verbose);

    // ── Config file ────────────────────────────────────────────────────────
    if let Some(ref config_path) = args.config.clone() {
        match ConfigFile::from_path(config_path) {
            Ok(cfg) => {
                info!(path = %config_path.display(), "loaded config file");
                args.merge_config(&cfg);
            }
            Err(e) => {
                error!("{e}");
                return ExitCode::FAILURE;
            }
        }
    }

    // ── Auto state file for --since ────────────────────────────────────────
    // If --since was not supplied explicitly, try to load the last-success
    // timestamp from the state file for automatic incremental backups.
    if args.since.is_none() {
        if let Some(ref output_path) = args.output {
            if let Some(ref owner) = args.owner {
                let output_tmp = OutputConfig::new(output_path);
                let state_path = output_tmp.backup_state_path(owner);
                match BackupState::load(&state_path) {
                    Ok(Some(state)) => {
                        info!(
                            since = %state.last_successful_run,
                            "auto-using last successful run timestamp as --since (incremental backup)"
                        );
                        args.since = Some(state.last_successful_run);
                    }
                    Ok(None) => {
                        info!("no prior backup state found; performing full backup");
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to read backup state file; performing full backup");
                    }
                }
            }
        }
    }

    // Validate --since format early so we fail fast with a clear error.
    if let Some(ref since) = args.since {
        if !is_valid_iso8601(since) {
            error!(
                since = %since,
                "invalid --since value; expected ISO 8601 format, e.g. \"2024-01-01T00:00:00Z\""
            );
            return ExitCode::FAILURE;
        }
    }

    // Validate that an owner was supplied (via CLI or config file).
    if args.owner.is_none() {
        error!("no owner specified; provide OWNER as a positional argument or via 'owner' in the config file");
        return ExitCode::FAILURE;
    }

    // ── Verify-only mode ──────────────────────────────────────────────────
    // When --verify is set we only check the manifest; no API calls needed.
    if args.verify {
        let owner = args.owner.as_deref().unwrap();
        let output_path = args.output.as_ref().cloned().unwrap_or_else(|| ".".into());
        let output = OutputConfig::new(&output_path);
        let json_dir = output.owner_json_dir(owner);
        return run_verify(&json_dir);
    }

    // Obtain GitHub credential — token, device flow, or anonymous.
    let credential = match obtain_credential(&args).await {
        Ok(c) => c,
        Err(e) => {
            error!("authentication failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    if matches!(credential, Credential::Anonymous) {
        warn!(
            "no token provided — running unauthenticated (public data only, 60 req/h rate limit)"
        );
    }

    // Capture values needed after `args` is (partially) consumed.
    let report_path = args.report.clone();
    let mirror_dest = build_mirror_dest(&args);
    let s3_config = build_s3_config(&args);
    let s3_include_assets = args.s3_include_assets;
    // Capture before `args` is consumed by `into_backup_options`.
    let api_url = args.api_url.clone();
    let write_manifest_flag = args.manifest;
    let _verify_only = args.verify;
    let prometheus_metrics_path = args.prometheus_metrics.clone();
    let diff_with = args.diff_with.clone();
    let keep_last = args.keep_last;
    let max_age_days = args.max_age_days;
    let restore_mode = args.restore;
    let restore_target_org = args.restore_target_org.clone();
    let encrypt_key = args.encrypt_key.clone();

    let (owner, output_path, opts) = args.into_backup_options();
    let output = OutputConfig::new(&output_path);
    let cred = credential;

    // Construct the GitHub client (with optional GHE base URL).
    let client = match api_url.as_deref() {
        Some(url) => GitHubClient::with_api_url(cred, url),
        None => GitHubClient::new(cred),
    };
    let client = match client {
        Ok(c) => c,
        Err(e) => {
            error!("failed to initialise GitHub client: {e}");
            return ExitCode::FAILURE;
        }
    };

    // ── Token scope pre-validation ─────────────────────────────────────────
    // Check whether the classic PAT has the required OAuth scopes *before*
    // starting the backup.  Fine-grained PATs omit X-OAuth-Scopes; we skip
    // the check silently for those.
    if client.token().is_some() {
        match client.get_token_scopes().await {
            Ok(scopes) if !scopes.is_empty() => {
                info!(scopes = ?scopes, "token scopes");

                let needs_org = opts.org_members
                    || opts.org_teams
                    || matches!(opts.target, github_backup_types::config::BackupTarget::Org);
                if needs_org && !scopes.iter().any(|s| s == "read:org" || s == "admin:org") {
                    warn!(
                        "token is missing the 'read:org' scope; organisation members \
                         and teams may be inaccessible. Add 'read:org' to avoid \
                         mid-backup failures."
                    );
                }

                if opts.private
                    && !scopes.contains(&"repo".to_string())
                    && !scopes.iter().any(|s| s.starts_with("repo:"))
                {
                    warn!(
                        "token does not have the 'repo' scope; private repository \
                         access will be limited. Add 'repo' to the token for a complete backup."
                    );
                }
            }
            Ok(_) => {
                info!("fine-grained PAT or GitHub App token detected — skipping OAuth scope check");
            }
            Err(e) => {
                // Non-fatal: the real error will surface on the first API call.
                warn!(error = %e, "token scope pre-validation request failed (continuing)");
            }
        }
    }

    // Capture wall-clock start time for the JSON summary report.
    let started_at_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // ── Primary backup ────────────────────────────────────────────────────
    let engine = BackupEngine::new(
        client,
        FsStorage::new(),
        ProcessGitRunner::new(),
        output.clone(),
        opts,
    );

    let stats = match engine.run(&owner).await {
        Ok(s) => {
            info!(
                repos_backed_up = s.repos_backed_up(),
                repos_skipped = s.repos_skipped(),
                repos_errored = s.repos_errored(),
                gists_backed_up = s.gists_backed_up(),
                issues_fetched = s.issues_fetched(),
                prs_fetched = s.prs_fetched(),
                "backup complete"
            );
            s
        }
        Err(e) => {
            error!("backup failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    info!("{stats}");

    // ── Write backup state (last-success timestamp) ────────────────────────
    {
        use std::time::{Duration, UNIX_EPOCH};
        let state = BackupState {
            last_successful_run: report::unix_to_iso8601(
                UNIX_EPOCH + Duration::from_secs(started_at_unix),
            ),
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            repos_backed_up: stats.repos_backed_up(),
        };
        let state_path = output.backup_state_path(&owner);
        if let Err(e) = state.save(&state_path) {
            warn!(error = %e, "failed to write backup state file");
        } else {
            info!(path = %state_path.display(), "wrote backup state");
        }
    }

    // ── Summary report ─────────────────────────────────────────────────────
    if let Some(report_file) = report_path {
        if let Err(e) = write_report(&report_file, &owner, &stats, started_at_unix) {
            error!("failed to write report: {e}");
            return ExitCode::FAILURE;
        }
        info!(path = %report_file.display(), "wrote summary report");
    }

    // ── SHA-256 manifest ───────────────────────────────────────────────────
    if write_manifest_flag {
        use std::time::{Duration, UNIX_EPOCH};
        let created_at = report::unix_to_iso8601(UNIX_EPOCH + Duration::from_secs(started_at_unix));
        let json_dir = output.owner_json_dir(&owner);
        match write_manifest(&json_dir, &created_at) {
            Ok(n) => info!(entries = n, "SHA-256 manifest written"),
            Err(e) => {
                error!("failed to write manifest: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    // ── Prometheus metrics ─────────────────────────────────────────────────
    if let Some(ref metrics_path) = prometheus_metrics_path {
        if let Err(e) = write_prometheus_metrics(metrics_path, &owner, &stats, started_at_unix) {
            error!("failed to write Prometheus metrics: {e}");
            return ExitCode::FAILURE;
        }
        info!(path = %metrics_path.display(), "wrote Prometheus metrics");
    }

    // ── Diff with previous backup ──────────────────────────────────────────
    if let Some(ref prev_dir) = diff_with {
        let json_dir = output.owner_json_dir(&owner);
        match run_diff(prev_dir, &json_dir) {
            Ok(summary) => info!(diff = %summary, "backup diff"),
            Err(e) => warn!(error = %e, "diff failed (non-fatal)"),
        }
    }

    // ── Restore mode ───────────────────────────────────────────────────────
    if restore_mode {
        if let Some(ref target_org) = restore_target_org {
            warn!(
                target_org = %target_org,
                "restore mode is not yet fully implemented; \
                 please create issues and labels manually from the JSON backup files"
            );
        }
    }

    // ── Post-processing: push mirrors ──────────────────────────────────────
    if let Some(dest) = mirror_dest {
        if let Err(e) = run_mirror_push_dest(&dest, &output, &owner).await {
            error!("mirror push failed: {e}");
            return ExitCode::FAILURE;
        }
    }

    // ── Post-processing: S3 sync ───────────────────────────────────────────
    if let Some(s3_cfg) = s3_config {
        if let Err(e) = run_s3_sync(
            &s3_cfg,
            &output,
            &owner,
            s3_include_assets,
            encrypt_key.as_deref(),
        )
        .await
        {
            error!("S3 sync failed: {e}");
            return ExitCode::FAILURE;
        }
    }

    // ── Retention / pruning ────────────────────────────────────────────────
    if keep_last.is_some() || max_age_days.is_some() {
        if let Err(e) = apply_retention(&output_path, keep_last, max_age_days) {
            warn!(error = %e, "retention policy application failed (non-fatal)");
        }
    }

    // Suppress unused-variable warnings for not-yet-fully-implemented features.
    let _ = restore_target_org;

    ExitCode::SUCCESS
}

/// Resolves the GitHub credential from CLI args.
///
/// Returns a [`Credential::Token`] (PAT or OAuth), or
/// [`Credential::Anonymous`] when no auth method is provided.
async fn obtain_credential(args: &Args) -> Result<Credential, String> {
    if let Some(token) = &args.token {
        return Ok(Credential::Token(token.clone()));
    }

    if args.device_auth {
        let client_id = args
            .oauth_client_id
            .as_deref()
            .ok_or_else(|| "--oauth-client-id is required when using --device-auth".to_string())?;

        info!("starting OAuth device flow");
        let scope = args.oauth_scopes.as_str();

        let token = device_flow(client_id, scope, |code, url| {
            eprintln!();
            eprintln!("──────────────────────────────────────────────────────");
            eprintln!("  GitHub OAuth device authorisation");
            eprintln!("──────────────────────────────────────────────────────");
            eprintln!("  1. Open:  {url}");
            eprintln!("  2. Enter: {code}");
            eprintln!("──────────────────────────────────────────────────────");
            eprintln!("  Waiting for authorisation…");
            eprintln!();
        })
        .await
        .map_err(|e| e.to_string())?;

        return Ok(Credential::Token(token));
    }

    Ok(Credential::Anonymous)
}

/// Mirror destination — either a Gitea-compatible host or a GitLab instance.
enum MirrorDest {
    Gitea(GiteaConfig),
    GitLab(GitLabConfig),
}

/// Dispatches the mirror push to the appropriate runner.
async fn run_mirror_push_dest(
    dest: &MirrorDest,
    output: &OutputConfig,
    owner: &str,
) -> Result<(), String> {
    match dest {
        MirrorDest::Gitea(config) => run_mirror_push_gitea(config, output, owner).await,
        MirrorDest::GitLab(config) => run_mirror_push_gitlab(config, output, owner).await,
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
async fn run_s3_sync(
    config: &S3Config,
    output: &OutputConfig,
    owner: &str,
    include_assets: bool,
    _encrypt_key: Option<&str>,
) -> Result<(), String> {
    let client = S3Client::new(config.clone()).map_err(|e| e.to_string())?;
    let backup_root = output.owner_json_dir(owner);

    if !backup_root.exists() {
        warn!(dir = %backup_root.display(), "backup directory does not exist; skipping S3 sync");
        return Ok(());
    }

    let stats = sync_to_s3(&client, config, &backup_root, include_assets)
        .await
        .map_err(|e| e.to_string())?;

    info!(
        uploaded = stats.uploaded,
        skipped = stats.skipped,
        errored = stats.errored,
        "S3 sync complete"
    );

    if stats.errored > 0 {
        warn!(errored = stats.errored, "some files failed to upload to S3");
    }

    Ok(())
}

/// Builds a [`MirrorDest`] from CLI args, or returns `None` if no mirror
/// destination is configured.
fn build_mirror_dest(args: &Args) -> Option<MirrorDest> {
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
fn build_s3_config(args: &Args) -> Option<S3Config> {
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

/// Checks raw args for `--completions <shell>` before clap parses them,
/// returning the requested [`Shell`] if found.
fn detect_completions_request() -> Option<Shell> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--completions" {
            return args.next().and_then(|s| s.parse().ok());
        }
    }
    None
}

/// Runs the verify-only mode: checks the SHA-256 manifest in `json_dir`.
fn run_verify(json_dir: &std::path::Path) -> ExitCode {
    info!(dir = %json_dir.display(), "verifying backup integrity");
    match verify_manifest(json_dir) {
        Err(e) => {
            error!("manifest verification failed: {e}");
            ExitCode::FAILURE
        }
        Ok(report) => {
            if report.is_clean() {
                info!(
                    ok = report.ok,
                    "backup integrity verified — all files match"
                );
                ExitCode::SUCCESS
            } else {
                if !report.tampered.is_empty() {
                    error!(files = ?report.tampered, "TAMPERED: digest mismatch");
                }
                if !report.missing.is_empty() {
                    error!(files = ?report.missing, "MISSING: files in manifest but not on disk");
                }
                if !report.unexpected.is_empty() {
                    warn!(files = ?report.unexpected, "UNEXPECTED: files on disk not in manifest");
                }
                ExitCode::FAILURE
            }
        }
    }
}

/// Writes Prometheus-format metrics to `path`.
fn write_prometheus_metrics(
    path: &std::path::Path,
    owner: &str,
    stats: &github_backup_core::BackupStats,
    started_at_unix: u64,
) -> Result<(), String> {
    let mut out = String::new();
    // Prometheus text format: each metric is HELP + TYPE + value.
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
fn run_diff(prev_dir: &std::path::Path, curr_dir: &std::path::Path) -> Result<String, String> {
    // Compare the repos.json files to find added/removed repositories.
    let prev_repos_path = prev_dir.join("repos.json");
    let curr_repos_path = curr_dir.join("repos.json");

    let prev_repos = read_repo_names(&prev_repos_path)?;
    let curr_repos = read_repo_names(&curr_repos_path)?;

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
fn read_repo_names(path: &std::path::Path) -> Result<Vec<String>, String> {
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
/// directly under `output_root`.
fn apply_retention(
    output_root: &std::path::Path,
    keep_last: Option<usize>,
    max_age_days: Option<u64>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(output_root).map_err(|e| format!("read output dir: {e}"))?;

    // Collect directories whose names start with a date stamp.
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
        let cutoff_secs = now_secs.saturating_sub(max_age * 86400);
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
        std::fs::remove_dir_all(path)
            .map_err(|e| format!("delete snapshot {}: {e}", path.display()))?;
    }

    if !to_delete.is_empty() {
        info!(deleted = to_delete.len(), "retention policy applied");
    }

    Ok(())
}

/// Initialises the `tracing` subscriber.
fn init_tracing(quiet: bool, verbose: u8) {
    use tracing_subscriber::{fmt, EnvFilter};

    let level = if quiet {
        "error"
    } else {
        match verbose {
            0 => "info",
            1 => "debug",
            _ => "trace",
        }
    };

    // Allow RUST_LOG to override the computed level.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
}
