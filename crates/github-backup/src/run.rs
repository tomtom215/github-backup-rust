// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! The backup run itself: engine, post-processing, honest reporting and the
//! process exit status.
//!
//! # Exit status
//!
//! | code | meaning |
//! |------|---------|
//! | `0`  | everything that was asked for succeeded |
//! | `1`  | the run could not be carried out (bad configuration, rejected credentials, the repository list could not be fetched, a fatal I/O error) |
//! | `2`  | usage error (reported by the argument parser) |
//! | `3`  | the run finished but **some items could not be backed up** — the backup is incomplete; see the summary, `--report` and the log |
//! | `130` / `143` | interrupted by SIGINT / SIGTERM |
//!
//! Anything that failed — a repository that could not be cloned, an issue list
//! that could not be fetched, an S3 upload that errored, a mirror push that was
//! refused — is recorded as a failure and yields `3`.  The summary, report,
//! metrics, run history and webhook all read from the same failure list, so
//! they cannot disagree with the exit status.

use std::path::PathBuf;
use std::process::ExitCode;

use tracing::{error, info, warn};
use zeroize::Zeroizing;

use github_backup_client::GitHubClient;
use github_backup_core::ProcessGitRunner;
use github_backup_core::{write_manifest, BackupEngine, BackupStats, CoreError, FsStorage};
use github_backup_s3::config::S3Config;
use github_backup_types::backup_state::{BackupRunEntry, BackupRunHistory};
use github_backup_types::config::{BackupTarget, Credential, OutputConfig};

use crate::cli::Args;
use crate::metrics::write_prometheus_metrics;
use crate::notify::{self, Notification, Status};
use crate::post_process::{
    build_mirror_dest, build_s3_config, run_diff, run_mirror_push_dest, run_s3_sync_with,
    MirrorDest, S3RunOptions,
};
use crate::report::{self, unix_secs_to_iso8601, write_report};
use crate::{errors, lock, restore, shutdown, ui};

/// Every item was backed up.
pub(crate) const EXIT_OK: u8 = 0;
/// The run could not be carried out.
pub(crate) const EXIT_FAILURE: u8 = 1;
/// The run finished, but some items could not be backed up.
pub(crate) const EXIT_INCOMPLETE: u8 = 3;

/// The failure scope used for everything that happens after the engine.
const POST_SCOPE: &str = "post-processing";

/// What to do after the engine has finished, captured from the arguments.
struct Post {
    report: Option<PathBuf>,
    mirror: Option<MirrorDest>,
    s3: Option<S3Config>,
    s3_include_assets: bool,
    s3_delete_stale: bool,
    manifest: bool,
    metrics: Option<PathBuf>,
    diff_with: Option<PathBuf>,
    restore: bool,
    restore_target_org: Option<String>,
    restore_yes: bool,
    webhook: Option<String>,
    history_size: usize,
    quiet: bool,
    dry_run: bool,
    keep_last_or_age: bool,
}

impl Post {
    fn from_args(args: &Args) -> Self {
        Self {
            report: args.report.clone(),
            mirror: build_mirror_dest(args),
            s3: build_s3_config(args),
            s3_include_assets: args.s3_include_assets,
            s3_delete_stale: args.s3_delete_stale,
            manifest: args.manifest,
            metrics: args.prometheus_metrics.clone(),
            diff_with: args.diff_with.clone(),
            restore: args.restore,
            restore_target_org: args.restore_target_org.clone(),
            restore_yes: args.restore_yes,
            webhook: args.notify_webhook.clone(),
            history_size: args.history_size,
            quiet: args.quiet,
            dry_run: args.dry_run,
            keep_last_or_age: args.keep_last.is_some() || args.max_age_days.is_some(),
        }
    }
}

