// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Git subprocess abstraction: clone, mirror, push, and fetch.
//!
//! The production implementation ([`ProcessGitRunner`]) shells out to the
//! system `git` binary.  Credentials for HTTPS cloning are injected via the
//! `GIT_ASKPASS` environment variable rather than being embedded in the URL,
//! which avoids leaking tokens in process listings and git reflog.
//!
//! # Hardening features
//!
//! - **Non-blocking** — the trait methods are `async`; [`ProcessGitRunner`]
//!   runs each git invocation on Tokio's blocking pool, so a clone that takes
//!   minutes never occupies an async worker thread and signal handling stays
//!   responsive.
//!
//! - **Stall timeout, not a wall-clock limit** — a git subprocess is stopped
//!   only when it has produced no output for
//!   `CloneOptions::stall_timeout_secs` (default 600 s).  git is run with
//!   `--progress`, so a huge repository that takes hours to clone is never
//!   interrupted while it is making progress, but a hung connection is.
//!
//! - **Prompt shutdown** — [`request_shutdown`] stops running git processes
//!   (and their transport helpers) within a fraction of a second.
//!
//! - **Partial clone cleanup** — if a fresh clone fails (destination did not
//!   exist before the attempt), any partially written directory is removed so
//!   the next run starts cleanly.
//!
//! - **Post-clone fsck** — after every *fresh* clone
//!   (`CloneOptions::run_fsck = true`) `git fsck --no-dangling` is run.
//!   Corruption is reported as [`CoreError::GitFsckFailed`] so callers can
//!   decide whether to abort or log and continue.
//!
//! # Sub-modules
//!
//! - `askpass` — RAII guard that writes and cleans up the `GIT_ASKPASS` script
//! - `process` — spawns and supervises one git subprocess
//! - `spy` — test-only `SpyGitRunner` stub (available under `test_support` in tests)

mod askpass;
mod process;
pub mod spy;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Command;

use tracing::{debug, info, warn};

use crate::error::CoreError;
use process::run_git;

pub use process::{request_shutdown, shutdown_requested};

// ── Public test-support re-export ─────────────────────────────────────────────

/// Test-support module: re-exported from [`spy`] for use by sibling tests.
///
/// Importing this module from outside the crate requires `#[cfg(test)]`
/// guards; the symbols are intentionally only `pub(crate)` at runtime.
#[cfg(test)]
pub mod test_support {
    pub use super::spy::{GitCall, SpyGitRunner};
}

// ── Types ─────────────────────────────────────────────────────────────────────

/// Default stall limit: a git process that prints nothing for 10 minutes is
/// considered hung.
const DEFAULT_STALL_TIMEOUT_SECS: u64 = 600;

/// Git clone options passed to the runner.
#[derive(Debug, Clone)]
pub struct CloneOptions {
    /// Token to inject for HTTPS authentication, or `None` for unauthenticated
    /// (public repos) or SSH-based cloning.
    pub token: Option<String>,
    /// When `true`, skip `--prune` during updates.
    pub no_prune: bool,
    /// Seconds a git subprocess may go without producing any output before it
    /// is killed and [`CoreError::GitTimeout`] is returned.
    ///
    /// This is a *stall* limit, not a total time limit: git runs with
    /// `--progress`, so a long transfer that keeps reporting progress is never
    /// interrupted.  Defaults to 600 s.
    pub stall_timeout_secs: u64,
    /// When `true`, run `git fsck --no-dangling` after every *fresh* clone to
    /// detect repository corruption early.
    pub run_fsck: bool,
}

impl CloneOptions {
    /// No authentication, prune enabled, default stall limit, fsck disabled.
    #[must_use]
    pub fn unauthenticated() -> Self {
        Self {
            token: None,
            no_prune: false,
            stall_timeout_secs: DEFAULT_STALL_TIMEOUT_SECS,
            run_fsck: false,
        }
    }
}

impl Default for CloneOptions {
    fn default() -> Self {
        Self::unauthenticated()
    }
}

// ── Trait ─────────────────────────────────────────────────────────────────────

