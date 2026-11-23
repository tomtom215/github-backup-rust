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
//!
//! A run interrupted by a signal — during the backup or during the
//! post-processing — still writes its history entry, report and metrics,
//! marked unsuccessful, and sends an `interrupted` notification.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tracing::{error, info, warn};

use github_backup_client::GitHubClient;
use github_backup_core::ProcessGitRunner;
use github_backup_core::{write_manifest, BackupEngine, BackupStats, CoreError, FsStorage};
use github_backup_s3::config::S3Config;
use github_backup_types::backup_state::{BackupRunEntry, BackupRunHistory};
use github_backup_types::config::{BackupTarget, Credential, OutputConfig};

use crate::cli::Args;
use crate::metrics::write_prometheus_metrics;
use crate::notify::{self, Notification, NotifyFormat, Status};
use crate::post_process::{
    build_mirror_dest, build_s3_config, run_diff, run_mirror_push_dest, run_s3_sync_with,
    validate_post_processing, MirrorDest, S3RunOptions,
};
use crate::report::{self, unix_secs_to_iso8601, write_report};
use crate::shutdown::Shutdown;
use crate::{errors, lock, restore, shutdown, ui};

/// Every item was backed up.
pub(crate) const EXIT_OK: u8 = 0;
/// The run could not be carried out.
pub(crate) const EXIT_FAILURE: u8 = 1;
/// The run finished, but some items could not be backed up.
pub(crate) const EXIT_INCOMPLETE: u8 = 3;

/// The failure scope used for everything that happens after the engine.
const POST_SCOPE: &str = "post-processing";

/// How long an interrupted run may spend on its notification: the watchdog
/// ends the process [`shutdown::SHUTDOWN_GRACE`] after the signal.
const INTERRUPTED_NOTIFY_LIMIT: Duration = Duration::from_secs(3);

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
    webhook: Option<String>,
    notify_format: NotifyFormat,
    history_size: usize,
    quiet: bool,
    dry_run: bool,
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
            webhook: args.notify_webhook.clone(),
            notify_format: args.notify_format,
            history_size: args.history_size,
            quiet: args.quiet,
            dry_run: args.dry_run,
        }
    }
}

/// The time base of one run.
#[derive(Clone, Copy)]
struct Clock {
    started_at_unix: u64,
    started: Instant,
}

impl Clock {
    fn start() -> Self {
        Self {
            started_at_unix: report::unix_now_secs(),
            started: Instant::now(),
        }
    }

    /// Seconds since the run started, post-processing included.
    fn elapsed_secs(self) -> f64 {
        self.started.elapsed().as_secs_f64()
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

/// Runs the backup described by `args` and everything that follows it, and
/// returns the exit status.
pub(crate) async fn execute(
    args: Args,
    credential: Credential,
    encrypt_key: Option<&[u8; 32]>,
    shutdown: &mut Shutdown,
) -> u8 {
    let post = Post::from_args(&args);
    // Unusable mirror or S3 settings are reported now, not after the backup.
    if !post.dry_run {
        if let Err(e) = validate_post_processing(&args) {
            error!("{e}");
            return EXIT_FAILURE;
        }
    }
    let api_url = args.api_url.clone();
    let defaulted = args.uses_default_categories();
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
                return EXIT_FAILURE;
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
            return EXIT_FAILURE;
        }
    };

    check_token_scopes(&client, &opts).await;

    let clock = Clock::start();

    // A short "what is about to happen" plan, skipped under --quiet so cron
    // and journal scrapes stay clean.
    if !post.quiet {
        ui::print_plan(&ui::Plan {
            owner: &owner,
            output_path: &output_path,
            opts: &opts,
            defaulted,
            output: &output,
        });
    }

    let clones = opts.repositories;
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
        code = shutdown.recv() => {
            warn!(
                exit_code = code,
                "backup interrupted by signal — partial data may remain on disk; \
                 re-run to resume"
            );
            shutdown::begin_shutdown(&cancel, code);
            // Steps abandon in-flight requests and git is killed, so this is
            // quick; the bound keeps a stuck task from outlasting the watchdog.
            let stats = match tokio::time::timeout(Duration::from_secs(4), engine_run).await {
                Ok(Ok(stats)) => stats,
                _ => BackupStats::new(),
            };
            return interrupted(&post, &output, &owner, &stats, clock, code).await;
        }
    };

    let stats = match backup_result {
        Ok(stats) => stats,
        Err(e) => return fatal(&post, &output, &owner, clock, &e).await,
    };

    // Post-processing can take long (mirror pushes, S3 uploads): a signal
    // must stop it as promptly as it stops the backup.
    let post_processing = post_process(&post, &output, &owner, &stats, clock, encrypt_key);
    tokio::select! {
        () = post_processing => {}
        code = shutdown.recv() => {
            warn!(exit_code = code, "post-processing interrupted by signal");
            shutdown::begin_shutdown(&cancel, code);
            return interrupted(&post, &output, &owner, &stats, clock, code).await;
        }
    }

    tokio::select! {
        code = report_outcome(&post, &output, &owner, &stats, clock, clones) => code,
        code = shutdown.recv() => {
            // The report, metrics and history are already written; only the
            // notification was still on its way.
            warn!(exit_code = code, "interrupted while sending the notification");
            code
        }
    }
}

