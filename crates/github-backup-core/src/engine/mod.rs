// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! The top-level [`BackupEngine`] that orchestrates all backup categories.
//!
//! # Failure model
//!
//! A backup that loses part of its data must say so.  Every unit of work is an
//! isolated *step* (see `steps`): a failing step is recorded as a
//! [`Failure`](crate::stats::Failure) in the returned [`BackupStats`] and the
//! rest of the run carries on, so one bad repository or category never costs
//! the backup of everything else.  [`BackupEngine::run`] returns `Err` only
//! when the run cannot usefully continue ([`CoreError::is_fatal`]: rejected
//! credentials, exhausted rate limit, full disk, cancellation) or when the
//! repository list itself cannot be fetched.
//!
//! The caller decides what a run with failures means — the CLI exits with a
//! distinct status — by looking at [`BackupStats::has_failures`].
//!
//! # Incremental state
//!
//! See `incremental`: per-repository watermarks let a run skip the per-item
//! requests for issues and pull requests that have not changed, advancing only
//! for repositories that were backed up without any failure.

mod incremental;
mod repo;
mod steps;

use std::sync::Arc;

use tokio::sync::Semaphore;
use tracing::{error, info, warn};

use github_backup_client::BackupClient;
use github_backup_types::backup_state::BackupCheckpoint;
use github_backup_types::config::{BackupOptions, BackupTarget, OutputConfig};
use github_backup_types::{Raw, Repository};

use self::incremental::{Incremental, RunSummary};
use self::repo::{backup_one_repo, RepoContext, RepoResult};
use self::steps::{RunControl, Steps};
use crate::{
    backup::{
        gist::backup_gists, merge::merge_list, package::backup_packages,
        repository::should_include, starred_repos::backup_starred_repos,
        user_data::backup_user_data,
    },
    cancel::CancelFlag,
    error::CoreError,
    events::{EngineEvent, EngineEventTx},
    git::{CloneOptions, GitRunner},
    lock::{BackupLock, LockError},
    stats::BackupStats,
    storage::Storage,
};

/// Orchestrates a complete backup of a single GitHub owner (user or org).
///
/// The engine is generic over the API client ([`BackupClient`]), [`Storage`]
/// and [`GitRunner`] for compile-time dispatch and full testability with stub
/// implementations.
///
/// # Concurrency
///
/// Repository backups run in parallel up to `opts.concurrency`. Set it to `1`
/// for fully sequential operation. The API client, storage, and git runner must
/// all be `Send + Sync` (the production implementations satisfy this).
///
/// # Progress events
///
/// Attach a channel via [`BackupEngine::with_event_channel`] to receive
/// real-time [`EngineEvent`]s during the run.  The TUI uses this to drive the
/// repository list and progress bar.  The CLI does not need to attach a channel.
///
/// # Cancellation
///
/// [`BackupEngine::cancel_handle`] returns a [`CancelFlag`]; triggering it kills
/// running git processes promptly and makes [`BackupEngine::run`] return
/// [`CoreError::Interrupted`].
///
/// # Example
///
/// ```no_run
/// use github_backup_core::{BackupEngine, FsStorage, ProcessGitRunner};
/// use github_backup_client::GitHubClient;
/// use github_backup_types::config::{BackupOptions, Credential, OutputConfig};
///
/// # async fn example() -> Result<(), github_backup_core::CoreError> {
/// let cred = Credential::Token("ghp_xxx".to_string());
/// let client = GitHubClient::new(cred)?;
/// let storage = FsStorage::new();
/// let git = ProcessGitRunner::new();
/// let out = OutputConfig::new("/var/backup/github");
/// let opts = BackupOptions::all();
///
/// let engine = BackupEngine::new(client, storage, git, out, opts);
/// let stats = engine.run("octocat").await?;
/// println!("{stats}");
/// if stats.has_failures() {
///     eprintln!("incomplete: {} failure(s)", stats.failure_count());
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct BackupEngine<C, S, G> {
    client: C,
    storage: S,
    git: G,
    output: OutputConfig,
    opts: BackupOptions,
    /// Optional channel for real-time per-repo progress events.
    engine_events: Option<EngineEventTx>,
    cancel: CancelFlag,
}

