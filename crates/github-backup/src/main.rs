// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! `github-backup` binary entry point.

use std::process::ExitCode;

use clap::{ArgMatches, CommandFactory};
use tracing::{error, info, warn};

use github_backup_tui::InitialConfig;
use github_backup_types::config::{ConfigFile, Credential, OutputConfig};

mod account;
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
mod schedule;
mod scopes;
mod setup;
mod shutdown;
mod ui;

use account::{AccountKind, Api};
use cli::Args;
use post_process::decode_encrypt_key;
use run::EXIT_FAILURE;

#[tokio::main]
async fn main() -> ExitCode {
    run().await
}

async fn run() -> ExitCode {
    let (mut args, matches) = Args::parse_cli();

    if let Some(shell) = args.completions {
        let mut script = Vec::new();
        clap_complete::generate(shell, &mut Args::command(), "github-backup", &mut script);
        ui::out(&String::from_utf8_lossy(&script));
        return ExitCode::SUCCESS;
    }
    if args.print_config_template {
        ui::out(setup::config_template());
        return ExitCode::SUCCESS;
    }

    // The TUI installs its own log sink; everything else logs to stderr.
    if !args.tui {
        setup::init_tracing(args.quiet, args.verbose);
    }

    if let Err(code) = load_config(&mut args, &matches) {
        return ExitCode::from(code);
    }
    // Flag-dependency rules that clap cannot express without also applying
    // them to environment variables or config-file values (see
    // `Args::check_dependencies`).  Checked after the config merge so a
    // config-file value satisfies them.
    if let Err(e) = args.check_dependencies(&matches) {
        e.exit();
    }

    if args.tui {
        return run_tui(args, &matches).await;
    }

    for flag in Args::secrets_on_command_line(&matches) {
        warn!(
            "{flag} was given on the command line, where every user of this machine can \
             read it (ps, /proc); use its environment variable or the config file instead"
        );
    }
    if args.keep_last.is_some() || args.max_age_days.is_some() {
        warn!(
            "--keep-last and --max-age-days are deprecated and ignored: github-backup keeps \
             one continuously updated backup per owner, and deleting directories by name \
             pattern was unsafe.  Rotate snapshots with your backup tool (restic, borg, ZFS)."
        );
    }

    // ── List recommended OAuth scopes and exit (after config merge so the
    // computed scope set reflects every category the user enabled).
    if args.list_scopes {
        ui::out(&scopes::render_recommendation(&args));
        return ExitCode::SUCCESS;
    }

    // ── Validation: everything a run would reject, before any mode runs, so
    // that `--doctor` / `--check` cannot call a configuration ready that the
    // backup refuses.
    if let Some(since) = args.since.take() {
        match report::normalise_since(&since) {
            Ok(normalised) => args.since = Some(normalised),
            Err(reason) => {
                error!(since = %since, "invalid --since value: {reason}");
                return ExitCode::FAILURE;
            }
        }
    }
    let encrypt_key = match decode_encrypt_key(args.encrypt_key.as_deref()) {
        Ok(k) => k,
        Err(e) => {
            error!("{e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(owner) = &args.owner {
        if let Err(reason) = setup::validate_owner_name(owner) {
            error!(owner = %owner, "invalid owner name: {reason}");
            return ExitCode::FAILURE;
        }
    }

    // ── Decrypt mode: needs no OWNER and no network ─────────────────────────
    if args.decrypt {
        let (Some(input), Some(output)) = (&args.decrypt_input, &args.decrypt_output) else {
            error!("--decrypt requires --decrypt-input <FILE> and --decrypt-output <FILE>");
            return ExitCode::FAILURE;
        };
        let Some(key) = encrypt_key.as_deref() else {
            error!("--decrypt requires --encrypt-key or BACKUP_ENCRYPT_KEY");
            return ExitCode::FAILURE;
        };
        return modes::run_decrypt(input, output, key);
    }

    // ── Verify-only mode: local, needs OWNER ────────────────────────────────
    if args.verify {
        let Some(owner) = args.owner.as_deref() else {
            error!("--verify needs OWNER: the account whose backup to verify");
            return ExitCode::FAILURE;
        };
        let output_path = args.output.clone().unwrap_or_else(|| ".".into());
        let json_dir = OutputConfig::new(&output_path).owner_json_dir(owner);
        return modes::run_verify(&json_dir);
    }

    discover_gh_token(&mut args).await;

    if args.doctor || args.check {
        return modes::run_doctor(&args).await;
    }

    // A mirror or S3 upload that cannot work should fail now, not after the
    // whole backup.
    if !args.restore {
        if let Err(e) = setup::check_destinations(&args) {
            error!("{e}");
            return ExitCode::FAILURE;
        }
    }

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

    if let Err(code) = resolve_owner(&mut args, &credential).await {
        return ExitCode::from(code);
    }

    if matches!(credential, Credential::Anonymous) {
        // Anonymous mode is supported but the GitHub unauthenticated limit
        // (60 req/h, no private data) is rarely what the operator actually
        // wants.  Be loud about it and tell them exactly how to fix it.
        if let Some(why) = needs_credential(&args) {
            error!(
                "no GitHub credential found (--token, GITHUB_TOKEN, GH_TOKEN, the config \
                 file, `gh auth login`, or --device-auth), but {why}, which anonymous \
                 requests cannot read — aborting before a partial backup.  Log in with \
                 `gh auth login` or set GITHUB_TOKEN; or select public categories only \
                 (e.g. --repositories --issues)."
            );
            return ExitCode::FAILURE;
        }
        warn!(
            "no GitHub credential supplied — running unauthenticated. \
             Limited to public data and 60 requests / hour. \
             Set GITHUB_TOKEN, run `gh auth login`, or use --device-auth for a full backup."
        );
    }

    let mut shutdown = shutdown::Shutdown::install();
    if args.restore {
        return ExitCode::from(run::execute_restore(args, credential).await);
    }
    let key = encrypt_key.as_deref();
    ExitCode::from(match args.every {
        Some(interval) => {
            schedule::run_every(interval, &mut shutdown, async |stop| {
                run::execute(args.clone(), credential.clone(), key, stop).await
            })
            .await
        }
        None => run::execute(args, credential, key, &mut shutdown).await,
    })
}

/// Loads `--config`, or the default config file when it exists, and merges it.
fn load_config(args: &mut Args, matches: &ArgMatches) -> Result<(), u8> {
    let path = match args.config.clone() {
        Some(path) => path,
        None => match setup::default_config_path(|k| std::env::var(k).ok()) {
            Some(path) if path.is_file() => path,
            _ => return Ok(()),
        },
    };
    match ConfigFile::from_path(&path) {
        Ok(cfg) => {
            info!(path = %path.display(), "loaded config file");
            setup::check_config_permissions(&path);
            args.merge_config_with(&cfg, matches);
            Ok(())
        }
        Err(e) => {
            // Before the TUI starts no log sink exists yet.
            if args.tui {
                ui::err(&format!("error: {e}\n"));
            } else {
                error!("{e}");
            }
            Err(EXIT_FAILURE)
        }
    }
}

/// Falls back to the GitHub CLI's token when no other was given.
async fn discover_gh_token(args: &mut Args) {
    if args.token.is_some() || args.device_auth {
        return;
    }
    let host = setup::gh_hostname(args.api_url.as_deref());
    if let Some(token) = setup::token_from_gh_cli("gh".as_ref(), &host).await {
        info!(host = %host, "using the GitHub CLI's login (gh auth token)");
        args.token = Some(token);
    }
}

/// Fills in OWNER when it was not given (the token's account) and settles
/// whether it is an organisation, so that `github-backup my-org` backs up the
/// organisation's private repositories without `--org`.
async fn resolve_owner(args: &mut Args, credential: &Credential) -> Result<(), u8> {
    let api = || {
        Api::new(credential, args.api_url.as_deref()).map_err(|e| {
            error!("{e}");
            EXIT_FAILURE
        })
    };
    let owner = match args.owner.clone() {
        Some(owner) => owner,
        None if matches!(credential, Credential::Anonymous) => {
            if ui::invoked_without_arguments(args) {
                ui::print_quickstart();
            } else {
                error!(
                    "no OWNER given and no GitHub token to find your account by; pass OWNER \
                     or log in (`gh auth login`, GITHUB_TOKEN)"
                );
            }
            return Err(EXIT_FAILURE);
        }
        None => {
            if args.org {
                error!("--org needs OWNER: the name of the organisation to back up");
                return Err(EXIT_FAILURE);
            }
            match api()?.authenticated_login().await {
                Ok(login) => {
                    info!(owner = %login, "no OWNER given: backing up {login}, the account the token belongs to");
                    if let Err(reason) = setup::validate_owner_name(&login) {
                        error!(owner = %login, "invalid owner name: {reason}");
                        return Err(EXIT_FAILURE);
                    }
                    args.owner = Some(login);
                    return Ok(());
                }
                Err(e) => {
                    error!("no OWNER given, and the token's account could not be looked up ({e}); pass OWNER");
                    return Err(EXIT_FAILURE);
                }
            }
        }
    };
    if args.restore {
        // Restore reads the local backup only; OWNER may no longer exist.
        return Ok(());
    }
    match api()?.account_kind(&owner).await {
        Ok(Some(AccountKind::Organization)) if !args.org => {
            info!(owner = %owner, "{owner} is an organisation: backing it up as one (--org)");
            args.org = true;
        }
        Ok(Some(AccountKind::User)) if args.org => {
            warn!(owner = %owner, "{owner} is a user, not an organisation: ignoring --org");
            args.org = false;
        }
        Ok(Some(_)) => {}
        Ok(None) => {
            error!(owner = %owner, "there is no GitHub account named '{owner}'; check the spelling");
            return Err(EXIT_FAILURE);
        }
        Err(e) => {
            let e = errors::redact_secrets(&e);
            warn!("could not look up '{owner}' ({e}); carrying on with the settings as given");
            if let Some(hint) = errors::explain_error(&e) {
                warn!("hint: {hint}");
            }
        }
    }
    Ok(())
}

/// Why the selected backup needs a credential, if it does: the categories
/// that anonymous requests cannot read, after `--all`, the default and
/// `--skip` are applied.
fn needs_credential(args: &Args) -> Option<String> {
    let mut resolved = args.clone();
    resolved.owner.get_or_insert_with(String::new);
    let (_, _, o) = resolved.into_backup_options();
    let private: Vec<&str> = [
        (o.private, "private"),
        (o.org_members, "org-members"),
        (o.org_teams, "org-teams"),
        (o.hooks, "hooks"),
        (o.deploy_keys, "deploy-keys"),
        (o.collaborators, "collaborators"),
        (o.actions, "actions"),
        (o.action_runs, "action-runs"),
        (o.packages, "packages"),
        (o.discussions, "discussions"),
        (o.projects, "projects"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect();
    (!private.is_empty()).then(|| format!("the backup includes {}", private.join(", ")))
}

/// Starts the TUI with the merged command line and config file.
async fn run_tui(mut args: Args, matches: &ArgMatches) -> ExitCode {
    // What the TUI cannot do must not be silently dropped.
    let unsupported: Vec<&str> = [
        (args.mirror_to.is_some(), "--mirror-to / mirror_to"),
        (args.s3_bucket.is_some(), "--s3-bucket / s3_bucket"),
        (args.report.is_some(), "--report / report"),
        (args.prometheus_metrics.is_some(), "--prometheus-metrics"),
        (
            matches.value_source("notify_webhook") == Some(clap::parser::ValueSource::CommandLine),
            "--notify-webhook",
        ),
        (args.diff_with.is_some(), "--diff-with"),
        (args.clone_host.is_some(), "--clone-host / clone_host"),
        (args.device_auth, "--device-auth"),
        (args.verify_git, "--verify-git / verify_git"),
        (args.git_timeout.is_some(), "--git-timeout / git_timeout"),
    ]
    .into_iter()
    .filter_map(|(set, flag)| set.then_some(flag))
    .collect();
    if !unsupported.is_empty() {
        Args::command()
            .error(
                clap::error::ErrorKind::ArgumentConflict,
                format!(
                    "--tui cannot do {}; run without --tui for these",
                    unsupported.join(", ")
                ),
            )
            .exit();
    }

    discover_gh_token(&mut args).await;
    let token = args.token.clone();
    let owner = args.owner.clone();
    let output = args.output.as_ref().map(|p| p.display().to_string());
    let api_url = args.api_url.clone();
    let manifest = args.manifest;
    // `into_backup_options` needs an owner; the TUI asks for it.
    args.owner.get_or_insert_with(String::new);
    let (_, _, options) = args.into_backup_options();
    let initial = InitialConfig {
        token,
        owner,
        output,
        api_url,
        options: Some(options),
        manifest,
    };
    github_backup_tui::run_tui(initial).await
}
