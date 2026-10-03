// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! `github-backup` binary entry point.

use std::io;
use std::process::ExitCode;

use clap::CommandFactory;
use clap_complete::generate;
use tracing::{error, info, warn};

use github_backup_client::GitHubClient;
use github_backup_core::{write_manifest, BackupEngine, FsStorage, ProcessGitRunner};
use github_backup_tui::InitialConfig;
use github_backup_types::backup_state::{BackupRunEntry, BackupRunHistory, BackupState};
use github_backup_types::config::{ConfigFile, Credential, OutputConfig};

mod cli;
mod doctor;
mod errors;
mod lock;
mod modes;
mod notify;
mod post_process;
mod report;
mod restore;
mod scopes;
mod setup;
mod shutdown;
mod ui;

use cli::Args;
use post_process::{
    apply_retention, build_mirror_dest, build_s3_config, decode_encrypt_key, run_diff,
    run_mirror_push_dest, run_s3_sync, write_prometheus_metrics,
};
use report::{is_valid_iso8601, unix_secs_to_iso8601, write_report};

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
                args.merge_config(&cfg);
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

    // Decode encryption key early so we fail fast before any network calls.
    let encrypt_key = match decode_encrypt_key(args.encrypt_key.as_deref()) {
        Ok(k) => k,
        Err(e) => {
            error!("{e}");
            return ExitCode::FAILURE;
        }
    };

    // ── Decrypt mode ──────────────────────────────────────────────────────
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

    // Capture values needed after `args` is (partially) consumed.
    let report_path = args.report.clone();
    let mirror_dest = build_mirror_dest(&args);
    let s3_config = build_s3_config(&args);
    let s3_include_assets = args.s3_include_assets;
    let s3_delete_stale = args.s3_delete_stale;
    let api_url = args.api_url.clone();
    let write_manifest_flag = args.manifest;
    let prometheus_metrics_path = args.prometheus_metrics.clone();
    let diff_with = args.diff_with.clone();
    let keep_last = args.keep_last;
    let max_age_days = args.max_age_days;
    let restore_mode = args.restore;
    let restore_target_org = args.restore_target_org.clone();
    let restore_yes = args.restore_yes;
    let dry_run = args.dry_run;
    let notify_webhook = args.notify_webhook.clone();
    let history_size = args.history_size;
    let quiet = args.quiet;

    let (owner, output_path, opts) = args.into_backup_options();
    let output = OutputConfig::new(&output_path);

    // Acquire an exclusive lock on the output directory so two concurrent
    // github-backup processes cannot corrupt each other's checkpoint and state
    // files.  The lock is automatically released when `_output_lock` is dropped
    // at the end of main.
    let _output_lock = match lock::acquire(&output_path) {
        Ok(l) => l,
        Err(e) => {
            error!("{e}");
            return ExitCode::FAILURE;
        }
    };
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

    let started_at_unix = report::unix_now_secs();

    // Print a single-line "what's about to happen" plan, including an ETA
    // computed from the rolling backup history when one exists.  Skipped
    // entirely under --quiet so cron and journal scrapes stay clean.
    if !quiet {
        ui::print_plan(&owner, &output_path, &opts, dry_run, &output);
    }

    // ── Primary backup ────────────────────────────────────────────────────
    let engine = BackupEngine::new(
        client.clone(),
        FsStorage::new(),
        ProcessGitRunner::new(),
        output.clone(),
        opts,
    );

    // Race the backup against a shutdown signal.
    //
    // Handles both Ctrl+C (SIGINT) and SIGTERM (used by `docker stop`,
    // `systemctl stop`, and Kubernetes pod eviction).  On interruption we log
    // a warning, skip post-processing, and exit with the conventional signal
    // exit code so the caller knows the process was terminated rather than
    // completing normally.
    //
    // Any temporary GIT_ASKPASS scripts are cleaned up by their RAII guards
    // when the Tokio runtime shuts down.
    let backup_result = tokio::select! {
        result = engine.run(&owner) => result,
        code = shutdown::wait_for_shutdown_signal() => {
            warn!(
                exit_code = code,
                "backup interrupted by signal — partial data may remain on disk; \
                 re-run to resume"
            );
            shutdown::begin_shutdown(code);
            return ExitCode::from(code);
        }
    };

    let stats = match backup_result {
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
            let raw = errors::redact_secrets(&e.to_string());
            error!("backup failed: {raw}");
            if let Some(hint) = errors::explain_error(&raw) {
                error!("hint: {hint}");
            }
            if let Some(ref url) = notify_webhook {
                notify::send_webhook(url, &owner, "failure", Some(&raw), 0, 0).await;
            }
            return ExitCode::FAILURE;
        }
    };

    info!("{stats}");

    // ── Write backup state ─────────────────────────────────────────────────
    let finished_at_unix = report::unix_now_secs();
    if !quiet {
        ui::print_summary_banner(&stats, finished_at_unix.saturating_sub(started_at_unix));
    }
    {
        let state = BackupState {
            last_successful_run: unix_secs_to_iso8601(started_at_unix),
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

    // ── Append to backup run history ───────────────────────────────────────
    {
        let history_path = output.backup_history_path(&owner);
        let mut history = BackupRunHistory::load(&history_path).unwrap_or_default();
        history.push(
            BackupRunEntry {
                timestamp: unix_secs_to_iso8601(started_at_unix),
                repos_backed_up: stats.repos_backed_up(),
                elapsed_secs: (finished_at_unix.saturating_sub(started_at_unix)) as f64,
                success: true,
                tool_version: env!("CARGO_PKG_VERSION").to_string(),
            },
            history_size,
        );
        if let Err(e) = history.save(&history_path) {
            warn!(error = %e, "failed to write backup history file");
        } else {
            info!(path = %history_path.display(), entries = history.entries.len(), "wrote backup history");
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
        let created_at = unix_secs_to_iso8601(started_at_unix);
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
        if !dry_run && !restore::confirm_restore(target_org, restore_yes) {
            error!("restore aborted — pass --restore-yes to confirm non-interactively");
            return ExitCode::FAILURE;
        }
        if let Err(e) = restore::run_restore(&client, &output, &owner, target_org, dry_run).await {
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
            s3_delete_stale,
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

    // ── Webhook notification ───────────────────────────────────────────────
    if let Some(ref url) = notify_webhook {
        notify::send_webhook(
            url,
            &owner,
            "success",
            None,
            stats.repos_backed_up(),
            stats.repos_errored(),
        )
        .await;
    }

    ExitCode::SUCCESS
}