impl<C, S, G> BackupEngine<C, S, G>
where
    C: BackupClient + Clone + 'static,
    S: Storage + Clone + 'static,
    G: GitRunner + Clone + 'static,
{
    /// Creates a new [`BackupEngine`].
    #[must_use]
    pub fn new(client: C, storage: S, git: G, output: OutputConfig, opts: BackupOptions) -> Self {
        Self {
            client,
            storage,
            git,
            output,
            opts,
            engine_events: None,
            cancel: CancelFlag::new(),
        }
    }

    /// Attaches an event channel for real-time progress reporting.
    ///
    /// The engine will send [`EngineEvent`]s on this channel during [`run`].
    /// Use [`tokio::sync::mpsc::unbounded_channel`] to create the matched
    /// receiver.
    ///
    /// [`run`]: Self::run
    #[must_use]
    pub fn with_event_channel(mut self, tx: EngineEventTx) -> Self {
        self.engine_events = Some(tx);
        self
    }

    /// Returns a handle that cancels this engine's run when triggered.
    ///
    /// Take it *before* calling [`run`](Self::run), which borrows the engine.
    #[must_use]
    pub fn cancel_handle(&self) -> CancelFlag {
        self.cancel.clone()
    }

    /// Runs the full backup for `owner`.
    ///
    /// - For user targets, fetches repositories via the user repos API.
    /// - For org targets, fetches repositories via the org repos API.
    ///
    /// Failures of individual steps are recorded in the returned
    /// [`BackupStats`] (see [`BackupStats::failures`]) and do not abort the
    /// run: a repository whose issues cannot be fetched still gets cloned.
    ///
    /// An advisory lock file is held for the duration of the run to prevent
    /// two concurrent backups from writing to the same directory.  A dry run
    /// writes nothing — not even the lock — and performs no git commands.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError`] when the run cannot usefully continue: the
    /// repository list cannot be fetched, the credentials are rejected, the
    /// rate-limit budget is exhausted, the disk is full, another backup holds
    /// the lock, or the run was cancelled.
    pub async fn run(&self, owner: &str) -> Result<BackupStats, CoreError> {
        let stats = BackupStats::new();
        let started_at = chrono::Utc::now();
        info!(owner, dry_run = self.opts.dry_run, "starting backup");

        if self.opts.dry_run {
            warn!("dry-run mode: no files will be written and no git commands will be run");
        }

        // ── Acquire advisory lock ──────────────────────────────────────────
        let json_dir = self.output.owner_json_dir(owner);
        let _lock = if self.opts.dry_run {
            None
        } else {
            Some(Self::acquire_lock(&json_dir, owner)?)
        };

        let control = RunControl::new(self.cancel.clone());
        let clone_opts = self.make_clone_opts();
        // The credential must never appear in a message that is stored or sent.
        let secrets: Arc<Vec<String>> = Arc::new(self.client.token().into_iter().collect());

        // ── Owner-level data ───────────────────────────────────────────────
        {
            let mut steps = Steps::new(owner, &stats, &control, &secrets);
            steps
                .run(
                    "owner data",
                    backup_user_data(&self.client, owner, &self.opts, &json_dir, &self.storage),
                )
                .await;
            steps
                .run(
                    "packages",
                    backup_packages(&self.client, owner, &self.opts, &json_dir, &self.storage),
                )
                .await;

            let starred = steps
                .run(
                    "starred clones",
                    backup_starred_repos(
                        &self.client,
                        &self.git,
                        owner,
                        &self.opts,
                        &self.output.starred_repos_dir(owner),
                        &self.output.starred_queue_path(owner),
                        &clone_opts,
                    ),
                )
                .await;
            for (repo, message) in starred.map(|s| s.failed).unwrap_or_default() {
                steps.record(&format!("starred clone {repo}"), &message);
            }

            let gists = steps
                .run(
                    "gists",
                    backup_gists(
                        &self.client,
                        owner,
                        &self.opts,
                        &self.output.gists_git_dir(owner),
                        &self.output.gists_meta_dir(owner),
                        &self.storage,
                        &self.git,
                        &clone_opts,
                    ),
                )
                .await;
            if let Some(gists) = gists {
                stats.add_gists(gists.count);
                for (id, message) in gists.failed {
                    steps.record(&format!("gist {id}"), &message);
                }
            }
        }
        if let Some(e) = control.take_stop_reason() {
            return Err(e);
        }

        // ── Repositories ───────────────────────────────────────────────────
        let repos = self.fetch_repos(owner).await?;
        let repo_count = repos.len();
        info!(owner, count = repo_count, "fetched repository list");
        stats.add_discovered(repo_count as u64);

        // Notify listeners of the total count before any per-repo events.
        self.emit(EngineEvent::ReposDiscovered {
            total: repo_count as u64,
        });

        if !self.opts.dry_run {
            self.write_repo_list(owner, &repos, &stats, &control, &secrets)
                .await;
        }

        let state_path = self.output.backup_state_path(owner);
        let incremental = Incremental::load(&state_path, &self.opts);

        self.backup_repos_concurrent(
            owner,
            repos,
            &stats,
            &incremental,
            &control,
            &secrets,
            &clone_opts,
        )
        .await;

        if let Some(e) = control.take_stop_reason() {
            // The checkpoint stays so the next invocation can resume.
            return Err(e);
        }

        // The run reached its end, so there is nothing to resume.  A repository
        // that failed was not marked in the checkpoint, and the next run starts
        // from scratch for it either way.
        let checkpoint_path = self.output.backup_checkpoint_path(owner);
        if let Err(e) = BackupCheckpoint::delete(&checkpoint_path) {
            warn!(error = %e, "failed to delete checkpoint file after the run");
        }

        if !self.opts.dry_run {
            let clean = stats.clean_repos();
            let summary = RunSummary {
                started_at,
                clean_repos: &clean,
                repos_backed_up: stats.repos_backed_up(),
                fully_successful: !stats.has_failures(),
            };
            match incremental.finish(&state_path, &summary) {
                Ok(()) => info!(path = %state_path.display(), "wrote backup state"),
                Err(e) => warn!(error = %e, "failed to write backup state file"),
            }
        }

        info!(owner, %stats, "backup finished");
        Ok(stats)
        // _lock is dropped here, releasing the advisory lock.
    }

    fn acquire_lock(json_dir: &std::path::Path, owner: &str) -> Result<BackupLock, CoreError> {
        match BackupLock::acquire(json_dir) {
            Ok(l) => Ok(l),
            Err(LockError::AlreadyRunning { pid }) => {
                let msg = match pid {
                    Some(p) => format!(
                        "another backup for '{owner}' is already running (PID {p}); \
                         wait for it to finish (the lock is released automatically if it dies)"
                    ),
                    None => format!(
                        "another backup for '{owner}' is already running; \
                         wait for it to finish (the lock is released automatically if it dies)"
                    ),
                };
                Err(CoreError::Io {
                    path: json_dir.display().to_string(),
                    source: std::io::Error::new(std::io::ErrorKind::AlreadyExists, msg),
                })
            }
            Err(LockError::DirCreate(e)) => Err(CoreError::Io {
                path: json_dir.display().to_string(),
                source: e,
            }),
            Err(LockError::Write(e)) => Err(CoreError::Io {
                path: json_dir.join(".backup.lock").display().to_string(),
                source: e,
            }),
        }
    }

    /// Sends an [`EngineEvent`] if a channel is attached.
    fn emit(&self, event: EngineEvent) {
        if let Some(ref tx) = self.engine_events {
            // Ignore send errors: the receiver may have been dropped (e.g.
            // the TUI was closed while a backup was still running).
            let _ = tx.send(event);
        }
    }

    /// Fetches the repository list using the user or org API as appropriate.
    async fn fetch_repos(&self, owner: &str) -> Result<Vec<Raw<Repository>>, CoreError> {
        match self.opts.target {
            BackupTarget::User => Ok(self.client.list_user_repos(owner).await?.into_items()),
            BackupTarget::Org => Ok(self.client.list_org_repos(owner).await?.into_items()),
        }
    }

    /// Writes `repos.json`: the listing of every repository this run covers,
    /// merged with earlier listings so a repository deleted on GitHub stays
    /// recorded.  Repositories the options exclude (forks, private, filters) are
    /// left out, so excluding something also keeps its metadata out of the
    /// backup directory.
    async fn write_repo_list(
        &self,
        owner: &str,
        repos: &[Raw<Repository>],
        stats: &BackupStats,
        control: &RunControl,
        secrets: &[String],
    ) {
        let included: Vec<&Raw<Repository>> = repos
            .iter()
            .filter(|r| should_include(r, &self.opts))
            .collect();
        let path = self.output.owner_json_dir(owner).join("repos.json");
        let mut steps = Steps::new(owner, stats, control, secrets);
        steps
            .run("repository list", async {
                let merged = merge_list(&self.storage, &path, &included, "id")?;
                self.storage.write_json(&path, &merged)
            })
            .await;
    }

    /// Backs up repositories concurrently, honouring `opts.concurrency`.
    ///
    /// Supports **resumption**: loads the checkpoint file (if any) and skips
    /// repositories already completed in a previous interrupted run.  After
    /// each repository completes *cleanly* the checkpoint is updated atomically;
    /// a repository with a failed step is not marked, so a resumed run retries it.
    #[allow(clippy::too_many_arguments)]
    async fn backup_repos_concurrent(
        &self,
        owner: &str,
        repos: Vec<Raw<Repository>>,
        stats: &BackupStats,
        incremental: &Incremental,
        control: &RunControl,
        secrets: &Arc<Vec<String>>,
        clone_opts: &CloneOptions,
    ) {
        let total = repos.len();
        let checkpoint_path = self.output.backup_checkpoint_path(owner);

        // Load any existing checkpoint from an interrupted prior run.
        let checkpoint = match BackupCheckpoint::load(&checkpoint_path) {
            Ok(cp) if !cp.completed_repos.is_empty() && checkpoint_is_stale(&cp) => {
                warn!(
                    last_activity = %cp.last_updated_at,
                    "ignoring a checkpoint that is too old to resume: every repository is refreshed"
                );
                Arc::new(tokio::sync::Mutex::new(BackupCheckpoint::default()))
            }
            Ok(cp) => {
                let resumed = cp.completed_repos.len();
                if resumed > 0 {
                    info!(
                        owner,
                        resumed,
                        total,
                        "resuming interrupted backup — skipping already-completed repositories"
                    );
                }
                Arc::new(tokio::sync::Mutex::new(cp))
            }
            Err(e) => {
                warn!(error = %e, "failed to load checkpoint; starting fresh");
                Arc::new(tokio::sync::Mutex::new(BackupCheckpoint::default()))
            }
        };

        let concurrency = self.opts.concurrency.max(1);
        let sem = Arc::new(Semaphore::new(concurrency));
        let completed_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let mut handles = Vec::with_capacity(repos.len());

        for repo in repos {
            if control.should_stop() {
                break;
            }

            // Skip repositories already completed in a prior interrupted run.
            {
                let cp = checkpoint.lock().await;
                if cp.is_complete(&repo.full_name) {
                    stats.inc_skipped();
                    completed_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    continue;
                }
            }

            let permit = Arc::clone(&sem)
                .acquire_owned()
                .await
                .expect("semaphore closed");
            // Something fatal may have happened while this waited for a slot.
            if control.should_stop() {
                break;
            }

            // Clone fields needed by the spawned task.
            let client = self.client.clone();
            let storage = self.storage.clone();
            let git = self.git.clone();
            let output = self.output.clone();
            let opts = self.opts.clone();
            let owner_str = owner.to_string();
            let clone_opts = clone_opts.clone();
            let task_stats = stats.handle();
            let cp = Arc::clone(&checkpoint);
            let cp_path = checkpoint_path.clone();
            let done_count = Arc::clone(&completed_count);
            let task_control = control.clone();
            let task_secrets = Arc::clone(secrets);
            let since = incremental.since_for(&repo.full_name);
            // Clone the event sender so the task can emit per-repo events.
            let event_tx = self.engine_events.clone();
            let name = repo.full_name.clone();

            let handle = tokio::spawn(async move {
                let _permit = permit; // released when task completes

                // Notify listeners that this repo is starting.
                if let Some(ref tx) = event_tx {
                    let _ = tx.send(EngineEvent::RepoStarted {
                        name: repo.full_name.clone(),
                    });
                }

                let ctx = RepoContext {
                    client: &client,
                    storage: &storage,
                    git: &git,
                    output: &output,
                    opts: &opts,
                    owner: &owner_str,
                    clone_opts: &clone_opts,
                    stats: &task_stats,
                    control: &task_control,
                    secrets: &task_secrets,
                    since: since.as_deref(),
                };
                let result = backup_one_repo(&ctx, &repo).await;

                let current = done_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                info!(
                    repo = %repo.full_name,
                    progress = format!("{current}/{total}"),
                    "repository processed"
                );

                match result {
                    RepoResult::Clean => {
                        task_stats.inc_backed_up();
                        task_stats.mark_repo_clean(&repo.full_name);
                        if let Some(ref tx) = event_tx {
                            let _ = tx.send(EngineEvent::RepoCompleted {
                                name: repo.full_name.clone(),
                                success: true,
                                error: None,
                            });
                        }
                        // Mark complete in the checkpoint.
                        let mut guard = cp.lock().await;
                        let now =
                            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                        if guard.run_started_at.is_empty() {
                            guard.run_started_at = now.clone();
                        }
                        guard.last_updated_at = now;
                        if let Err(e) = guard.mark_complete_and_save(&repo.full_name, &cp_path) {
                            warn!(
                                repo = %repo.full_name,
                                error = %e,
                                "failed to update checkpoint"
                            );
                        }
                    }
                    // Filtered out or dry run: nothing was attempted.
                    RepoResult::Skipped => task_stats.inc_skipped(),
                    RepoResult::Failed(summary) => {
                        task_stats.inc_errored();
                        error!(
                            repo = %repo.full_name,
                            error = %summary,
                            "repository backup incomplete, continuing"
                        );
                        if let Some(ref tx) = event_tx {
                            let _ = tx.send(EngineEvent::RepoCompleted {
                                name: repo.full_name.clone(),
                                success: false,
                                error: Some(summary),
                            });
                        }
                    }
                }
            });
            handles.push((name, handle));
        }

        for (name, handle) in handles {
            // A panicking task must not abort the whole backup, but it must be
            // reported: the repository was not backed up.
            if let Err(e) = handle.await {
                error!(repo = %name, error = %e, "repository backup task panicked");
                stats.inc_errored();
                stats.record_failure(&name, "task", format!("backup task panicked: {e}"));
            }
        }
    }

    /// Builds [`CloneOptions`] from the current `BackupOptions`.
    fn make_clone_opts(&self) -> CloneOptions {
        let token = match self.opts.prefer_ssh {
            // SSH uses key-based auth; no token needed in clone opts.
            true => None,
            false => {
                // Extract token from the client's credential for injection.
                self.client.token()
            }
        };
        CloneOptions {
            token,
            no_prune: self.opts.no_prune,
            cancel: self.cancel.clone(),
            ..CloneOptions::default()
        }
    }
}

/// How long after its last activity an interrupted run's checkpoint is still
/// resumed.
const CHECKPOINT_MAX_AGE: chrono::Duration = chrono::Duration::hours(6);

/// `true` if `cp` is too old (or of unknown age) to resume.
fn checkpoint_is_stale(cp: &BackupCheckpoint) -> bool {
    match chrono::DateTime::parse_from_rfc3339(&cp.last_updated_at) {
        Ok(last) => chrono::Utc::now().signed_duration_since(last) > CHECKPOINT_MAX_AGE,
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests;
