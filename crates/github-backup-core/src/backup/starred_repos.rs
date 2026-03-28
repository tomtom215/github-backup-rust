// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Starred-repository clone with durable queue, retry/backoff, and progress.
//!
//! # Overview
//!
//! When `opts.clone_starred` is enabled this module:
//!
//! 1. Fetches the owner's full starred-repository list from the GitHub API.
//! 2. Loads (or creates) a durable JSON queue at `queue_path` and merges any
//!    newly starred repos into it as `Pending` items.
//! 3. Processes every `Pending` item sequentially:
//!    - Clones or updates the repository into `starred_dir/<owner>/<repo>.git`
//!      using the same clone mode as owned repos (`opts.clone_type`).
//!    - On failure, retries up to [`MAX_ATTEMPTS`] times with exponential
//!      backoff ([`RETRY_DELAYS_SECS`]).
//!    - Saves the queue to disk after every item (atomic rename, crash-safe).
//!    - Checks for a Ctrl+C signal before each item; saves and exits cleanly
//!      if one is received.
//! 4. Logs structured progress after every clone:
//!    `done`, `pending`, `failed`, `total`, `rate_per_min`, `eta_secs`.
//!
//! # Resuming
//!
//! Re-running with `--clone-starred` picks up where the previous run stopped.
//! `Done` items are skipped; `Failed` items remain skipped unless manually
//! reset to `"pending"` in the queue file.
//!
//! [`MAX_ATTEMPTS`]: crate::starred_queue::MAX_ATTEMPTS
//! [`RETRY_DELAYS_SECS`]: crate::starred_queue::RETRY_DELAYS_SECS

use std::path::Path;
use std::time::Instant;

use chrono::Utc;
use tracing::{info, warn};

use github_backup_client::BackupClient;
use github_backup_types::config::{BackupOptions, CloneType};
use github_backup_types::starred_queue::CloneState;

use crate::{
    error::CoreError,
    git::{CloneOptions, GitRunner},
    starred_queue::{self, MAX_ATTEMPTS, RETRY_DELAYS_SECS},
};

// ── Public entry point ────────────────────────────────────────────────────────