/// `--restore`: re-creates labels, milestones and issues from the **local
/// backup** in the target organisation.
///
/// It is a mode of its own — no backup is made first and the source account is
/// not contacted — so it works when the source repository or account is gone.
/// Confirmation is asked before anything else happens.
pub(crate) async fn execute_restore(args: Args, credential: Credential) -> u8 {
    let yes = args.restore_yes;
    let dry_run = args.dry_run;
    let into_source = args.restore_into_source;
    let target_org_arg = args.restore_target_org.clone();
    let api_url = args.api_url.clone();
    let (owner, output_path, _opts) = args.into_backup_options();
    let target_org = match restore::choose_target(&owner, target_org_arg.as_deref(), into_source) {
        Ok(t) => t,
        Err(why) => {
            error!("{why}");
            return EXIT_FAILURE;
        }
    };
    let output = OutputConfig::new(&output_path);

    if !dry_run && !restore::confirm_restore(&owner, &target_org, yes) {
        error!("restore aborted — pass --restore-yes to confirm non-interactively");
        return EXIT_FAILURE;
    }

    let client = match api_url.as_deref() {
        Some(url) => GitHubClient::with_api_url(credential, url),
        None => GitHubClient::new(credential),
    };
    let client = match client {
        Ok(c) => c,
        Err(e) => {
            error!("failed to initialise GitHub client: {e}");
            return EXIT_FAILURE;
        }
    };

    match restore::run_restore(&client, &output, &owner, &target_org, dry_run).await {
        Ok(stats) if stats.errored() > 0 => {
            error!(
                errored = stats.errored(),
                "restore finished with errors: {} item(s) could not be restored (exit status {EXIT_INCOMPLETE})",
                stats.errored()
            );
            EXIT_INCOMPLETE
        }
        Ok(_) => EXIT_OK,
        Err(e) => {
            error!("restore failed: {e}");
            EXIT_FAILURE
        }
    }
}

/// Reports a run that could not be completed.
///
/// The report, metrics and history are written too — with the run marked failed
/// — so monitors reading them do not keep seeing the previous run's success.
async fn fatal(post: &Post, output: &OutputConfig, owner: &str, clock: Clock, e: &CoreError) -> u8 {
    let raw = errors::redact_secrets(&e.to_string());
    error!("backup failed: {raw}");
    if let Some(hint) = errors::explain_error(&raw) {
        error!("hint: {hint}");
    }
    if post.dry_run {
        return EXIT_FAILURE;
    }

    let stats = BackupStats::new();
    stats.record_failure(owner, "run", raw.clone());
    write_records(post, output, owner, &stats, clock);
    notify(post, owner, &stats, Status::Failure, Some(&raw)).await;
    EXIT_FAILURE
}

/// Records a run stopped by a signal: history, report and metrics marked
/// unsuccessful, and a (time-bounded) `interrupted` notification.
async fn interrupted(
    post: &Post,
    output: &OutputConfig,
    owner: &str,
    stats: &BackupStats,
    clock: Clock,
    code: u8,
) -> u8 {
    if post.dry_run {
        return code;
    }
    stats.record_failure(
        owner,
        "run",
        format!("interrupted by a signal (exit status {code})"),
    );
    write_records(post, output, owner, stats, clock);
    let send = notify(post, owner, stats, Status::Interrupted, None);
    if tokio::time::timeout(INTERRUPTED_NOTIFY_LIMIT, send)
        .await
        .is_err()
    {
        warn!("the interrupted-run notification did not finish in time");
    }
    code
}

/// Writes the run history entry and the `--report` / `--prometheus-metrics`
/// files.
fn write_records(
    post: &Post,
    output: &OutputConfig,
    owner: &str,
    stats: &BackupStats,
    clock: Clock,
) {
    let elapsed = clock.elapsed_secs();
    write_history(post, output, owner, stats, clock.started_at_unix, elapsed);
    if let Some(path) = &post.report {
        match write_report(path, owner, stats, clock.started_at_unix, elapsed) {
            Ok(()) => info!(path = %path.display(), "wrote summary report"),
            Err(e) => error!("failed to write report: {e}"),
        }
    }
    if let Some(path) = &post.metrics {
        match write_prometheus_metrics(path, owner, stats, clock.started_at_unix, elapsed) {
            Ok(()) => info!(path = %path.display(), "wrote Prometheus metrics"),
            Err(e) => error!("failed to write Prometheus metrics: {e}"),
        }
    }
}

