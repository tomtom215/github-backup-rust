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
use github_backup_tui::InitialConfig;
use github_backup_types::backup_state::BackupState;
use github_backup_types::config::{ConfigFile, Credential, OutputConfig};

mod cli;
mod post_process;
mod report;
mod restore;

use cli::Args;
use post_process::{
    apply_retention, build_mirror_dest, build_s3_config, decode_encrypt_key, run_diff,
    run_mirror_push_dest, run_s3_sync, write_prometheus_metrics,
};
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
    if args.verify {
        let owner = args.owner.as_deref().unwrap();
        let output_path = args.output.as_ref().cloned().unwrap_or_else(|| ".".into());
        let output = OutputConfig::new(&output_path);
        let json_dir = output.owner_json_dir(owner);
        return run_verify(&json_dir);
    }

    // Decode encryption key early so we fail fast before any network calls.
    let encrypt_key = match decode_encrypt_key(args.encrypt_key.as_deref()) {
        Ok(k) => k,
        Err(e) => {
            error!("{e}");
            return ExitCode::FAILURE;
        }
    };

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
    let api_url = args.api_url.clone();
    let write_manifest_flag = args.manifest;
    let prometheus_metrics_path = args.prometheus_metrics.clone();
    let diff_with = args.diff_with.clone();
    let keep_last = args.keep_last;
    let max_age_days = args.max_age_days;
    let restore_mode = args.restore;
    let restore_target_org = args.restore_target_org.clone();

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
                warn!(error = %e, "token scope pre-validation request failed (continuing)");
            }
        }
    }

    let started_at_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // ── Primary backup ────────────────────────────────────────────────────
    let engine = BackupEngine::new(
        client.clone(),
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

    // ── Write backup state ─────────────────────────────────────────────────
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
        let target_org = restore_target_org.as_deref().unwrap_or(&owner);
        if let Err(e) =
            restore::run_restore(&client, &output, &owner, target_org, api_url.as_deref()).await
        {
            error!("restore failed: {e}");
            return ExitCode::FAILURE;
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

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
}
