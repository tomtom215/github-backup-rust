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
use github_backup_types::config::{Credential, OutputConfig};

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

    let args = Args::parse();

    // Initialise structured logging.
    init_tracing(args.quiet, args.verbose);

    // Obtain GitHub credential (PAT or OAuth device flow).
    let token = match obtain_token(&args).await {
        Ok(t) => t,
        Err(e) => {
            error!("authentication failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Capture values needed after `args` is consumed.
    let owner = args.owner.clone();
    let output_path = args.output.clone();
    let mirror_config = build_mirror_config(&args);
    let s3_config = build_s3_config(&args);
    let s3_include_assets = args.s3_include_assets;
    let output = OutputConfig::new(&output_path);
    let opts = args.into_backup_options();
    let cred = Credential::Token(token);

    // Construct the GitHub client.
    let client = match GitHubClient::new(cred) {
        Ok(c) => c,
        Err(e) => {
            error!("failed to initialise GitHub client: {e}");
            return ExitCode::FAILURE;
        }
    };

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
        .unwrap_or_else(|| args.owner.clone());

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