/// Sends the `--notify-webhook` notification, if one is configured.
async fn notify(
    post: &Post,
    owner: &str,
    stats: &BackupStats,
    status: Status,
    error: Option<&str>,
) {
    let Some(url) = &post.webhook else {
        return;
    };
    notify::send_webhook(
        url,
        post.notify_format,
        &Notification {
            status,
            owner,
            error,
            repos_backed_up: stats.repos_backed_up(),
            repos_errored: stats.repos_errored(),
            failures: &stats.failures(),
        },
    )
    .await;
}

/// The steps after a completed engine run that can add failures: manifest,
/// diff, mirror push and S3 sync.  Nothing here runs for a dry run.
async fn post_process(
    post: &Post,
    output: &OutputConfig,
    owner: &str,
    stats: &BackupStats,
    clock: Clock,
    encrypt_key: Option<&[u8; 32]>,
) {
    if post.dry_run {
        return;
    }

    // ── SHA-256 manifest (before the upload so the manifest is uploaded too) ──
    if post.manifest {
        let created_at = unix_secs_to_iso8601(clock.started_at_unix);
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

    // ── Mirror push ──────────────────────────────────────────────────────────
    if let Some(dest) = &post.mirror {
        if let Err(e) = run_mirror_push_dest(dest, output, owner, stats).await {
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
}

/// The end of a completed run: summary, history, report, metrics, webhook and
/// the exit status.  From here on the failure list is final.
async fn report_outcome(
    post: &Post,
    output: &OutputConfig,
    owner: &str,
    stats: &BackupStats,
    clock: Clock,
    clones: bool,
) -> u8 {
    let elapsed = clock.elapsed_secs() as u64;
    if post.dry_run {
        // A dry run changes nothing: no manifest, mirror push, S3 sync, report,
        // metrics, history or webhook.
        if !post.quiet {
            ui::print_summary_banner(stats, elapsed, true, clones);
        }
        info!("dry run: skipped state, report, metrics, mirror push, S3 sync and notification");
        return EXIT_OK;
    }

    if !post.quiet {
        ui::print_summary_banner(stats, elapsed, false, clones);
    }
    write_records(post, output, owner, stats, clock);

    let incomplete = stats.has_failures();
    let status = if incomplete {
        Status::Partial
    } else {
        Status::Success
    };
    notify(post, owner, stats, status, None).await;

    if incomplete {
        let failures = stats.failure_count();
        error!(
            failures,
            "backup is incomplete: {failures} item(s) could not be backed up (exit status {EXIT_INCOMPLETE})",
        );
        EXIT_INCOMPLETE
    } else {
        EXIT_OK
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
    elapsed_secs: f64,
) {
    let path = output.backup_history_path(owner);
    let mut history = BackupRunHistory::load(&path).unwrap_or_default();
    history.push(
        BackupRunEntry {
            timestamp: unix_secs_to_iso8601(started_at_unix),
            repos_backed_up: stats.repos_backed_up(),
            elapsed_secs,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn post(dir: &std::path::Path) -> Post {
        Post {
            report: Some(dir.join("report.json")),
            mirror: None,
            s3: None,
            s3_include_assets: false,
            s3_delete_stale: false,
            manifest: false,
            metrics: Some(dir.join("metrics.prom")),
            diff_with: None,
            webhook: None,
            notify_format: NotifyFormat::Json,
            history_size: 20,
            quiet: true,
            dry_run: false,
        }
    }

    /// An interrupted run used to write nothing, so monitors kept showing the
    /// previous run's success.
    #[tokio::test]
    async fn an_interrupted_run_writes_report_metrics_and_history_as_failed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let output = OutputConfig::new(dir.path());
        let post = post(dir.path());
        let stats = BackupStats::new();
        let code = interrupted(&post, &output, "octocat", &stats, Clock::start(), 143).await;
        assert_eq!(code, 143);

        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("report.json")).unwrap())
                .unwrap();
        assert_eq!(report["success"], false);
        assert!(report["failures"][0]["message"]
            .as_str()
            .unwrap()
            .contains("interrupted"));
        let metrics = std::fs::read_to_string(dir.path().join("metrics.prom")).unwrap();
        assert!(metrics.contains("github_backup_success{owner=\"octocat\"} 0"));
        let history = BackupRunHistory::load(&output.backup_history_path("octocat")).unwrap();
        assert!(!history.entries.last().expect("entry").success);
    }

    /// The fatal path built a fresh `BackupStats`, so its report said the run
    /// took 0 seconds whatever it really took.
    #[tokio::test]
    async fn a_fatal_run_reports_its_real_duration() {
        let dir = tempfile::tempdir().expect("tempdir");
        let output = OutputConfig::new(dir.path());
        let post = post(dir.path());
        let clock = Clock {
            started_at_unix: report::unix_now_secs(),
            started: Instant::now() - Duration::from_secs(90),
        };
        let code = fatal(&post, &output, "octocat", clock, &CoreError::Interrupted).await;
        assert_eq!(code, EXIT_FAILURE);
        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("report.json")).unwrap())
                .unwrap();
        assert!(
            report["duration_secs"].as_f64().unwrap() >= 90.0,
            "{report}"
        );
    }
}
