// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! `github-backup` binary entry point.

use std::io;
use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use clap_complete::{generate, Shell};
use tracing::{error, info};

use github_backup_client::GitHubClient;
use github_backup_core::{BackupEngine, FsStorage, ProcessGitRunner};
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

    // Build configuration — extract fields before consuming args.
    let owner = args.owner.clone();
    let output = OutputConfig::new(&args.output);
    let token = args.token.clone();
    let opts = args.into_backup_options();
    let cred = Credential::Token(token);

    // Construct the client.
    let client = match GitHubClient::new(cred) {
        Ok(c) => c,
        Err(e) => {
            error!("failed to initialise GitHub client: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Run the backup.
    let engine = BackupEngine::new(
        client,
        FsStorage::new(),
        ProcessGitRunner::new(),
        output,
        opts,
    );

    match engine.run(&owner).await {
        Ok(stats) => {
            info!(
                repos_backed_up = stats.repos_backed_up(),
                repos_skipped = stats.repos_skipped(),
                repos_errored = stats.repos_errored(),
                gists_backed_up = stats.gists_backed_up(),
                "backup complete"
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            error!("backup failed: {e}");
            ExitCode::FAILURE
        }
    }
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