/// Clones or updates every starred repository for `username`.
///
/// Reads and writes a durable queue at `queue_path`.  Repositories that were
/// already successfully cloned in a previous run are skipped automatically.
///
/// Returns immediately (no-op) when `opts.clone_starred` is `false` or
/// `opts.dry_run` is `true`.
///
/// # Arguments
///
/// - `client` — GitHub API client used to fetch the starred list.
/// - `git` — Git runner used to perform the actual clones.
/// - `username` — GitHub user whose starred repos are being backed up.
/// - `opts` — Backup options (clone type, SSH preference, etc.).
/// - `starred_dir` — Root directory for cloned repos:
///   `<starred_dir>/<repo_owner>/<repo_name>.git`.
/// - `queue_path` — Path to the JSON queue file (created if absent).
/// - `clone_opts` — Token and no-prune settings passed to git.
///
/// # Errors
///
/// Returns [`CoreError`] on fatal API errors or queue I/O failures.
/// Per-repo clone errors are retried and ultimately recorded as `Failed`
/// in the queue rather than aborting the run.
pub async fn backup_starred_repos(
    client: &impl BackupClient,
    git: &impl GitRunner,
    username: &str,
    opts: &BackupOptions,
    starred_dir: &Path,
    queue_path: &Path,
    clone_opts: &CloneOptions,
) -> Result<(), CoreError> {
    if !opts.clone_starred {
        return Ok(());
    }

    if opts.dry_run {
        info!(username, "dry-run: skipping starred repos clone");
        return Ok(());
    }

    // ── Fetch starred list ────────────────────────────────────────────────────
    info!(username, "fetching starred repositories for clone queue");
    let starred = client.list_starred(username).await?;
    info!(
        username,
        count = starred.len(),
        "starred repositories discovered"
    );

    // ── Load or create queue ──────────────────────────────────────────────────
    let mut queue = starred_queue::load_or_create(queue_path, username, &starred)?;
    let initial = starred_queue::compute_stats(&queue);

    info!(
        total = initial.total,
        done = initial.done,
        pending = initial.pending,
        failed = initial.failed,
        queue_path = %queue_path.display(),
        "queue loaded; starting clone run"
    );

    if initial.pending == 0 {
        info!(
            done = initial.done,
            failed = initial.failed,
            "all starred repos already processed; nothing to clone"
        );
        return Ok(());
    }

    // ── Shutdown signal (Ctrl+C) ──────────────────────────────────────────────
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::info!("Ctrl+C received; finishing current clone and stopping");
            let _ = shutdown_tx.send(true);
        }
    });

    // ── Process queue ─────────────────────────────────────────────────────────
    let run_start = Instant::now();
    let mut cloned_this_run: u64 = 0;

    for idx in 0..queue.items.len() {
        if queue.items[idx].state != CloneState::Pending {
            continue;
        }

        // Graceful shutdown: save and exit.
        if *shutdown_rx.borrow() {
            info!("shutdown signal received; saving queue and stopping");
            starred_queue::save(&mut queue, queue_path)?;
            break;
        }

        // Snapshot fields needed during the retry loop to avoid borrow issues.
        let full_name = queue.items[idx].full_name.clone();
        let size_kb = queue.items[idx].size_kb;
        let url = if opts.prefer_ssh {
            queue.items[idx].ssh_url.clone()
        } else {
            queue.items[idx].clone_url.clone()
        };

        // Destination: `<starred_dir>/<upstream-owner>/<repo>.git`
        let dest = starred_dir
            .join(queue.items[idx].repo_owner())
            .join(format!("{}.git", queue.items[idx].repo_name()));

        info!(
            repo = %full_name,
            size_kb,
            dest = %dest.display(),
            "cloning starred repository"
        );

        // ── Retry loop ──────────────────────────────────────────────────────
        let mut success = false;
        let mut last_err: Option<String> = None;

        for attempt in 0..MAX_ATTEMPTS {
            match do_clone(git, &url, &dest, opts, clone_opts) {
                Ok(()) => {
                    success = true;
                    break;
                }
                Err(e) => {
                    last_err = Some(e.to_string());
                    queue.items[idx].retries = attempt + 1;
                    queue.items[idx].last_error = last_err.clone();

                    if attempt + 1 >= MAX_ATTEMPTS {
                        break; // retries exhausted
                    }

                    let delay = RETRY_DELAYS_SECS[attempt as usize];
                    warn!(
                        repo = %full_name,
                        attempt = attempt + 1,
                        max_attempts = MAX_ATTEMPTS,
                        delay_secs = delay,
                        error = %e,
                        "clone failed; retrying with backoff"
                    );

                    // Persist progress before sleeping so a crash during
                    // the sleep doesn't lose the retry count.
                    starred_queue::save(&mut queue, queue_path)?;
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;

                    if *shutdown_rx.borrow() {
                        info!("shutdown during backoff; saving and stopping");
                        starred_queue::save(&mut queue, queue_path)?;
                        return Ok(());
                    }
                }
            }
        }

        // ── Record outcome ──────────────────────────────────────────────────
        let now = Utc::now().to_rfc3339();
        if success {
            queue.items[idx].state = CloneState::Done;
            queue.items[idx].finished_at = Some(now);
            queue.items[idx].last_error = None;
            cloned_this_run += 1;

            let stats = starred_queue::compute_stats(&queue);
            let (rate, eta) = compute_rate_eta(cloned_this_run, run_start.elapsed(), stats.pending);

            info!(
                repo = %full_name,
                done = stats.done,
                pending = stats.pending,
                failed = stats.failed,
                total = stats.total,
                rate_per_min = format_rate(rate),
                eta_secs = eta,
                "starred repo cloned"
            );
        } else {
            queue.items[idx].state = CloneState::Failed;
            queue.items[idx].finished_at = Some(now);
            warn!(
                repo = %full_name,
                retries = queue.items[idx].retries,
                error = ?last_err,
                "starred repo permanently failed after max retries"
            );
        }

        // Persist after every item.
        starred_queue::save(&mut queue, queue_path)?;
    }

    // ── Final summary ─────────────────────────────────────────────────────────
    let final_stats = starred_queue::compute_stats(&queue);
    info!(
        cloned_this_run,
        done = final_stats.done,
        failed = final_stats.failed,
        pending = final_stats.pending,
        total = final_stats.total,
        elapsed_secs = run_start.elapsed().as_secs(),
        "starred repos clone run complete"
    );

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Dispatches to the appropriate [`GitRunner`] method based on `opts.clone_type`.
fn do_clone(
    git: &impl GitRunner,
    url: &str,
    dest: &Path,
    opts: &BackupOptions,
    clone_opts: &CloneOptions,
) -> Result<(), CoreError> {
    if opts.lfs {
        return git.lfs_clone(url, dest, clone_opts);
    }
    match &opts.clone_type {
        CloneType::Mirror => git.mirror_clone(url, dest, clone_opts),
        CloneType::Bare => git.bare_clone(url, dest, clone_opts),
        CloneType::Full => git.full_clone(url, dest, clone_opts),
        CloneType::Shallow(depth) => git.shallow_clone(url, dest, clone_opts, *depth),
    }
}