/// Abstraction over git subprocess operations.
///
/// The production implementation ([`ProcessGitRunner`]) shells out to the
/// system `git` binary.  A no-op stub can be substituted during unit tests to
/// avoid network and filesystem side-effects.
///
/// All clone methods follow a common pattern:
/// - If `dest` already exists, update it in-place.
/// - If `dest` does not exist, perform a fresh clone.
///
/// For HTTPS URLs, `opts.token` is injected via a temporary `GIT_ASKPASS`
/// script that is removed by a RAII guard after the git process exits.
///
/// The methods return `Send` futures so they can be awaited inside spawned
/// Tokio tasks.
pub trait GitRunner: Send + Sync {
    /// Clones `url` into `dest` as a bare mirror (`git clone --mirror`).
    ///
    /// If `dest` already exists, updates it with `git fetch --all` (pruning
    /// deleted refs unless `opts.no_prune` is set).
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::GitFailed`] if git exits non-zero, or
    /// [`CoreError::GitSpawn`] if the binary cannot be started.
    fn mirror_clone(
        &self,
        url: &str,
        dest: &Path,
        opts: &CloneOptions,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    /// Clones `url` into `dest` as a bare clone (`git clone --bare`).
    ///
    /// Similar to `mirror_clone` but does not configure remote-tracking refs.
    /// If `dest` already exists, updates refs with `git fetch --all`.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::GitFailed`] or [`CoreError::GitSpawn`].
    fn bare_clone(
        &self,
        url: &str,
        dest: &Path,
        opts: &CloneOptions,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    /// Clones `url` into `dest` as a full working-tree clone.
    ///
    /// Use when you need to browse or build the backed-up source code.
    /// If `dest` already exists, updates with `git fetch --all --prune`.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::GitFailed`] or [`CoreError::GitSpawn`].
    fn full_clone(
        &self,
        url: &str,
        dest: &Path,
        opts: &CloneOptions,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    /// Clones `url` into `dest` as a shallow clone with limited history.
    ///
    /// Creates a bare-style repository with at most `depth` commits per
    /// branch.  Reduces disk usage significantly but loses older history.
    /// If `dest` already exists, deepens the clone with `git fetch --depth`.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::GitFailed`] or [`CoreError::GitSpawn`].
    fn shallow_clone(
        &self,
        url: &str,
        dest: &Path,
        opts: &CloneOptions,
        depth: u32,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    /// Clones `url` into `dest` using Git LFS.
    ///
    /// Fetches LFS objects in addition to regular git objects.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::GitFailed`] or [`CoreError::GitSpawn`].
    fn lfs_clone(
        &self,
        url: &str,
        dest: &Path,
        opts: &CloneOptions,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;

    /// Pushes all refs from the local repository at `src` to `remote_url`.
    ///
    /// Equivalent to `git -C <src> push --mirror <remote_url>`.  Used to push
    /// a local bare/mirror clone to a secondary Git host (Gitea, Codeberg,
    /// GitLab, etc.) after the primary backup has completed.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::GitFailed`] or [`CoreError::GitSpawn`].
    fn push_mirror(
        &self,
        src: &Path,
        remote_url: &str,
        opts: &CloneOptions,
    ) -> impl Future<Output = Result<(), CoreError>> + Send;
}

// ── Production implementation ─────────────────────────────────────────────────

/// Production [`GitRunner`] that shells out to the system `git` binary.
#[derive(Debug, Clone)]
pub struct ProcessGitRunner {
    /// The program to execute; `git` (resolved through `PATH`) by default.
    program: PathBuf,
}

impl Default for ProcessGitRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessGitRunner {
    /// Creates a runner that executes the `git` found on `PATH`.
    #[must_use]
    pub fn new() -> Self {
        Self::with_program("git")
    }

    /// Creates a runner that executes `program` instead of `git`.
    ///
    /// Intended for tests, which substitute a small script so process
    /// handling can be exercised without a real remote.
    #[must_use]
    pub fn with_program(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// Captures everything a blocking git job needs, by value.
    fn job(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Job {
        Job {
            program: self.program.clone(),
            url: url.to_owned(),
            dest: dest.to_owned(),
            opts: opts.clone(),
        }
    }
}

/// Runs `work` on Tokio's blocking pool so it never stalls an async worker.
async fn blocking<F>(work: F) -> Result<(), CoreError>
where
    F: FnOnce() -> Result<(), CoreError> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|join_err| Err(CoreError::TaskFailed(join_err.to_string())))
}

impl GitRunner for ProcessGitRunner {
    fn mirror_clone(
        &self,
        url: &str,
        dest: &Path,
        opts: &CloneOptions,
    ) -> impl Future<Output = Result<(), CoreError>> + Send {
        let job = self.job(url, dest, opts);
        async move { blocking(move || job.mirror_clone()).await }
    }

    fn bare_clone(
        &self,
        url: &str,
        dest: &Path,
        opts: &CloneOptions,
    ) -> impl Future<Output = Result<(), CoreError>> + Send {
        let job = self.job(url, dest, opts);
        async move { blocking(move || job.bare_clone()).await }
    }

    fn full_clone(
        &self,
        url: &str,
        dest: &Path,
        opts: &CloneOptions,
    ) -> impl Future<Output = Result<(), CoreError>> + Send {
        let job = self.job(url, dest, opts);
        async move { blocking(move || job.full_clone()).await }
    }

    fn shallow_clone(
        &self,
        url: &str,
        dest: &Path,
        opts: &CloneOptions,
        depth: u32,
    ) -> impl Future<Output = Result<(), CoreError>> + Send {
        let job = self.job(url, dest, opts);
        async move { blocking(move || job.shallow_clone(depth)).await }
    }

    fn lfs_clone(
        &self,
        url: &str,
        dest: &Path,
        opts: &CloneOptions,
    ) -> impl Future<Output = Result<(), CoreError>> + Send {
        let job = self.job(url, dest, opts);
        async move { blocking(move || job.lfs_clone()).await }
    }

    fn push_mirror(
        &self,
        src: &Path,
        remote_url: &str,
        opts: &CloneOptions,
    ) -> impl Future<Output = Result<(), CoreError>> + Send {
        // For a push, `dest` is the local repository and `url` the remote.
        let job = self.job(remote_url, src, opts);
        async move { blocking(move || job.push_mirror()).await }
    }
}

/// One owned, blocking git operation.  Holds no borrows so it can move onto
/// the blocking pool.
struct Job {
    program: PathBuf,
    url: String,
    dest: PathBuf,
    opts: CloneOptions,
}

impl Job {
    fn mirror_clone(&self) -> Result<(), CoreError> {
        let dest_str = path_to_str(&self.dest)?;
        if self.dest.exists() {
            info!(dest = %self.dest.display(), "repository exists, updating mirror");
            self.update(self.fetch_all_args())
        } else {
            info!(url = %self.url, dest = %self.dest.display(), "cloning bare mirror");
            self.fresh_clone(&["clone", "--progress", "--mirror", &self.url, dest_str])
        }
    }

    fn bare_clone(&self) -> Result<(), CoreError> {
        let dest_str = path_to_str(&self.dest)?;
        if self.dest.exists() {
            info!(dest = %self.dest.display(), "bare repository exists, fetching");
            self.update(self.fetch_all_args())
        } else {
            info!(url = %self.url, dest = %self.dest.display(), "cloning bare");
            self.fresh_clone(&["clone", "--progress", "--bare", &self.url, dest_str])
        }
    }

    fn full_clone(&self) -> Result<(), CoreError> {
        let dest_str = path_to_str(&self.dest)?;
        if self.dest.exists() {
            info!(dest = %self.dest.display(), "full clone exists, fetching all branches");
            self.update(self.fetch_all_args())
        } else {
            info!(url = %self.url, dest = %self.dest.display(), "cloning full working tree");
            self.fresh_clone(&["clone", "--progress", "--no-local", &self.url, dest_str])
        }
    }

    fn shallow_clone(&self, depth: u32) -> Result<(), CoreError> {
        let dest_str = path_to_str(&self.dest)?;
        let depth_str = depth.to_string();
        if self.dest.exists() {
            info!(dest = %self.dest.display(), depth, "shallow clone exists, deepening fetch");
            self.update(&["fetch", "--progress", "--depth", &depth_str])
        } else {
            info!(url = %self.url, dest = %self.dest.display(), depth, "cloning shallow");
            self.fresh_clone(&[
                "clone",
                "--progress",
                "--mirror",
                "--depth",
                &depth_str,
                &self.url,
                dest_str,
            ])
        }
    }

    fn lfs_clone(&self) -> Result<(), CoreError> {
        let dest_str = path_to_str(&self.dest)?;
        if self.dest.exists() {
            info!(dest = %self.dest.display(), "LFS repository exists, updating");
            self.update(&["lfs", "fetch", "--all"])
        } else {
            info!(url = %self.url, dest = %self.dest.display(), "cloning with LFS");
            self.fresh_clone(&["lfs", "clone", &self.url, dest_str])
        }
    }

    fn push_mirror(&self) -> Result<(), CoreError> {
        info!(
            src = %self.dest.display(),
            remote = %self.url,
            "pushing mirror to remote"
        );
        run_git(
            &self.program,
            &["push", "--progress", "--mirror", &self.url],
            &self.dest,
            self.opts.token.as_deref(),
            &self.opts,
        )
    }

    /// Arguments that bring an existing clone up to date with its remote.
    ///
    /// `git fetch --all` is used rather than `git remote update` because it
    /// accepts `--progress`; on a mirror clone the two yield identical refs
    /// (including force-pushed branches and deleted tags with `--prune`).
    fn fetch_all_args(&self) -> &'static [&'static str] {
        if self.opts.no_prune {
            &["fetch", "--progress", "--all"]
        } else {
            &["fetch", "--progress", "--all", "--prune"]
        }
    }

    /// Runs an update command inside the existing repository.
    fn update(&self, args: &[&str]) -> Result<(), CoreError> {
        run_git(
            &self.program,
            args,
            &self.dest,
            self.opts.token.as_deref(),
            &self.opts,
        )
    }

    /// Performs a **fresh** clone: runs git with `args`, then:
    /// - On failure: removes the partially-written destination directory.
    /// - On success (when `opts.run_fsck`): runs `git fsck` and logs issues.
    fn fresh_clone(&self, args: &[&str]) -> Result<(), CoreError> {
        match run_git(
            &self.program,
            args,
            Path::new("."),
            self.opts.token.as_deref(),
            &self.opts,
        ) {
            Ok(()) => {
                if self.opts.run_fsck && self.dest.exists() {
                    run_fsck(&self.program, &self.dest);
                }
                Ok(())
            }
            Err(e) => {
                // Remove any partial clone directory so the next run starts fresh.
                if self.dest.exists() {
                    if let Err(rm_err) = std::fs::remove_dir_all(&self.dest) {
                        warn!(
                            dest = %self.dest.display(),
                            error = %rm_err,
                            "failed to remove partial clone directory after git failure"
                        );
                    } else {
                        debug!(dest = %self.dest.display(), "removed partial clone directory");
                    }
                }
                Err(e)
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Converts a [`Path`] to a `&str`, returning a [`CoreError`] if the path
/// contains non-UTF-8 bytes.
fn path_to_str(path: &Path) -> Result<&str, CoreError> {
    path.to_str().ok_or_else(|| CoreError::NonUtf8Path {
        path: path.to_string_lossy().into_owned(),
    })
}

/// Runs `git fsck --no-dangling` on `repo_dir` and logs any issues found.
///
/// Corruption is not treated as a fatal error — the backup has already
/// completed — but the issues are logged at `warn` level so operators can
/// investigate.
fn run_fsck(program: &Path, repo_dir: &Path) {
    debug!(repo = %repo_dir.display(), "running git fsck");
    let output = Command::new(program)
        .args(["fsck", "--no-dangling"])
        .current_dir(repo_dir)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            debug!(repo = %repo_dir.display(), "git fsck: clean");
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let stdout = String::from_utf8_lossy(&o.stdout);
            let combined = format!("{stdout}{stderr}");
            warn!(
                repo = %repo_dir.display(),
                output = %combined.chars().take(512).collect::<String>(),
                "git fsck reported issues after clone"
            );
        }
        Err(e) => {
            warn!(
                repo = %repo_dir.display(),
                error = %e,
                "could not run git fsck (git binary issue?)"
            );
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::spy::SpyGitRunner;
    use super::*;
    use std::time::{Duration, Instant};

    fn opts() -> CloneOptions {
        CloneOptions::unauthenticated()
    }

    #[test]
    fn path_to_str_returns_error_for_non_utf8_path() {
        #[cfg(unix)]
        {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt;
            let invalid = OsStr::from_bytes(b"/tmp/invalid\xff");
            let path = std::path::Path::new(invalid);
            assert!(path_to_str(path).is_err());
        }
    }

    #[test]
    fn path_to_str_returns_str_for_valid_utf8() {
        let path = Path::new("/tmp/valid-path.git");
        assert_eq!(
            path_to_str(path).expect("valid path"),
            "/tmp/valid-path.git"
        );
    }

    #[test]
    fn clone_options_unauthenticated_has_no_token() {
        let opts = CloneOptions::unauthenticated();
        assert!(opts.token.is_none());
        assert!(!opts.no_prune);
        assert_eq!(opts.stall_timeout_secs, DEFAULT_STALL_TIMEOUT_SECS);
        assert!(!opts.run_fsck);
    }

    #[test]
    fn clone_options_default_matches_unauthenticated() {
        let a = CloneOptions::default();
        let b = CloneOptions::unauthenticated();
        assert_eq!(a.stall_timeout_secs, b.stall_timeout_secs);
        assert_eq!(a.run_fsck, b.run_fsck);
        assert_eq!(a.no_prune, b.no_prune);
    }

    #[tokio::test]
    async fn spy_runner_mirror_clone() {
        let runner = SpyGitRunner::default();
        let dest = PathBuf::from("/tmp/test.git");
        runner
            .mirror_clone("https://github.com/octocat/Hello-World.git", &dest, &opts())
            .await
            .expect("mirror clone");
        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "mirror_clone");
    }

    #[tokio::test]
    async fn spy_runner_push_mirror() {
        let runner = SpyGitRunner::default();
        let src = PathBuf::from("/tmp/local.git");
        runner
            .push_mirror(&src, "https://gitea.example.com/user/repo.git", &opts())
            .await
            .expect("push mirror");
        let calls = runner.recorded_calls();
        assert_eq!(calls[0].method, "push_mirror");
    }

    // ── Process handling (stand-in `git` scripts, Unix only) ─────────────
    //
    // Process-wide shutdown (`request_shutdown`) is covered by the
    // integration test `tests/git_shutdown.rs`, which runs in its own process
    // so the one-way flag cannot leak into these tests.

    /// Writes an executable shell script into `dir` and returns a runner that
    /// executes it instead of the real `git`.
    #[cfg(unix)]
    fn runner_with_script(dir: &Path, body: &str) -> ProcessGitRunner {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("fake-git");
        std::fs::write(&script, format!("#!/bin/sh\n{body}\n")).expect("write script");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");
        ProcessGitRunner::with_program(script)
    }

    #[cfg(unix)]
    fn with_stall(secs: u64) -> CloneOptions {
        CloneOptions {
            stall_timeout_secs: secs,
            ..CloneOptions::unauthenticated()
        }
    }

    /// Regression: stdout/stderr were piped but only read after the child
    /// exited, so a child writing more than the pipe buffer (64 KiB on Linux)
    /// blocked forever and was killed by the timeout.  `git fetch` prints one
    /// line per updated ref, so a repository with thousands of new refs
    /// (`refs/pull/*`) hit this in practice.
    #[cfg(unix)]
    #[tokio::test]
    async fn large_stderr_output_does_not_deadlock() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runner = runner_with_script(
            dir.path(),
            r"head -c 1048576 /dev/zero | tr '\0' 'x' >&2; exit 0",
        );
        let dest = dir.path().join("repo.git");
        let started = Instant::now();
        let result = runner
            .mirror_clone("https://example.invalid/r.git", &dest, &with_stall(10))
            .await;
        assert!(result.is_ok(), "1 MiB of stderr must not fail: {result:?}");
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "took {:?}: the child was stuck on a full pipe until the stall limit",
            started.elapsed()
        );
    }

    /// The error text must come from the END of stderr (where git prints the
    /// `fatal:` line), not be lost behind a flood of earlier output.
    #[cfg(unix)]
    #[tokio::test]
    async fn failure_reports_the_tail_of_a_long_stderr() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runner = runner_with_script(
            dir.path(),
            r#"head -c 500000 /dev/zero | tr '\0' 'x' >&2
echo >&2
echo "fatal: repository 'https://example.invalid/r.git/' not found" >&2
exit 128"#,
        );
        let dest = dir.path().join("repo.git");
        let err = runner
            .mirror_clone("https://example.invalid/r.git", &dest, &with_stall(10))
            .await
            .expect_err("must fail");
        match err {
            CoreError::GitFailed { code, stderr, .. } => {
                assert_eq!(code, 128);
                assert!(
                    stderr.contains("not found"),
                    "tail lost: {} bytes",
                    stderr.len()
                );
                assert!(stderr.len() <= 64 * 1024, "stderr must be bounded");
            }
            other => panic!("expected GitFailed, got {other:?}"),
        }
    }

    /// A child that produces no output is a hung connection: it is stopped
    /// after the stall limit.
    #[cfg(unix)]
    #[tokio::test]
    async fn silent_child_is_stopped_after_the_stall_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runner = runner_with_script(dir.path(), "exec sleep 30");
        let dest = dir.path().join("repo.git");
        let started = Instant::now();
        let err = runner
            .mirror_clone("https://example.invalid/r.git", &dest, &with_stall(1))
            .await
            .expect_err("a silent child must be stopped");
        assert!(
            matches!(
                err,
                CoreError::GitTimeout {
                    timeout_secs: 1,
                    ..
                }
            ),
            "{err:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
    }

    /// The limit is a *stall* limit: a slow child that keeps reporting
    /// progress must never be interrupted, however long it runs.  (The old
    /// 600 s wall-clock limit made repositories that take longer than ten
    /// minutes to clone impossible to back up.)
    #[cfg(unix)]
    #[tokio::test]
    async fn chatty_child_is_not_stopped_by_the_stall_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runner = runner_with_script(
            dir.path(),
            r#"i=0
while [ $i -lt 8 ]; do
  echo "Receiving objects: $((i * 12))% ($i/8)" >&2
  sleep 0.5
  i=$((i + 1))
done
exit 0"#,
        );
        let dest = dir.path().join("repo.git");
        let started = Instant::now();
        let result = runner
            .mirror_clone("https://example.invalid/r.git", &dest, &with_stall(2))
            .await;
        assert!(
            result.is_ok(),
            "progress must keep the child alive: {result:?}"
        );
        assert!(
            started.elapsed() > Duration::from_secs(3),
            "the script runs for ~4 s, longer than the 2 s stall limit"
        );
    }

    /// A failed fresh clone must not leave a half-written directory behind.
    #[cfg(unix)]
    #[tokio::test]
    async fn failed_fresh_clone_removes_the_partial_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runner = runner_with_script(
            dir.path(),
            r#"for last in "$@"; do :; done
mkdir -p "$last/objects"
echo "fatal: early EOF" >&2
exit 128"#,
        );
        let dest = dir.path().join("repo.git");
        runner
            .mirror_clone("https://example.invalid/r.git", &dest, &with_stall(10))
            .await
            .expect_err("must fail");
        assert!(!dest.exists(), "partial clone directory must be removed");
    }

    /// git must never be able to prompt (cron/Docker have no terminal) and
    /// must emit stable English messages.
    #[cfg(unix)]
    #[tokio::test]
    async fn git_gets_no_terminal_and_a_stable_locale() {
        let dir = tempfile::tempdir().expect("tempdir");
        let seen = dir.path().join("seen.txt");
        let runner = runner_with_script(
            dir.path(),
            &format!(
                r#"{{ echo "prompt=$GIT_TERMINAL_PROMPT"; echo "lc=$LC_ALL"; \
if [ -t 0 ]; then echo stdin=tty; else echo stdin=none; fi; }} > '{}'"#,
                seen.display()
            ),
        );
        let dest = dir.path().join("repo.git");
        runner
            .mirror_clone("https://example.invalid/r.git", &dest, &with_stall(10))
            .await
            .expect("ok");
        let text = std::fs::read_to_string(&seen).expect("script output");
        assert!(text.contains("prompt=0"), "{text}");
        assert!(text.contains("lc=C"), "{text}");
        assert!(text.contains("stdin=none"), "{text}");
    }

    /// Network operations pass `--progress` so a stall can be told apart from
    /// a slow transfer even when stderr is not a terminal.
    #[cfg(unix)]
    #[tokio::test]
    async fn network_operations_request_progress() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = dir.path().join("argv.txt");
        let runner =
            runner_with_script(dir.path(), &format!(r#"echo "$@" >> '{}'"#, log.display()));
        let fresh = dir.path().join("fresh.git");
        runner
            .mirror_clone("https://example.invalid/r.git", &fresh, &with_stall(10))
            .await
            .expect("fresh clone");
        let existing = dir.path().join("existing.git");
        std::fs::create_dir(&existing).expect("mkdir");
        runner
            .mirror_clone("https://example.invalid/r.git", &existing, &with_stall(10))
            .await
            .expect("update");
        let argv = std::fs::read_to_string(&log).expect("argv log");
        let lines: Vec<&str> = argv.lines().collect();
        assert_eq!(lines.len(), 2, "{argv}");
        assert!(lines[0].starts_with("clone --progress --mirror "), "{argv}");
        assert_eq!(lines[1], "fetch --progress --all --prune", "{argv}");
    }
}
