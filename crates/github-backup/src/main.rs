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
use github_backup_types::backup_state::{BackupRunEntry, BackupRunHistory, BackupState};
use github_backup_types::config::{ConfigFile, Credential, OutputConfig};

mod cli;
mod doctor;
mod lock;
mod notify;
mod post_process;
mod report;
mod restore;
mod scopes;

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
    if let Some(shell) = detect_completions_request() {
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
        print!("{}", config_template());
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
                check_config_permissions(config_path);
                args.merge_config(&cfg);
            }
            Err(e) => {
                error!("{e}");
                return ExitCode::FAILURE;
            }
        }
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
        return run_doctor(&args).await;
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
        if invoked_without_arguments(&args) {
            print_quickstart();
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
        if let Err(reason) = validate_owner_name(owner) {
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
        return run_decrypt(input_path, output_path, key);
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
    let credential = match obtain_credential(&args).await {
        Ok(c) => c,
        Err(e) => {
            let redacted = redact_secrets(&e);
            error!("authentication failed: {redacted}");
            if let Some(hint) = explain_error(&redacted) {
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

    let started_at_unix = unix_now_secs();

    // Print a single-line "what's about to happen" plan, including an ETA
    // computed from the rolling backup history when one exists.  Skipped
    // entirely under --quiet so cron and journal scrapes stay clean.
    if !quiet {
        print_plan(&owner, &output_path, &opts, dry_run, &output);
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
        code = wait_for_shutdown_signal() => {
            warn!(
                exit_code = code,
                "backup interrupted by signal — partial data may remain on disk; \
                 re-run to resume"
            );
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
            let raw = redact_secrets(&e.to_string());
            error!("backup failed: {raw}");
            if let Some(hint) = explain_error(&raw) {
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
    let finished_at_unix = unix_now_secs();
    if !quiet {
        print_summary_banner(&stats, finished_at_unix.saturating_sub(started_at_unix));
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
        if !dry_run && !confirm_restore(target_org, restore_yes) {
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

/// Decrypts `input_path` with `key` and writes plaintext to `output_path`.
fn run_decrypt(
    input_path: &std::path::Path,
    output_path: &std::path::Path,
    key: &[u8; 32],
) -> ExitCode {
    let ciphertext = match std::fs::read(input_path) {
        Ok(b) => b,
        Err(e) => {
            error!(path = %input_path.display(), "failed to read encrypted file: {e}");
            return ExitCode::FAILURE;
        }
    };
    let plaintext = match github_backup_s3::encrypt::decrypt(key, &ciphertext) {
        Ok(p) => p,
        Err(e) => {
            error!("decryption failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                error!(path = %parent.display(), "failed to create output directory: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    match std::fs::write(output_path, &plaintext) {
        Ok(()) => {
            info!(
                input = %input_path.display(),
                output = %output_path.display(),
                bytes = plaintext.len(),
                "decryption complete"
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            error!(path = %output_path.display(), "failed to write decrypted output: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Checks config file permissions and warns if it is group- or world-readable.
///
/// A config file commonly contains `token`, `s3_access_key`, or
/// `s3_secret_key`.  If the file is readable by other users, those credentials
/// are exposed.
fn check_config_permissions(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if let Ok(meta) = std::fs::metadata(path) {
            let mode = meta.mode();
            if mode & 0o077 != 0 {
                warn!(
                    path = %path.display(),
                    mode = format!("{:o}", mode & 0o777),
                    "config file is readable by group or others; credentials stored \
                     in it may be exposed. Run: chmod 600 {}",
                    path.display()
                );
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path; // permission checks not supported on this platform
    }
}

/// Prints a restore warning banner and, when interactive, asks for explicit
/// confirmation.
///
/// Returns `true` if the user confirmed.  Confirmation can come from any of:
/// - The `--restore-yes` CLI flag.
/// - The `GITHUB_BACKUP_RESTORE_YES=1` environment variable (handy for CI
///   pipelines where adding a flag is awkward).
/// - Typing `yes` on a TTY.
///
/// Returns `false` if the user declined, stdin is not a TTY, or any of the
/// above failed.  The non-TTY error message explicitly tells the user *both*
/// escape hatches so they don't have to dig through `--help`.
fn confirm_restore(target_org: &str, restore_yes: bool) -> bool {
    if restore_yes {
        return true;
    }
    if std::env::var("GITHUB_BACKUP_RESTORE_YES").as_deref() == Ok("1") {
        info!("GITHUB_BACKUP_RESTORE_YES=1 — proceeding with restore");
        return true;
    }

    eprintln!();
    eprintln!("╔══════════════════════════════════════════════════════════════╗");
    eprintln!("║         WARNING: RESTORE WILL MODIFY GITHUB DATA            ║");
    eprintln!("╚══════════════════════════════════════════════════════════════╝");
    eprintln!("  Target : {target_org}");
    eprintln!("  This will CREATE labels, milestones, and issues in the target");
    eprintln!("  organisation.  This action cannot be automatically undone.");
    eprintln!();

    use std::io::IsTerminal as _;
    if !std::io::stdin().is_terminal() {
        eprintln!("  stdin is not a TTY — to confirm non-interactively, either:");
        eprintln!("    • re-run with --restore-yes, or");
        eprintln!("    • export GITHUB_BACKUP_RESTORE_YES=1");
        eprintln!();
        return false;
    }

    eprint!("  Type 'yes' to continue: ");
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    eprintln!();
    input.trim() == "yes"
}

/// Waits for a process shutdown signal and returns the conventional exit code.
///
/// Handles:
/// - `SIGINT` (Ctrl+C) → exit code 130  (128 + 2)
/// - `SIGTERM` (`docker stop`, `systemctl stop`, Kubernetes) → exit code 143  (128 + 15)
///
/// On Windows only `Ctrl+C` is handled (exit code 130); there is no SIGTERM.
async fn wait_for_shutdown_signal() -> u8 {
    // Ctrl+C / SIGINT is cross-platform via tokio.
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
        130u8
    };

    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            code = ctrl_c => code,
            _ = sigterm.recv() => 143u8,
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await
    }
}

/// Initialises the `tracing` subscriber.
///
/// Respects the standard observability conventions:
/// - `RUST_LOG` overrides the level filter when set;
/// - `NO_COLOR` (any value) disables ANSI colour codes;
/// - `CLICOLOR_FORCE=1` forces colour even when stderr is not a TTY.
///
/// When stderr is not a TTY (e.g. a log file, CI, journald) we default to
/// no colour so the file contains plain UTF-8 — anyone who wants colour back
/// can set `CLICOLOR_FORCE=1`.
fn init_tracing(quiet: bool, verbose: u8) {
    use std::io::IsTerminal as _;
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

    let no_color = std::env::var_os("NO_COLOR").is_some();
    let force_color = std::env::var("CLICOLOR_FORCE")
        .map(|v| v == "1")
        .unwrap_or(false);
    let ansi = !no_color && (force_color || std::io::stderr().is_terminal());

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(ansi)
        .with_writer(std::io::stderr)
        .init();
}

/// Returns an annotated TOML configuration template.
///
/// All entries are commented out so an unedited template parses as the
/// default configuration.  Lines marked `# REQUIRED` flag the minimum
/// fields a working config typically needs.
fn config_template() -> &'static str {
    include_str!("config_template.toml")
}

/// Current Unix time in seconds.
///
/// Falls back to `0` when the system clock is somehow before the epoch
/// (effectively impossible on any host we run on, but the saturating
/// fallback avoids a panic in pathological environments).
fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Runs the `--doctor` / `--check` pre-flight diagnostic and exits.
///
/// Returns `ExitCode::SUCCESS` when every blocking check passed (warnings
/// are non-blocking), `ExitCode::FAILURE` otherwise.  The whole report is
/// printed to stdout so users can pipe it into a bug report.
async fn run_doctor(args: &Args) -> ExitCode {
    let mut report = doctor::Report::default();
    report.push(doctor::check_git_binary());
    report.push(doctor::check_output_dir(args.output.as_deref()));
    report.push(doctor::check_credential(args));
    let api_url = args.api_url.as_deref();
    report.push(doctor::check_connectivity(api_url).await);

    let ansi = use_ansi();
    let label = if args.check { "check" } else { "doctor" };
    println!("github-backup {label} v{}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("{}", report.render(ansi));
    println!();

    // `--check` echoes the resolved configuration so the user can confirm
    // the categories really are what they expect.
    if args.check {
        println!("Resolved configuration:");
        println!(
            "  owner            {}",
            args.owner.as_deref().unwrap_or("(unset)")
        );
        println!(
            "  output           {}",
            args.output
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(unset)".to_string())
        );
        println!(
            "  api_url          {}",
            args.api_url
                .as_deref()
                .unwrap_or("https://api.github.com (default)")
        );
        println!("  concurrency      {}", args.concurrency.unwrap_or(4));
        println!(
            "  enabled scopes   {}",
            scopes::recommended_scopes(args).join(" ")
        );
        println!();
    }

    let failures = report.failures();
    let warnings = report.warnings();
    if failures > 0 {
        println!(
            "Summary: {failures} blocking issue{} and {warnings} warning{} — backup will not start.",
            if failures == 1 { "" } else { "s" },
            if warnings == 1 { "" } else { "s" },
        );
        ExitCode::FAILURE
    } else if warnings > 0 {
        println!(
            "Summary: ready, but {warnings} warning{} to consider.",
            if warnings == 1 { "" } else { "s" }
        );
        ExitCode::SUCCESS
    } else {
        println!("Summary: ready — every check passed.");
        ExitCode::SUCCESS
    }
}

/// Returns `true` if the current stdout supports ANSI colour codes.
///
/// Honours the same conventions as [`init_tracing`]: `NO_COLOR` disables,
/// `CLICOLOR_FORCE=1` forces, otherwise we autodetect TTY.
fn use_ansi() -> bool {
    use std::io::IsTerminal as _;
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var("CLICOLOR_FORCE").as_deref() == Ok("1") {
        return true;
    }
    std::io::stdout().is_terminal()
}

/// Prints a one-shot summary of what the run is about to do.
///
/// Aimed at non-technical operators who want to see, in one glance,
/// which owner is being backed up, where the files will land, and
/// roughly how long it will take if there's a previous run to compare
/// against.  Skipped under `--quiet` to keep cron / journald output
/// machine-friendly.
fn print_plan(
    owner: &str,
    output_path: &std::path::Path,
    opts: &github_backup_types::config::BackupOptions,
    dry_run: bool,
    output: &OutputConfig,
) {
    let bold = if use_ansi() { "\x1b[1m" } else { "" };
    let reset = if use_ansi() { "\x1b[0m" } else { "" };
    let dim = if use_ansi() { "\x1b[2m" } else { "" };

    let mode = if dry_run {
        format!("{bold}dry run{reset} (no files will be written)")
    } else {
        format!("{bold}backup{reset}")
    };
    let categories = enabled_category_list(opts);

    eprintln!();
    eprintln!("{bold}━━ github-backup ━━{reset}");
    eprintln!("  Owner       {owner}");
    eprintln!("  Mode        {mode}");
    eprintln!("  Output      {}", output_path.display());
    eprintln!("  Concurrency {}", opts.concurrency);
    eprintln!("  Categories  {categories}");
    if let Some(eta) = estimated_duration_from_history(output, owner) {
        eprintln!(
            "  Last run    {dim}~{}s elapsed → expect a similar duration{reset}",
            eta.as_secs()
        );
    }
    eprintln!();
}

/// Prints a colour-coded summary banner at the end of a run.
///
/// Non-technical users tend to scroll past pages of structured log lines
/// without internalising the numbers; this single block gives them a
/// clear pass/fail signal and a one-line tally they can copy into a
/// ticket or status update.
fn print_summary_banner(stats: &github_backup_core::BackupStats, elapsed_secs: u64) {
    let ansi = use_ansi();
    let bold = if ansi { "\x1b[1m" } else { "" };
    let green = if ansi { "\x1b[32m" } else { "" };
    let yellow = if ansi { "\x1b[33m" } else { "" };
    let red = if ansi { "\x1b[31m" } else { "" };
    let reset = if ansi { "\x1b[0m" } else { "" };

    let errored = stats.repos_errored();
    let backed_up = stats.repos_backed_up();
    let skipped = stats.repos_skipped();
    let (glyph, colour, headline) = if errored > 0 {
        (
            if ansi { "✗" } else { "[fail]" },
            red,
            "backup completed with errors",
        )
    } else if backed_up == 0 && skipped == 0 {
        // Zero repos found often means a wrong target or insufficient
        // scope.  Surface it loudly with an actionable suggestion.
        (
            if ansi { "⚠" } else { "[warn]" },
            yellow,
            "backup completed but no repositories were processed",
        )
    } else {
        (
            if ansi { "✓" } else { "[ ok ]" },
            green,
            "backup completed successfully",
        )
    };

    eprintln!();
    eprintln!("{colour}{glyph}{reset}  {bold}{headline}{reset}");
    eprintln!("   {} repo(s) backed up", backed_up);
    if skipped > 0 {
        eprintln!("   {} repo(s) skipped (already in checkpoint)", skipped);
    }
    if errored > 0 {
        eprintln!("   {colour}{errored} repo(s) errored{reset}");
    }
    if stats.issues_fetched() > 0 {
        eprintln!("   {} issue(s) fetched", stats.issues_fetched());
    }
    if stats.prs_fetched() > 0 {
        eprintln!("   {} pull request(s) fetched", stats.prs_fetched());
    }
    if stats.gists_backed_up() > 0 {
        eprintln!("   {} gist(s) backed up", stats.gists_backed_up());
    }
    eprintln!("   elapsed: {}", format_duration(elapsed_secs));

    if backed_up == 0 && skipped == 0 && errored == 0 {
        eprintln!();
        eprintln!(
            "   {yellow}hint{reset}: zero repositories found.  \
             Check OWNER spelling, confirm the token has access, and \
             enable at least one of --repositories / --all / --gists / …"
        );
    }
    eprintln!();
}

/// Formats a duration in seconds as `Hh Mm Ss`, dropping leading zero
/// units so a 90-second run reads as `1m 30s` rather than `0h 1m 30s`.
fn format_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m}m {s}s")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

/// Returns the elapsed duration of the most recent successful run from
/// the backup history, or `None` when no history file exists yet.
fn estimated_duration_from_history(
    output: &OutputConfig,
    owner: &str,
) -> Option<std::time::Duration> {
    let path = output.backup_history_path(owner);
    let history = BackupRunHistory::load(&path).ok()?;
    let last = history.entries.iter().rev().find(|e| e.success)?;
    Some(std::time::Duration::from_secs_f64(
        last.elapsed_secs.max(0.0),
    ))
}

/// Returns a short human-readable list of which categories are enabled.
///
/// Used in the pre-run plan banner so users can confirm at a glance that
/// the right set was selected.  Truncates after the first eight to keep
/// the line readable.
fn enabled_category_list(opts: &github_backup_types::config::BackupOptions) -> String {
    let mut cats: Vec<&'static str> = Vec::new();
    if opts.repositories {
        cats.push("repos");
    }
    if opts.issues {
        cats.push("issues");
    }
    if opts.pulls {
        cats.push("pulls");
    }
    if opts.releases {
        cats.push("releases");
    }
    if opts.wikis {
        cats.push("wikis");
    }
    if opts.gists {
        cats.push("gists");
    }
    if opts.starred {
        cats.push("starred");
    }
    if opts.clone_starred {
        cats.push("clone-starred");
    }
    if opts.actions {
        cats.push("actions");
    }
    if opts.environments {
        cats.push("environments");
    }
    if opts.discussions {
        cats.push("discussions");
    }
    if opts.projects {
        cats.push("projects");
    }
    if opts.packages {
        cats.push("packages");
    }
    if cats.is_empty() {
        return "(none — nothing to do!)".to_string();
    }
    if cats.len() > 8 {
        let head = cats[..8].join(", ");
        return format!("{head} (+{} more)", cats.len() - 8);
    }
    cats.join(", ")
}

/// Redacts anything that looks like a GitHub token in `s`.
///
/// Last-line-of-defence — the rest of the codebase already takes care to
/// keep tokens out of error and log strings, but a misbehaving proxy
/// (which can echo a request URL) or an unusual GitHub error body could
/// in principle still surface a token in `--verbose` output.  This
/// scrubber recognises every official GitHub token prefix and replaces
/// the body with `<redacted>` while preserving the prefix so the
/// operator can still tell *what kind* of token it was.
fn redact_secrets(s: &str) -> String {
    // Order matters: `github_pat_` must be checked before `gh*_` so the
    // longer prefix wins.
    const PREFIXES: &[&str] = &["github_pat_", "ghp_", "gho_", "ghu_", "ghs_", "ghr_"];
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while !rest.is_empty() {
        let mut hit: Option<(usize, &'static str)> = None;
        for prefix in PREFIXES {
            if let Some(idx) = rest.find(prefix) {
                if hit.map(|(j, _)| idx < j).unwrap_or(true) {
                    hit = Some((idx, prefix));
                }
            }
        }
        match hit {
            Some((idx, prefix)) => {
                out.push_str(&rest[..idx]);
                out.push_str(prefix);
                out.push_str("<redacted>");
                let after = &rest[idx + prefix.len()..];
                // Skip the alphanumeric run that constitutes the token body.
                let body_end = after
                    .char_indices()
                    .find(|(_, c)| !c.is_ascii_alphanumeric() && *c != '_')
                    .map(|(i, _)| i)
                    .unwrap_or(after.len());
                rest = &after[body_end..];
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
    }
    out
}

/// Translates a raw error message string into an actionable hint for the
/// user, or returns `None` when no specific advice applies.
///
/// Recognises every common failure pattern: 401/403, expired token,
/// missing scope, rate-limit exhaustion, git binary missing, network
/// timeout, TLS / proxy issues.  The patterns are matched on the
/// `Display` output of `CoreError` / `ClientError`, which is stable
/// because those errors live in our own crates.
fn explain_error(raw: &str) -> Option<&'static str> {
    let r = raw.to_ascii_lowercase();

    // Rate limit / abuse detection.
    if r.contains("rate limit") || r.contains("ratelimit") {
        return Some(
            "GitHub rate-limited the run.  Wait for the printed reset window, \
             use a token with higher limits, or lower --concurrency.",
        );
    }

    // 401 — token wrong or revoked.
    if r.contains("401")
        || r.contains("bad credentials")
        || r.contains("unauthorized")
        || r.contains("requires authentication")
    {
        return Some(
            "Authentication rejected.  Verify GITHUB_TOKEN is set to a current, \
             unrevoked token at https://github.com/settings/tokens.  Run \
             `github-backup --doctor` to confirm the token is reachable.",
        );
    }

    // 403 — usually a missing scope or org-restriction.
    if r.contains("403")
        || r.contains("forbidden")
        || r.contains("resource not accessible")
        || r.contains("must have admin")
    {
        return Some(
            "GitHub refused access.  The token likely lacks a required scope. \
             Run `github-backup --list-scopes` to see what the current flag \
             set needs, then regenerate the token with those scopes.",
        );
    }

    // 404 — wrong target.
    if r.contains("404") || r.contains("not found") {
        return Some(
            "GitHub returned 404.  Check OWNER spelling and capitalisation, and \
             confirm the token has access to that account / org.",
        );
    }

    // Git binary missing.
    if r.contains("could not start git")
        || r.contains("no such file or directory") && r.contains("git")
    {
        return Some(
            "The `git` binary could not be launched.  Install git \
             (https://git-scm.com/downloads) and ensure it is on the PATH, \
             then re-run.",
        );
    }

    // Network failures.
    if r.contains("connection refused")
        || r.contains("dns error")
        || r.contains("tcp connect")
        || r.contains("network is unreachable")
    {
        return Some(
            "Could not reach GitHub.  Check your network connection, DNS, or \
             set HTTPS_PROXY if you are behind a corporate proxy.",
        );
    }

    // TLS issues.
    if r.contains("tls") || r.contains("certificate") {
        return Some(
            "TLS handshake failed.  Update your system's CA bundle \
             (e.g. install ca-certificates), or set HTTPS_PROXY if traffic \
             must traverse a TLS-intercepting proxy.",
        );
    }

    // Disk space / I/O.
    if r.contains("no space left") || r.contains("disk full") {
        return Some(
            "The output disk is full.  Free space or choose a different \
             --output, then re-run; partial progress will resume from the \
             checkpoint.",
        );
    }

    None
}

/// Heuristic: was `github-backup` invoked with no meaningful arguments?
///
/// Used to swap the bare "no owner specified" error for a friendlier
/// quickstart message.  We treat any auth, config-file, output, category,
/// or special-mode flag as a real invocation; everything else is treated
/// as a first-contact run.
fn invoked_without_arguments(args: &Args) -> bool {
    args.owner.is_none()
        && args.config.is_none()
        && args.output.is_none()
        && args.token.is_none()
        && !args.device_auth
        && !args.tui
        && !args.doctor
        && !args.check
        && !args.list_scopes
        && !args.print_config_template
        && !args.verify
        && !args.decrypt
        && !args.restore
}

/// Prints a friendly quickstart for users who run `github-backup` with no
/// arguments.  Aimed at non-technical first-time operators who would
/// otherwise see an unfamiliar error and abandon the tool.
fn print_quickstart() {
    let bold = if use_ansi() { "\x1b[1m" } else { "" };
    let dim = if use_ansi() { "\x1b[2m" } else { "" };
    let reset = if use_ansi() { "\x1b[0m" } else { "" };

    let q = format!(
        r#"{bold}github-backup{reset} v{ver} — back up everything GitHub knows about an account.

{bold}Quickstart{reset}
  1.  Create a personal access token:
        https://github.com/settings/tokens/new
      For a full backup, tick the {bold}repo{reset} and {bold}read:org{reset} scopes.

  2.  Export the token so it does not appear in your shell history:
        {dim}$ export GITHUB_TOKEN=ghp_yourtokenhere{reset}

  3.  Run a real backup:
        {dim}$ github-backup octocat --output ~/github-backups --all{reset}

  4.  Or launch the interactive TUI for a guided run:
        {dim}$ github-backup octocat --tui{reset}

{bold}Helpful flags{reset}
  --doctor                    run pre-flight checks (git, network, token)
  --check                     validate config without performing a backup
  --list-scopes               print OAuth scopes for current flag set
  --print-config-template     write a fresh annotated TOML config to stdout
  --help                      full reference

Documentation:  https://tomtom215.github.io/github-backup-rust/
"#,
        ver = env!("CARGO_PKG_VERSION"),
    );
    eprintln!("{q}");
}

/// Validates a GitHub owner / organisation name as a safe path segment.
///
/// Rejects anything that could escape the output directory or break path
/// construction across operating systems.  Mirrors GitHub's own rules
/// (alphanumerics and hyphens, no leading/trailing hyphens, 1–39 chars)
/// but is intentionally a little more permissive on length so that future
/// GitHub policy changes do not break this client.
///
/// The function never tries to be the authoritative "is this a real GitHub
/// account" check — that responsibility belongs to the API server.  Its
/// only job is to refuse traversal payloads (`..`, `/`, `\`, NUL) and
/// other typos that would manifest as confusing later errors.
fn validate_owner_name(owner: &str) -> Result<(), &'static str> {
    if owner.is_empty() {
        return Err("name is empty");
    }
    if owner.len() > 100 {
        return Err("name is longer than 100 characters");
    }
    if owner == "." || owner == ".." {
        return Err("name must not be '.' or '..'");
    }
    for c in owner.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => {}
            '/' | '\\' => return Err("name must not contain path separators"),
            '\0' => return Err("name must not contain a NUL byte"),
            _ if c.is_control() => return Err("name must not contain control characters"),
            _ => return Err("name contains an unsupported character"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use github_backup_types::config::ConfigFile;

    #[test]
    fn config_template_is_valid_toml() {
        let toml = config_template();
        ConfigFile::from_toml_str(toml).expect("embedded template must parse as ConfigFile");
    }

    #[test]
    fn config_template_unedited_parses_to_defaults() {
        // An untouched template (all keys commented out) should yield the
        // default `ConfigFile` — i.e. every Option<…> is `None`.  This guards
        // against an accidental uncommented line shipping with the binary.
        let cfg = ConfigFile::from_toml_str(config_template()).expect("parse");
        assert!(cfg.owner.is_none(), "untouched template must not set owner");
        assert!(cfg.token.is_none(), "untouched template must not set token");
        assert!(
            cfg.output.is_none(),
            "untouched template must not set output"
        );
        assert!(cfg.all.is_none(), "untouched template must not set all");
    }

    #[test]
    fn config_template_documents_required_fields() {
        let toml = config_template();
        // Defensive: any key marked REQUIRED in the README + docs must
        // still be present in the template, otherwise onboarding silently
        // regresses.
        assert!(
            toml.contains("REQUIRED — the GitHub user"),
            "template must flag owner as REQUIRED"
        );
        assert!(
            toml.contains("REQUIRED — root directory"),
            "template must flag output as REQUIRED"
        );
    }

    // ── validate_owner_name ───────────────────────────────────────────

    #[test]
    fn validate_owner_accepts_realistic_github_names() {
        for ok in [
            "octocat",
            "GitHub",
            "tom-tom215",
            "a",
            "rust-lang",
            "github-actions",
            "user_with_underscore",
            "ORG-Name42",
        ] {
            assert!(
                validate_owner_name(ok).is_ok(),
                "{ok:?} should pass validation"
            );
        }
    }

    #[test]
    fn validate_owner_rejects_path_traversal_attempts() {
        for bad in ["..", ".", "../etc", "foo/bar", "foo\\bar", "/abs", "a/b"] {
            assert!(
                validate_owner_name(bad).is_err(),
                "{bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn validate_owner_rejects_control_and_special_characters() {
        for bad in ["foo\0bar", "foo\nbar", "foo\tbar", "foo bar", "foo$bar"] {
            assert!(
                validate_owner_name(bad).is_err(),
                "{bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn validate_owner_rejects_empty_or_huge() {
        assert!(validate_owner_name("").is_err());
        let huge: String = "a".repeat(101);
        assert!(validate_owner_name(&huge).is_err());
    }

    // ── redact_secrets ────────────────────────────────────────────────

    #[test]
    fn redact_secrets_replaces_classic_pat() {
        let s = "401 Unauthorized: ghp_abcdef1234567890";
        let out = redact_secrets(s);
        assert!(!out.contains("ghp_abcdef1234567890"));
        assert!(out.contains("ghp_<redacted>"));
    }

    #[test]
    fn redact_secrets_replaces_fine_grained_pat() {
        let s = "url=https://x@github.com?token=github_pat_X9Y8Z7Q";
        let out = redact_secrets(s);
        assert!(!out.contains("github_pat_X9Y8Z7Q"));
        assert!(out.contains("github_pat_<redacted>"));
    }

    #[test]
    fn redact_secrets_replaces_every_known_prefix() {
        for prefix in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
            let raw = format!("token={prefix}xyz123");
            let out = redact_secrets(&raw);
            assert!(
                out.contains(&format!("{prefix}<redacted>")),
                "prefix {prefix:?} not redacted: {out}"
            );
            assert!(!out.contains("xyz123"), "literal body leaked: {out}");
        }
    }

    #[test]
    fn redact_secrets_preserves_text_around_token() {
        let s = "Before: ghp_LEAKED After";
        let out = redact_secrets(s);
        assert_eq!(out, "Before: ghp_<redacted> After");
    }

    #[test]
    fn redact_secrets_handles_text_without_secrets() {
        assert_eq!(redact_secrets("just a regular log"), "just a regular log");
    }

    #[test]
    fn redact_secrets_handles_multiple_tokens() {
        let s = "first ghp_AAA second github_pat_BBB done";
        let out = redact_secrets(s);
        assert!(!out.contains("ghp_AAA"));
        assert!(!out.contains("github_pat_BBB"));
        assert!(out.contains("ghp_<redacted>"));
        assert!(out.contains("github_pat_<redacted>"));
    }

    // ── explain_error ────────────────────────────────────────────────

    #[test]
    fn explain_error_recognises_rate_limit() {
        assert!(explain_error("GitHub rate limit exceeded").is_some());
        assert!(explain_error("ratelimit hit").is_some());
    }

    #[test]
    fn explain_error_recognises_401_403_404() {
        assert!(explain_error("status 401 Unauthorized").is_some());
        assert!(explain_error("status 403 Forbidden").is_some());
        assert!(explain_error("status 404 Not Found").is_some());
        assert!(explain_error("Bad credentials").is_some());
        assert!(explain_error("Resource not accessible by integration").is_some());
    }

    #[test]
    fn explain_error_recognises_git_missing() {
        assert!(explain_error("could not start git: ENOENT").is_some());
    }

    #[test]
    fn explain_error_returns_none_for_unknown() {
        assert!(explain_error("an unrelated message").is_none());
    }

    // ── invoked_without_arguments ─────────────────────────────────────

    #[test]
    fn invoked_without_arguments_true_for_bare_run() {
        use clap::Parser;
        let a = Args::parse_from(["github-backup"]);
        assert!(invoked_without_arguments(&a));
    }

    #[test]
    fn invoked_without_arguments_false_for_owner() {
        use clap::Parser;
        let a = Args::parse_from(["github-backup", "octocat"]);
        assert!(!invoked_without_arguments(&a));
    }

    // ── format_duration ──────────────────────────────────────────────

    #[test]
    fn format_duration_seconds_only() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(45), "45s");
    }

    #[test]
    fn format_duration_minutes_and_seconds() {
        assert_eq!(format_duration(60), "1m 0s");
        assert_eq!(format_duration(125), "2m 5s");
    }

    #[test]
    fn format_duration_includes_hours_when_long() {
        assert_eq!(format_duration(3661), "1h 1m 1s");
        assert_eq!(format_duration(7200), "2h 0m 0s");
    }

    // ── enabled_category_list ────────────────────────────────────────

    #[test]
    fn enabled_category_list_handles_empty_set() {
        use github_backup_types::config::BackupOptions;
        let opts = BackupOptions::default();
        let out = enabled_category_list(&opts);
        assert!(out.contains("none"), "got {out:?}");
    }

    #[test]
    fn enabled_category_list_lists_enabled_categories() {
        use github_backup_types::config::BackupOptions;
        let opts = BackupOptions {
            repositories: true,
            issues: true,
            ..Default::default()
        };
        let out = enabled_category_list(&opts);
        assert!(out.contains("repos"));
        assert!(out.contains("issues"));
    }

    #[test]
    fn enabled_category_list_truncates_long_sets() {
        use github_backup_types::config::BackupOptions;
        let opts = BackupOptions {
            repositories: true,
            issues: true,
            pulls: true,
            releases: true,
            wikis: true,
            gists: true,
            starred: true,
            clone_starred: true,
            actions: true,
            environments: true,
            discussions: true,
            ..Default::default()
        };
        let out = enabled_category_list(&opts);
        assert!(out.contains("+"), "expected truncation marker in: {out}");
    }
}
