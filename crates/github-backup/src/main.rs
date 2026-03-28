// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! `github-backup` binary entry point.

use std::io;
use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use clap_complete::{generate, Shell};
use tracing::{error, info, warn};

use github_backup_client::{oauth::device_flow, GitHubClient};
use github_backup_core::{BackupEngine, FsStorage, ProcessGitRunner};
use github_backup_mirror::{config::GiteaConfig, runner::push_mirrors, GiteaClient};
use github_backup_s3::{config::S3Config, sync::sync_to_s3, S3Client};
use github_backup_types::config::{ConfigFile, Credential, OutputConfig};

mod cli;

use cli::Args;

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

    // Validate that an owner was supplied (via CLI or config file).
    if args.owner.is_none() {
        error!("no owner specified; provide OWNER as a positional argument or via 'owner' in the config file");
        return ExitCode::FAILURE;
    }

    // Validate that an auth method was supplied.
    if args.token.is_none() && !args.device_auth {
        error!("no authentication method; use --token / GITHUB_TOKEN, --device-auth, or set 'token' in the config file");
        return ExitCode::FAILURE;
    }

    // Obtain GitHub credential (PAT or OAuth device flow).
    let token = match obtain_token(&args).await {
        Ok(t) => t,
        Err(e) => {
            error!("authentication failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Capture values needed after `args` is (partially) consumed.
    let report_path = args.report.clone();
    let mirror_config = build_mirror_config(&args);
    let s3_config = build_s3_config(&args);
    let s3_include_assets = args.s3_include_assets;

    let (owner, output_path, opts) = args.into_backup_options();
    let output = OutputConfig::new(&output_path);
    let cred = Credential::Token(token);

    // Construct the GitHub client.
    let client = match GitHubClient::new(cred) {
        Ok(c) => c,
        Err(e) => {
            error!("failed to initialise GitHub client: {e}");
            return ExitCode::FAILURE;
        }
    };

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

    // ── Summary report ─────────────────────────────────────────────────────
    if let Some(report_file) = report_path {
        if let Err(e) = write_report(&report_file, &owner, &stats, started_at_unix) {
            error!("failed to write report: {e}");
            return ExitCode::FAILURE;
        }
        info!(path = %report_file.display(), "wrote summary report");
    }

    // ── Post-processing: push mirrors ──────────────────────────────────────
    if let Some(mirror_cfg) = mirror_config {
        if let Err(e) = run_mirror_push(&mirror_cfg, &output, &owner).await {
            error!("mirror push failed: {e}");
            return ExitCode::FAILURE;
        }
    }

    // ── Post-processing: S3 sync ───────────────────────────────────────────
    if let Some(s3_cfg) = s3_config {
        if let Err(e) = run_s3_sync(&s3_cfg, &output, &owner, s3_include_assets).await {
            error!("S3 sync failed: {e}");
            return ExitCode::FAILURE;
        }
    }

    ExitCode::SUCCESS
}

/// Obtains a GitHub access token, either from the CLI arg or via OAuth.
async fn obtain_token(args: &Args) -> Result<String, String> {
    if let Some(token) = &args.token {
        return Ok(token.clone());
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

        return Ok(token);
    }

    Err("no authentication method provided".to_string())
}

/// Writes a JSON summary report to `path`.
///
/// The report includes counters, elapsed time, tool version, and an ISO 8601
/// timestamp so monitoring systems can parse and alert on backup health.
fn write_report(
    path: &std::path::Path,
    owner: &str,
    stats: &github_backup_core::BackupStats,
    started_at_unix: u64,
) -> Result<(), String> {
    use std::time::{Duration, UNIX_EPOCH};

    // Format `started_at_unix` as an ISO 8601 string (best-effort; no chrono dep).
    let started_dt = UNIX_EPOCH + Duration::from_secs(started_at_unix);
    let started_iso = humanise_unix(started_dt);

    let report = serde_json::json!({
        "tool_version": env!("CARGO_PKG_VERSION"),
        "owner": owner,
        "started_at": started_iso,
        "duration_secs": stats.elapsed_secs(),
        "repos_discovered": stats.repos_discovered(),
        "repos_backed_up": stats.repos_backed_up(),
        "repos_skipped": stats.repos_skipped(),
        "repos_errored": stats.repos_errored(),
        "gists_backed_up": stats.gists_backed_up(),
        "success": stats.repos_errored() == 0,
    });
    let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create report directory: {e}"))?;
    }
    std::fs::write(path, json).map_err(|e| format!("cannot write report: {e}"))?;
    Ok(())
}

/// Formats a `SystemTime` as an RFC 3339 / ISO 8601 string without external
/// dependencies.
///
/// Output is always in UTC: `"2026-01-15T12:34:56Z"`.
fn humanise_unix(t: std::time::SystemTime) -> String {
    use std::time::UNIX_EPOCH;

    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Gregorian calendar calculation (no leap-second awareness needed for a
    // timestamp label).
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;

    // Days since epoch → year/month/day (algorithm: https://howardhinnant.github.io/date_algorithms.html)
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };

    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Pushes all locally-cloned repositories as mirrors to the configured
/// Gitea-compatible destination.
async fn run_mirror_push(
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
        "mirror push complete"
    );

    if stats.errored > 0 {
        warn!(
            errored = stats.errored,
            "some repositories failed to push to mirror"
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

/// Builds a [`GiteaConfig`] from CLI args, or returns `None` if no mirror
/// destination is configured.
fn build_mirror_config(args: &Args) -> Option<GiteaConfig> {
    let base_url = args.mirror_to.clone()?;
    let token = args.mirror_token.clone().unwrap_or_default();
    let owner = args
        .mirror_owner
        .clone()
        .unwrap_or_else(|| args.owner.clone().unwrap_or_default());

    Some(GiteaConfig {
        base_url,
        token,
        owner,
        private: args.mirror_private,
    })
}

/// Builds an [`S3Config`] from CLI args, or returns `None` if no S3 bucket
/// is configured.
fn build_s3_config(args: &Args) -> Option<S3Config> {
    let bucket = args.s3_bucket.clone()?;
    let access_key_id = args.s3_access_key.clone().unwrap_or_default();
    let secret_access_key = args.s3_secret_key.clone().unwrap_or_default();

    Some(S3Config {
        bucket,
        region: args.s3_region.clone(),
        prefix: args.s3_prefix.clone(),
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