/// Warns about the check of token scopes that would make the run incomplete.
async fn check_token_scopes(
    client: &GitHubClient,
    opts: &github_backup_types::config::BackupOptions,
) {
    if client.token().is_none() {
        return;
    }
    match client.get_token_scopes().await {
        Ok(scopes) if !scopes.is_empty() => {
            info!(scopes = ?scopes, "token scopes");

            let needs_org =
                opts.org_members || opts.org_teams || matches!(opts.target, BackupTarget::Org);
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

/// Runs the backup described by `args` and everything that follows it.
pub(crate) async fn execute(
    args: Args,
    credential: Credential,
    encrypt_key: Option<Zeroizing<[u8; 32]>>,
) -> ExitCode {
    let post = Post::from_args(&args);
    let api_url = args.api_url.clone();
    let (owner, output_path, opts) = args.into_backup_options();
    let output = OutputConfig::new(&output_path);

    // Two concurrent processes would corrupt each other's checkpoint and state.
    // A dry run writes nothing, so it needs no lock (and must not create one).
    let _output_lock = if post.dry_run {
        None
    } else {
        match lock::acquire(&output_path) {
            Ok(l) => Some(l),
            Err(e) => {
                error!("{e}");
                return ExitCode::from(EXIT_FAILURE);
            }
        }
    };

    let client = match api_url.as_deref() {
        Some(url) => GitHubClient::with_api_url(credential, url),
        None => GitHubClient::new(credential),
    };
    let client = match client {
        Ok(c) => c,
        Err(e) => {
            error!("failed to initialise GitHub client: {e}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };

    check_token_scopes(&client, &opts).await;

    let started_at_unix = report::unix_now_secs();

    // A one-line "what is about to happen" plan, skipped under --quiet so cron
    // and journal scrapes stay clean.
    if !post.quiet {
        ui::print_plan(&owner, &output_path, &opts, post.dry_run, &output);
    }

    let engine = BackupEngine::new(
        client.clone(),
        FsStorage::new(),
        ProcessGitRunner::new(),
        output.clone(),
        opts,
    );
    let cancel = engine.cancel_handle();

    // Race the backup against a shutdown signal (Ctrl+C, and SIGTERM from
    // `docker stop` / `systemctl stop` / Kubernetes).  On a signal the engine
    // is cancelled — running git processes are killed, no new work starts — and
    // given a moment to wind down, so locks are released and files are never
    // left half-written; a watchdog forces the exit if it does not.
    let mut engine_run = Box::pin(engine.run(&owner));
    let backup_result = tokio::select! {
        result = &mut engine_run => result,
        code = shutdown::wait_for_shutdown_signal() => {
            warn!(
                exit_code = code,
                "backup interrupted by signal — partial data may remain on disk; \
                 re-run to resume"
            );
            shutdown::begin_shutdown(&cancel, code);
            // Steps abandon in-flight requests and git is killed, so this is
            // quick; the bound keeps a stuck task from outlasting the watchdog.
            let _ = tokio::time::timeout(std::time::Duration::from_secs(4), engine_run).await;
            return ExitCode::from(code);
        }
    };

    let stats = match backup_result {
        Ok(stats) => stats,
        Err(e) => return fatal(&post, &owner, &e).await,
    };

    finish(
        &post,
        &client,
        &output,
        &owner,
        &stats,
        started_at_unix,
        encrypt_key.as_deref(),
    )
    .await
}

/// Reports a run that could not be completed.
async fn fatal(post: &Post, owner: &str, e: &CoreError) -> ExitCode {
    let raw = errors::redact_secrets(&e.to_string());
    error!("backup failed: {raw}");
    if let Some(hint) = errors::explain_error(&raw) {
        error!("hint: {hint}");
    }
    if let Some(url) = &post.webhook {
        if !post.dry_run {
            notify::send_webhook(
                url,
                &Notification {
                    status: Status::Failure,
                    owner,
                    error: Some(&raw),
                    repos_backed_up: 0,
                    repos_errored: 0,
                    failures: &[],
                },
            )
            .await;
        }
    }
    ExitCode::from(EXIT_FAILURE)
}

/// Everything after a completed engine run: post-processing, then the reports
/// that must include its outcome, then the exit status.
async fn finish(
    post: &Post,
    client: &GitHubClient,
    output: &OutputConfig,
    owner: &str,
    stats: &BackupStats,
    started_at_unix: u64,
    encrypt_key: Option<&[u8; 32]>,
) -> ExitCode {
    info!("{stats}");

    if post.dry_run {
        // A dry run changes nothing: no manifest, mirror push, S3 sync, report,
        // metrics, history or webhook.
        if !post.quiet {
            ui::print_summary_banner(
                stats,
                report::unix_now_secs().saturating_sub(started_at_unix),
                true,
            );
        }
        info!("dry run: skipped state, report, metrics, mirror push, S3 sync and notification");
        return ExitCode::from(EXIT_OK);
    }

    if post.keep_last_or_age {
        warn!(
            "--keep-last and --max-age-days are deprecated and ignored: github-backup keeps \
             one continuously updated backup per owner, and deleting directories by name \
             pattern was unsafe.  Rotate snapshots with your backup tool (restic, borg, ZFS)."
        );
    }

    // ── SHA-256 manifest (before the upload so the manifest is uploaded too) ──
    if post.manifest {
        let created_at = unix_secs_to_iso8601(started_at_unix);
        match write_manifest(&output.owner_json_dir(owner), &created_at) {
            Ok(n) => info!(entries = n, "SHA-256 manifest written"),
            Err(e) => record(stats, "manifest", format!("failed to write manifest: {e}")),
        }
    }

    // ── Diff with a previous backup (informational) ──────────────────────────
    if let Some(prev_dir) = &post.diff_with {
        match run_diff(prev_dir, &output.owner_json_dir(owner)) {
            Ok(summary) => info!(diff = %summary, "backup diff"),
            Err(e) => warn!(error = %e, "diff failed (non-fatal)"),
        }
    }

    // ── Restore ──────────────────────────────────────────────────────────────
    if post.restore {
        let target_org = post.restore_target_org.as_deref().unwrap_or(owner);
        if !restore::confirm_restore(target_org, post.restore_yes) {
            error!("restore aborted — pass --restore-yes to confirm non-interactively");
            return ExitCode::from(EXIT_FAILURE);
        }
        if let Err(e) = restore::run_restore(client, output, owner, target_org, false).await {
            error!("restore failed: {e}");
            return ExitCode::from(EXIT_FAILURE);
        }
    }

    // ── Mirror push ──────────────────────────────────────────────────────────
    if let Some(dest) = &post.mirror {
        if let Err(e) = run_mirror_push_dest(dest, output, owner).await {
            record(stats, "mirror push", errors::redact_secrets(&e.to_string()));
        }
    }

    // ── S3 sync ──────────────────────────────────────────────────────────────
    if let Some(s3) = &post.s3 {
        let options = S3RunOptions {
            include_assets: post.s3_include_assets,
            encrypt_key,
            delete_stale: post.s3_delete_stale,
            // An incomplete local copy must never remove a good remote one.
            allow_delete: !stats.has_failures(),
            dry_run: false,
        };
        if let Err(e) = run_s3_sync_with(s3, output, owner, &options).await {
            record(stats, "s3 sync", errors::redact_secrets(&e.to_string()));
        }
    }

    // ── From here on the failure list is final ───────────────────────────────
    let finished_at_unix = report::unix_now_secs();
    let elapsed = finished_at_unix.saturating_sub(started_at_unix);
    if !post.quiet {
        ui::print_summary_banner(stats, elapsed, false);
    }

    write_history(post, output, owner, stats, started_at_unix, elapsed);

    if let Some(path) = &post.report {
        match write_report(path, owner, stats, started_at_unix) {
            Ok(()) => info!(path = %path.display(), "wrote summary report"),
            Err(e) => error!("failed to write report: {e}"),
        }
    }
    if let Some(path) = &post.metrics {
        match write_prometheus_metrics(path, owner, stats, started_at_unix) {
            Ok(()) => info!(path = %path.display(), "wrote Prometheus metrics"),
            Err(e) => error!("failed to write Prometheus metrics: {e}"),
        }
    }

    let failures = stats.failures();
    let incomplete = !failures.is_empty();
    if let Some(url) = &post.webhook {
        notify::send_webhook(
            url,
            &Notification {
                status: if incomplete {
                    Status::Partial
                } else {
                    Status::Success
                },
                owner,
                error: None,
                repos_backed_up: stats.repos_backed_up(),
                repos_errored: stats.repos_errored(),
                failures: &failures,
            },
        )
        .await;
    }

    if incomplete {
        error!(
            failures = failures.len(),
            "backup is incomplete: {} item(s) could not be backed up (exit status {EXIT_INCOMPLETE})",
            failures.len()
        );
        ExitCode::from(EXIT_INCOMPLETE)
    } else {
        ExitCode::from(EXIT_OK)
    }
}

/// Records a post-processing failure in the shared failure list.
fn record(stats: &BackupStats, step: &str, message: String) {
    error!(step, "{message}");
    stats.record_failure(POST_SCOPE, step, message);
}

/// Appends this run to the rolling history shown by the TUI dashboard.
fn write_history(
    post: &Post,
    output: &OutputConfig,
    owner: &str,
    stats: &BackupStats,
    started_at_unix: u64,
    elapsed_secs: u64,
) {
    let path = output.backup_history_path(owner);
    let mut history = BackupRunHistory::load(&path).unwrap_or_default();
    history.push(
        BackupRunEntry {
            timestamp: unix_secs_to_iso8601(started_at_unix),
            repos_backed_up: stats.repos_backed_up(),
            elapsed_secs: elapsed_secs as f64,
            success: !stats.has_failures(),
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            failures: stats.failure_count() as u64,
        },
        post.history_size,
    );
    match history.save(&path) {
        Ok(()) => {
            info!(path = %path.display(), entries = history.entries.len(), "wrote backup history")
        }
        Err(e) => warn!(error = %e, "failed to write backup history file"),
    }
}
