// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! `github-backup` binary entry point.

use std::io;
use std::process::ExitCode;

use clap::CommandFactory;
use clap_complete::generate;
use tracing::{error, info, warn};

use github_backup_tui::InitialConfig;
use github_backup_types::config::{ConfigFile, Credential, OutputConfig};

mod cli;
mod doctor;
mod errors;
mod lock;
mod metrics;
mod modes;
mod notify;
mod post_process;
mod report;
mod restore;
mod run;
mod scopes;
mod setup;
mod shutdown;
mod ui;

use cli::Args;
use post_process::decode_encrypt_key;

#[tokio::main]
async fn main() -> ExitCode {
    // Check for --completions <shell> before full arg parsing so it works
    // even when required args (token, owner) are absent.
    if let Some(shell) = setup::detect_completions_request() {
        generate(
            shell,
            &mut Args::command(),
            "github-backup",
            &mut io::stdout(),
        );
        return ExitCode::SUCCESS;
    }

    // Same trick for --print-config-template: handled before full parsing so
    // operators bootstrapping a fresh install do not have to supply
    // unrelated required flags first.
    if std::env::args().any(|a| a == "--print-config-template") {
        print!("{}", setup::config_template());
        return ExitCode::SUCCESS;
    }

    let (mut args, matches) = Args::parse_cli();

    // ── TUI mode ──────────────────────────────────────────────────────────────
    if args.tui {
        if let Err(e) = args.check_dependencies(&matches) {
            e.exit();
        }
        let initial = InitialConfig {
            token: args.token.clone(),
            owner: args.owner.clone(),
            output: args.output.as_ref().map(|p| p.display().to_string()),
            api_url: args.api_url.clone(),
        };
        return github_backup_tui::run_tui(initial).await;
    }

    // Initialise structured logging early so config-file errors are logged.
    setup::init_tracing(args.quiet, args.verbose);

    // ── Config file ────────────────────────────────────────────────────────
    if let Some(ref config_path) = args.config.clone() {
        match ConfigFile::from_path(config_path) {
            Ok(cfg) => {
                info!(path = %config_path.display(), "loaded config file");
                setup::check_config_permissions(config_path);
                args.merge_config_with(&cfg, &matches);
            }
            Err(e) => {
                error!("{e}");
                return ExitCode::FAILURE;
            }
        }
    }

    // Flag-dependency rules that clap cannot express without also applying
    // them to environment variables (see `Args::check_dependencies`).  Checked
    // after the config merge so a config-file value satisfies them.
    if let Err(e) = args.check_dependencies(&matches) {
        e.exit();
    }

    // ── List recommended OAuth scopes and exit (after config merge so the
    // computed scope set reflects every category the user enabled).
    if args.list_scopes {
        print!("{}", scopes::render_recommendation(&args));
        return ExitCode::SUCCESS;
    }

    // ── --doctor / --check: run diagnostics and exit (before locks or
    // backup state is touched).  Both modes share the same checks; `--check`
    // additionally echoes the resolved configuration.
    if args.doctor || args.check {
        return modes::run_doctor(&args).await;
    }

    // `--since` is an expert override (see `--help`); the engine otherwise uses
    // each repository's own watermark from the previous run.  Normalise it here
    // so a bad value fails fast with a clear message.
    if let Some(since) = args.since.take() {
        match report::normalise_since(&since) {
            Ok(normalised) => args.since = Some(normalised),
            Err(reason) => {
                error!(since = %since, "invalid --since value: {reason}");
                return ExitCode::FAILURE;
            }
        }
    }

    // Decode encryption key early so we fail fast before any network calls.
    let encrypt_key = match decode_encrypt_key(args.encrypt_key.as_deref()) {
        Ok(k) => k,
        Err(e) => {
            error!("{e}");
            return ExitCode::FAILURE;
        }
    };

    // ── Decrypt mode ──────────────────────────────────────────────────────
    // Needs no OWNER: it only reads a file, so it is handled before the owner
    // checks below.
    if args.decrypt {
        let input_path = match args.decrypt_input.as_ref() {
            Some(p) => p,
            None => {
                error!("--decrypt requires --decrypt-input <FILE>");
                return ExitCode::FAILURE;
            }
        };
        let output_path = match args.decrypt_output.as_ref() {
            Some(p) => p,
            None => {
                error!("--decrypt requires --decrypt-output <FILE>");
                return ExitCode::FAILURE;
            }
        };
        let key = match encrypt_key.as_deref() {
            Some(k) => k,
            None => {
                error!("--decrypt requires --encrypt-key or BACKUP_ENCRYPT_KEY");
                return ExitCode::FAILURE;
            }
        };
        return modes::run_decrypt(input_path, output_path, key);
    }

    // Validate that an owner was supplied (via CLI or config file).
    if args.owner.is_none() {
        // If the user invoked us with no useful arguments at all (no owner,
        // no config, no special flag), print a friendly quickstart instead
        // of just a one-line error — most "first contact" runs land here.
        if ui::invoked_without_arguments(&args) {
            ui::print_quickstart();
            return ExitCode::FAILURE;
        }
        error!("no owner specified; provide OWNER as a positional argument or via 'owner' in the config file");
        return ExitCode::FAILURE;
    }

    // Defense in depth: even though `owner` is supposed to be a GitHub user
    // or organisation name, it ends up as a path segment under `--output`.
    // Refuse anything that could escape the output root or break path
    // construction across operating systems.  The real upstream validation
    // is done by GitHub's API itself (a malformed owner just 404s), so this
    // is a safety net for typos and accidental shell-injection.
    if let Some(ref owner) = args.owner {
        if let Err(reason) = setup::validate_owner_name(owner) {
            error!(owner = %owner, "invalid owner name: {reason}");
            return ExitCode::FAILURE;
        }
    }

    // ── Verify-only mode ──────────────────────────────────────────────────
    if args.verify {
        let owner = args.owner.as_deref().unwrap();
        let output_path = args.output.as_ref().cloned().unwrap_or_else(|| ".".into());
        let output = OutputConfig::new(&output_path);
        let json_dir = output.owner_json_dir(owner);
        return modes::run_verify(&json_dir);
    }

    // Warn when --encrypt-key was supplied on the command line (visible in ps aux).
    // If BACKUP_ENCRYPT_KEY is set in the environment, the value came from the env
    // var and is safe; if it is absent the user must have passed --encrypt-key directly.
    if args.encrypt_key.is_some() && std::env::var("BACKUP_ENCRYPT_KEY").is_err() {
        warn!(
            "--encrypt-key was supplied on the command line. The key is visible \
             in the process list (ps aux) to any user on this machine. \
             Use the BACKUP_ENCRYPT_KEY environment variable instead."
        );
    }

    // Obtain GitHub credential — token, device flow, or anonymous.
    let credential = match setup::obtain_credential(&args).await {
        Ok(c) => c,
        Err(e) => {
            let redacted = errors::redact_secrets(&e);
            error!("authentication failed: {redacted}");
            if let Some(hint) = errors::explain_error(&redacted) {
                error!("hint: {hint}");
            }
            return ExitCode::FAILURE;
        }
    };

    if matches!(credential, Credential::Anonymous) {
        // Anonymous mode is supported but the GitHub unauthenticated limit
        // (60 req/h, no private data) is rarely what the operator actually
        // wants.  Be loud about it and tell them exactly how to fix it.
        let asked_for_private = args.private
            || args.org_members
            || args.org_teams
            || args.hooks
            || args.deploy_keys
            || args.collaborators
            || args.action_runs
            || args.actions
            || args.packages
            || args.discussions
            || args.projects;

        if asked_for_private {
            error!(
                "no GitHub credential supplied (--token / GITHUB_TOKEN / --device-auth), \
                 but private or admin-scoped data was requested. \
                 Anonymous requests cannot read this data — aborting before partial backup."
            );
            return ExitCode::FAILURE;
        }

        warn!(
            "no GitHub credential supplied — running unauthenticated. \
             Limited to public data and 60 requests / hour. \
             Set GITHUB_TOKEN, pass --token, or use --device-auth for a full backup."
        );
    }

    if args.restore {
        return run::execute_restore(args, credential).await;
    }
    run::execute(args, credential, encrypt_key).await
}