/// Returns `(rate_per_min, eta_secs)` from run-level counters.
///
/// Rate is repos cloned per minute; ETA is estimated seconds until all
/// `pending` items are done.  Both return `0` / `None` until at least a
/// few seconds of data are available.
fn compute_rate_eta(
    cloned: u64,
    elapsed: std::time::Duration,
    pending: usize,
) -> (f64, Option<u64>) {
    let elapsed_secs = elapsed.as_secs_f64();
    if elapsed_secs < 1.0 || cloned == 0 {
        return (0.0, None);
    }
    let rate = cloned as f64 / elapsed_secs * 60.0; // repos / min
    let eta = if rate > 0.0 {
        Some((pending as f64 / rate * 60.0) as u64)
    } else {
        None
    };
    (rate, eta)
}

/// Formats rate as a 1-decimal-place string suitable for structured logging.
fn format_rate(rate: f64) -> String {
    format!("{rate:.1}")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::git::test_support::SpyGitRunner;
    use github_backup_types::config::{BackupOptions, CloneType};
    use tempfile::TempDir;

    fn clone_opts() -> CloneOptions {
        CloneOptions::unauthenticated()
    }

    fn starred_opts() -> BackupOptions {
        BackupOptions {
            clone_starred: true,
            clone_type: CloneType::Mirror,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn disabled_flag_is_noop() {
        let client = MockBackupClient::new();
        let git = SpyGitRunner::default();
        let dir = TempDir::new().unwrap();
        let opts = BackupOptions::default(); // clone_starred = false

        backup_starred_repos(
            &client,
            &git,
            "octocat",
            &opts,
            dir.path(),
            &dir.path().join("queue.json"),
            &clone_opts(),
        )
        .await
        .expect("noop");

        assert_eq!(git.recorded_calls().len(), 0, "no git calls for disabled flag");
    }

    #[tokio::test]
    async fn dry_run_is_noop() {
        let client = MockBackupClient::new();
        let git = SpyGitRunner::default();
        let dir = TempDir::new().unwrap();
        let opts = BackupOptions {
            clone_starred: true,
            dry_run: true,
            ..Default::default()
        };

        backup_starred_repos(
            &client,
            &git,
            "octocat",
            &opts,
            dir.path(),
            &dir.path().join("queue.json"),
            &clone_opts(),
        )
        .await
        .expect("dry_run noop");

        assert_eq!(git.recorded_calls().len(), 0, "dry-run must not clone");
    }

    #[tokio::test]
    async fn empty_starred_list_writes_queue_with_no_items() {
        let client = MockBackupClient::new(); // default: empty starred list
        let git = SpyGitRunner::default();
        let dir = TempDir::new().unwrap();
        let queue_path = dir.path().join("queue.json");

        backup_starred_repos(
            &client,
            &git,
            "octocat",
            &starred_opts(),
            dir.path(),
            &queue_path,
            &clone_opts(),
        )
        .await
        .expect("empty list");

        assert_eq!(git.recorded_calls().len(), 0, "nothing to clone");
    }

    #[tokio::test]
    async fn repos_are_cloned_into_subdirectory() {
        use github_backup_types::Repository;
        use github_backup_types::user::User;

        let repo = Repository {
            id: 1,
            full_name: "rust-lang/rust".to_string(),
            name: "rust".to_string(),
            owner: User {
                id: 42,
                login: "rust-lang".to_string(),
                user_type: "Organization".to_string(),
                avatar_url: String::new(),
                html_url: String::new(),
            },
            private: false,
            fork: false,
            archived: false,
            disabled: false,
            description: None,
            clone_url: "https://github.com/rust-lang/rust.git".to_string(),
            ssh_url: "git@github.com:rust-lang/rust.git".to_string(),
            default_branch: "master".to_string(),
            size: 500_000,
            has_issues: true,
            has_wiki: true,
            created_at: "2010-01-01T00:00:00Z".to_string(),
            pushed_at: None,
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            html_url: "https://github.com/rust-lang/rust".to_string(),
        };

        let client = MockBackupClient::new().with_starred(vec![repo]);
        let git = SpyGitRunner::default();
        let dir = TempDir::new().unwrap();
        let queue_path = dir.path().join("queue.json");

        backup_starred_repos(
            &client,
            &git,
            "octocat",
            &starred_opts(),
            dir.path(),
            &queue_path,
            &clone_opts(),
        )
        .await
        .expect("clone starred");

        let calls = git.recorded_calls();
        assert_eq!(calls.len(), 1, "expected one git call");
        assert_eq!(calls[0].method, "mirror_clone");

        // Verify queue persisted with Done state.
        assert!(queue_path.exists(), "queue file should be written");
        let queue = starred_queue::load_or_create(&queue_path, "octocat", &[])
            .expect("load saved queue");
        assert_eq!(queue.items.len(), 1);
        assert_eq!(queue.items[0].state, CloneState::Done);
    }

    #[tokio::test]
    async fn second_run_skips_done_items() {
        use github_backup_types::Repository;
        use github_backup_types::starred_queue::StarredQueueItem;
        use github_backup_types::user::User;

        let repo = Repository {
            id: 999,
            full_name: "octocat/hello".to_string(),
            name: "hello".to_string(),
            owner: User {
                id: 1,
                login: "octocat".to_string(),
                user_type: "User".to_string(),
                avatar_url: String::new(),
                html_url: String::new(),
            },
            private: false,
            fork: false,
            archived: false,
            disabled: false,
            description: None,
            clone_url: "https://github.com/octocat/hello.git".to_string(),
            ssh_url: "git@github.com:octocat/hello.git".to_string(),
            default_branch: "main".to_string(),
            size: 10,
            has_issues: true,
            has_wiki: false,
            created_at: "2020-01-01T00:00:00Z".to_string(),
            pushed_at: None,
            updated_at: "2020-01-01T00:00:00Z".to_string(),
            html_url: "https://github.com/octocat/hello".to_string(),
        };

        let dir = TempDir::new().unwrap();
        let queue_path = dir.path().join("queue.json");

        // Pre-populate the queue with a Done entry for this repo.
        let mut pre_queue = github_backup_types::starred_queue::StarredCloneQueue {
            version: starred_queue::QUEUE_VERSION,
            owner: "octocat".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            items: vec![StarredQueueItem {
                id: 999,
                full_name: "octocat/hello".to_string(),
                clone_url: "https://github.com/octocat/hello.git".to_string(),
                ssh_url: "git@github.com:octocat/hello.git".to_string(),
                size_kb: 10,
                state: CloneState::Done,
                retries: 0,
                last_error: None,
                finished_at: Some("2026-01-01T00:00:00Z".to_string()),
            }],
        };
        starred_queue::save(&mut pre_queue, &queue_path).expect("pre-save");

        let client = MockBackupClient::new().with_starred(vec![repo]);
        let git = SpyGitRunner::default();

        backup_starred_repos(
            &client,
            &git,
            "octocat",
            &starred_opts(),
            dir.path(),
            &queue_path,
            &clone_opts(),
        )
        .await
        .expect("second run");

        assert_eq!(
            git.recorded_calls().len(),
            0,
            "Done item must not be re-cloned"
        );
    }

    #[test]
    fn compute_rate_eta_zero_when_no_elapsed() {
        let (rate, eta) = compute_rate_eta(0, std::time::Duration::from_secs(0), 100);
        assert_eq!(rate, 0.0);
        assert!(eta.is_none());
    }

    #[test]
    fn compute_rate_eta_reasonable_values() {
        // 10 repos in 60 seconds → 10 repo/min, 100 pending → ETA = 600s
        let (rate, eta) = compute_rate_eta(10, std::time::Duration::from_secs(60), 100);
        assert!((rate - 10.0).abs() < 0.01, "rate should be 10.0 repos/min");
        assert_eq!(eta, Some(600));
    }
}
