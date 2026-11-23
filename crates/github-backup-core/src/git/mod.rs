// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Git subprocess abstraction: clone and update repositories.
//!
//! The production implementation ([`ProcessGitRunner`]) shells out to the
//! system `git` binary.  Credentials for HTTPS cloning are handed to git
//! through the environment of the child process and an inline, host-scoped
//! credential helper (see `credential`): never in the URL, in argv or in a file,
//! and never offered to any host other than the one being cloned.
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
//! - **Prompt cancellation** — triggering [`CloneOptions::cancel`] stops running
//!   git processes (and their transport helpers) within a fraction of a second.
//!
//! - **Atomic clones** — a fresh clone is made in a hidden staging directory
//!   next to the destination and renamed into place only when git finished, so
//!   the destination is either absent or a complete repository.  A run killed
//!   mid-clone leaves only a staging directory, which the next run deletes.
//!
//! - **Optional verification** — with `CloneOptions::run_fsck = true` (off by
//!   default) `git fsck --no-dangling` checks every repository after it was
//!   cloned or updated; a repository that fails is reported as
//!   [`CoreError::GitFsckFailed`], and a fresh clone that fails is not moved
//!   into place.
//!
//! # Sub-modules
//!
//! - `credential` — the host-scoped credential helper and its configuration
//! - `process` — spawns and supervises one git subprocess
//! - `spy` — the `SpyGitRunner` stub used by the crate's tests (`cfg(test)` only)

mod credential;
mod process;
#[cfg(test)]
pub(crate) mod spy;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use tracing::{debug, info, warn};

use crate::cancel::CancelFlag;
use crate::error::CoreError;
use process::run_git;

// ── Public test-support re-export ─────────────────────────────────────────────

/// The `SpyGitRunner` stub, for the crate's own tests.
#[cfg(test)]
pub(crate) mod test_support {
    pub use super::spy::SpyGitRunner;
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
    /// When `true`, updates pass `--prune`, so branches and tags deleted on the
    /// remote are deleted from the local clone too.  The default (`false`)
    /// keeps them: a backup should still hold what GitHub no longer has.
    /// Force-pushed branches are overwritten either way.
    pub prune: bool,
    /// Seconds a git subprocess may go without producing any output before it
    /// is killed and [`CoreError::GitTimeout`] is returned.
    ///
    /// This is a *stall* limit, not a total time limit: git runs with
    /// `--progress`, so a long transfer that keeps reporting progress is never
    /// interrupted.  Defaults to 600 s.
    pub stall_timeout_secs: u64,
    /// When `true`, run `git fsck --no-dangling` after every clone and update;
    /// a repository that fails the check is reported as
    /// [`CoreError::GitFsckFailed`] (a fresh clone that fails is discarded).
    pub run_fsck: bool,
    /// Triggering this stops running git processes promptly and makes new ones
    /// fail with [`CoreError::Interrupted`].
    pub cancel: CancelFlag,
}

impl CloneOptions {
    /// No authentication, pruning off, default stall limit, fsck disabled.
    #[must_use]
    pub fn unauthenticated() -> Self {
        Self {
            token: None,
            prune: false,
            stall_timeout_secs: DEFAULT_STALL_TIMEOUT_SECS,
            run_fsck: false,
            cancel: CancelFlag::new(),
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
/// Updates prune refs deleted upstream only when `opts.prune` is set; the
/// choice is always passed to git explicitly (`--prune`, or `--no-prune
/// --no-prune-tags`), so pruning settings in the user's git configuration do
/// not apply.
///
/// For HTTPS URLs, `opts.token` is passed through the git child's environment
/// and a credential helper scoped to the URL's host (see `credential`).
///
/// The methods return `Send` futures so they can be awaited inside spawned
/// Tokio tasks.
pub trait GitRunner: Send + Sync {
    /// Clones `url` into `dest` as a bare mirror (`git clone --mirror`).
    ///
    /// If `dest` already exists, updates every ref with `git fetch --all`.
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
    /// Holds branches and tags only (no other refs).  If `dest` already
    /// exists, fetches `refs/heads/*` and `refs/tags/*` into the same names.
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
    /// If `dest` already exists, every local branch, remote-tracking branch
    /// and tag is set to its upstream position and the working tree is reset
    /// (`reset --hard`) to the checked-out branch: the clone is the tool's
    /// own, local edits in it are not kept.
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
    /// Creates a mirror-style repository of every branch and tag with at most
    /// `depth` commits each.  Reduces disk usage significantly but loses older
    /// history.  If `dest` already exists, fetches what is new with
    /// `git fetch --depth <depth>`, which keeps the history cut at `depth`.
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
    /// A mirror clone (see [`mirror_clone`](Self::mirror_clone)) followed by
    /// `git lfs fetch --all`, which adds the LFS objects of every ref.
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
}

// ── Production implementation ─────────────────────────────────────────────────

/// Production [`GitRunner`] that shells out to the system `git` binary.
#[derive(Debug, Clone)]
pub struct ProcessGitRunner {
    git: GitCommand,
}

/// What is executed in place of `git`: a program, and arguments placed before
/// git's own (empty except in tests, which run a stand-in script through
/// `sh`).
#[derive(Debug, Clone)]
pub(crate) struct GitCommand {
    program: PathBuf,
    leading_args: Vec<PathBuf>,
}

impl GitCommand {
    /// A [`Command`] for this program, with its leading arguments.
    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.leading_args);
        cmd
    }
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
            git: GitCommand {
                program: program.into(),
                leading_args: Vec::new(),
            },
        }
    }

    /// Creates a runner that runs the shell script `script` (through `sh`)
    /// instead of `git`.
    ///
    /// Executing a script the test has just written races with other tests
    /// forking at the same moment: a forked child briefly holds the still-open
    /// file descriptor, and `exec` then fails with "Text file busy".  Reading
    /// the script with `sh` needs no `exec` of the file at all.
    #[cfg(all(test, unix))]
    pub(crate) fn with_shell_script(script: impl Into<PathBuf>) -> Self {
        Self {
            git: GitCommand {
                program: PathBuf::from("sh"),
                leading_args: vec![script.into()],
            },
        }
    }

    /// Captures everything a blocking git job needs, by value.
    fn job(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Job {
        Job {
            git: self.git.clone(),
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
}

/// One owned, blocking git operation.  Holds no borrows so it can move onto
/// the blocking pool.
struct Job {
    git: GitCommand,
    url: String,
    dest: PathBuf,
    opts: CloneOptions,
}

impl Job {
    fn mirror_clone(&self) -> Result<(), CoreError> {
        path_to_str(&self.dest)?;
        if self.dest.exists() {
            info!(dest = %self.dest.display(), "repository exists, updating mirror");
            // `git fetch --all` rather than `git remote update` because it
            // accepts `--progress`; the mirror refspec (`+refs/*:refs/*`)
            // updates every ref, force-pushed branches and moved tags included.
            self.fetch(&["--all"])?;
            self.verify(&self.dest)
        } else {
            info!(url = %self.url, dest = %self.dest.display(), "cloning bare mirror");
            self.fresh_clone(&["clone", "--progress", "--mirror", &self.url, DEST])
        }
    }

    fn bare_clone(&self) -> Result<(), CoreError> {
        path_to_str(&self.dest)?;
        if self.dest.exists() {
            info!(dest = %self.dest.display(), "bare repository exists, fetching");
            // `git clone --bare` writes no fetch refspec, so `git fetch --all`
            // would exit 0 having fetched nothing, forever.  Name the refspecs.
            self.fetch(&[
                "origin",
                "+refs/heads/*:refs/heads/*",
                "+refs/tags/*:refs/tags/*",
            ])?;
            self.verify(&self.dest)
        } else {
            info!(url = %self.url, dest = %self.dest.display(), "cloning bare");
            self.fresh_clone(&["clone", "--progress", "--bare", &self.url, DEST])
        }
    }

    /// A working-tree clone.  Its update brings every local branch, every
    /// remote-tracking branch and every tag to its upstream position (the
    /// clone is ours: nobody is expected to commit in it) and then the working
    /// tree to the checked-out branch.
    fn full_clone(&self) -> Result<(), CoreError> {
        path_to_str(&self.dest)?;
        if self.dest.exists() {
            info!(dest = %self.dest.display(), "full clone exists, updating branches, tags and checkout");
            // `--update-head-ok` lets the fetch move the checked-out branch too.
            self.fetch(&[
                "--update-head-ok",
                "origin",
                "+refs/heads/*:refs/heads/*",
                "+refs/heads/*:refs/remotes/origin/*",
                "+refs/tags/*:refs/tags/*",
            ])?;
            self.check_out_head()?;
            self.verify(&self.dest)
        } else {
            info!(url = %self.url, dest = %self.dest.display(), "cloning full working tree");
            self.fresh_clone(&["clone", "--progress", "--no-local", &self.url, DEST])
        }
    }

    fn shallow_clone(&self, depth: u32) -> Result<(), CoreError> {
        path_to_str(&self.dest)?;
        let depth_str = depth.to_string();
        if self.dest.exists() {
            // Fetches what is new, keeping the history cut at `depth` commits.
            info!(dest = %self.dest.display(), depth, "shallow clone exists, fetching");
            self.fetch(&["--depth", &depth_str])?;
            self.verify(&self.dest)
        } else {
            info!(url = %self.url, dest = %self.dest.display(), depth, "cloning shallow");
            self.fresh_clone(&[
                "clone",
                "--progress",
                "--mirror",
                // `--depth` implies `--single-branch`, which would leave every
                // branch but the default one out of the first backup.
                "--no-single-branch",
                "--depth",
                &depth_str,
                &self.url,
                DEST,
            ])
        }
    }

    /// LFS mode is a mirror plus the LFS objects.  (`git lfs clone` made a
    /// working-tree checkout whose refs `git lfs fetch --all` never advanced,
    /// so every update after the first silently did nothing.)
    fn lfs_clone(&self) -> Result<(), CoreError> {
        self.mirror_clone()?;
        info!(dest = %self.dest.display(), "fetching LFS objects");
        self.run_update(&["lfs", "fetch", "--all", "origin"])
    }

    /// Runs `git fetch --progress <prune flags> <rest>` in the existing
    /// repository.
    ///
    /// The prune flags are always explicit: `--prune` when pruning was asked
    /// for, `--no-prune --no-prune-tags` otherwise.  Leaving them out would let
    /// a `fetch.prune`, `remote.<name>.prune` or `fetch.pruneTags` setting in
    /// the user's or the system's git configuration delete refs that this
    /// tool promises to keep.
    ///
    /// Not pruning has one failure mode: when a branch that was deleted on the
    /// remote is replaced by one whose name needs its place as a directory
    /// (`foo` deleted, `foo/bar` created), git cannot create the new ref while
    /// the stale one exists.  Such a fetch is repeated once with `--prune`, so
    /// the repository keeps being backed up; it loses only the stale refs.
    fn fetch(&self, rest: &[&str]) -> Result<(), CoreError> {
        remove_stale_locks(&self.dest, self.stale_lock_age());
        let run = |prune: bool| {
            let mut args = vec!["fetch", "--progress"];
            if prune {
                args.push("--prune");
            } else {
                args.extend(["--no-prune", "--no-prune-tags"]);
            }
            args.extend_from_slice(rest);
            self.run_update(&args)
        };
        match run(self.opts.prune) {
            Err(CoreError::GitFailed { stderr, .. })
                if !self.opts.prune && is_ref_name_conflict(&stderr) =>
            {
                warn!(
                    dest = %self.dest.display(),
                    "a deleted branch blocks a new one of the same path; pruning deleted refs once to continue"
                );
                run(true)
            }
            other => other,
        }
    }

    /// How old a `.lock` file must be to count as left behind by a git that
    /// was killed.
    ///
    /// git holds a lock for the moment it takes to write one file.  The owner
    /// lock keeps other runs of this tool away from the repository, and any git
    /// this tool runs is stopped after the stall limit; a lock older than that
    /// limit therefore belongs to no live git of ours, and removing it is what
    /// lets the update proceed (git asks for exactly that).
    fn stale_lock_age(&self) -> Duration {
        Duration::from_secs(self.opts.stall_timeout_secs)
    }

    /// With [`CloneOptions::run_fsck`], checks the repository at `repo` (the
    /// destination, or the staging directory of a fresh clone) with `git fsck`.
    ///
    /// The check runs like any other git command: supervised for stalls
    /// (`--progress` keeps a long check alive), cancellable, with bounded
    /// output.  A failed check is [`CoreError::GitFsckFailed`].
    fn verify(&self, repo: &Path) -> Result<(), CoreError> {
        if !self.opts.run_fsck {
            return Ok(());
        }
        debug!(repo = %self.dest.display(), "checking the repository with git fsck");
        match run_git(
            &self.git,
            &["fsck", "--progress", "--no-dangling"],
            repo,
            "",
            None,
            &self.opts,
        ) {
            Err(CoreError::GitFailed { stderr, .. }) => Err(CoreError::GitFsckFailed {
                repo: self.dest.display().to_string(),
                output: stderr,
            }),
            other => other,
        }
    }

    /// Brings the working tree of a full clone to the commit its checked-out
    /// branch now points to.
    ///
    /// Nothing is done when that branch does not resolve: the repository is
    /// empty, or (with pruning) the branch was deleted upstream; the working
    /// tree is then left as it was.
    fn check_out_head(&self) -> Result<(), CoreError> {
        match self.run_update(&["rev-parse", "--verify", "--quiet", "HEAD"]) {
            Ok(()) => self.run_update(&["reset", "--hard", "--quiet", "HEAD"]),
            Err(CoreError::GitFailed { .. }) => {
                info!(
                    dest = %self.dest.display(),
                    "the checked-out branch has no commit (empty repository, or deleted upstream); working tree left as it was"
                );
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    fn run_update(&self, args: &[&str]) -> Result<(), CoreError> {
        run_git(
            &self.git,
            args,
            &self.dest,
            &self.url,
            self.opts.token.as_deref(),
            &self.opts,
        )
    }

    /// Performs a **fresh** clone atomically.
    ///
    /// git clones into a hidden staging directory next to the destination, and
    /// only a clone that finished is renamed into place.  The destination
    /// therefore always holds either nothing or a complete repository: a
    /// process killed mid-clone (OOM, `docker stop` timeout, power loss) leaves
    /// only a staging directory, which the next attempt deletes — never a
    /// half-initialised repository that later updates would treat as complete
    /// and "fetch" nothing into forever.
    ///
    /// `args` are git's arguments, with [`DEST`] standing for the staging
    /// directory.
    fn fresh_clone(&self, args: &[&str]) -> Result<(), CoreError> {
        let parent = self.dest.parent().unwrap_or_else(|| Path::new("."));
        let name = self
            .dest
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        std::fs::create_dir_all(parent).map_err(|e| CoreError::io(parent.display(), e))?;

        // Staging directories left by a killed predecessor (possibly still being
        // written to by its orphaned git) are of no use to anyone.
        remove_stale_staging(parent, &name);

        let staging = parent.join(format!(".{name}{STAGING_MARKER}{}", std::process::id()));
        let staging_str = path_to_str(&staging)?;
        let args: Vec<&str> = args
            .iter()
            .map(|a| if *a == DEST { staging_str } else { *a })
            .collect();

        let result = run_git(
            &self.git,
            &args,
            Path::new("."),
            &self.url,
            self.opts.token.as_deref(),
            &self.opts,
        )
        // A clone that fails its check is not moved into place.
        .and_then(|()| self.verify(&staging))
        .and_then(|()| {
            std::fs::rename(&staging, &self.dest).map_err(|e| CoreError::io(self.dest.display(), e))
        });

        if result.is_err() && staging.exists() {
            match std::fs::remove_dir_all(&staging) {
                Ok(()) => debug!(dir = %staging.display(), "removed partial clone"),
                Err(e) => warn!(
                    dir = %staging.display(),
                    error = %e,
                    "failed to remove partial clone directory after git failure"
                ),
            }
        }
        result
    }
}

/// Placeholder for the clone destination in the argument lists above; replaced
/// by the staging directory in [`Job::fresh_clone`].
const DEST: &str = "\0dest";

/// Infix of a staging directory's name: `.<repo>.git.partial-<pid>`.
const STAGING_MARKER: &str = ".partial-";

/// Removes the `.lock` files older than `min_age` from the git directory of
/// the repository at `repo` (`repo/.git` for a working-tree clone): those at
/// its top level (`HEAD.lock`, `packed-refs.lock`, `config.lock`,
/// `shallow.lock`, `index.lock`, ...) and those under `refs/`.
///
/// A git that is killed with `SIGKILL` (by the OOM killer, or with the whole
/// container) leaves its lock files behind, and every later update of the
/// repository then fails with "Unable to create '...lock': File exists".
fn remove_stale_locks(repo: &Path, min_age: Duration) {
    let dot_git = repo.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        repo.to_path_buf()
    };
    let mut dirs = vec![(git_dir.clone(), false), (git_dir.join("refs"), true)];
    while let Some((dir, recurse)) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                if recurse {
                    dirs.push((path, true));
                }
                continue;
            }
            let is_lock = path.extension().is_some_and(|ext| ext == "lock");
            let age = meta
                .modified()
                .ok()
                .and_then(|m| SystemTime::now().duration_since(m).ok());
            if is_lock && age.is_some_and(|age| age >= min_age) {
                match std::fs::remove_file(&path) {
                    Ok(()) => {
                        warn!(file = %path.display(), "removed a stale git lock file left by a killed git")
                    }
                    Err(e) => {
                        warn!(file = %path.display(), error = %e, "could not remove a stale git lock file")
                    }
                }
            }
        }
    }
}

/// Removes every `.<name>.partial-*` sibling in `parent`.
fn remove_stale_staging(parent: &Path, name: &str) {
    let prefix = format!(".{name}{STAGING_MARKER}");
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            match std::fs::remove_dir_all(entry.path()) {
                Ok(()) => info!(dir = %entry.path().display(), "removed leftover partial clone"),
                Err(e) => {
                    warn!(dir = %entry.path().display(), error = %e, "could not remove leftover partial clone")
                }
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

// ── Tests ─────────────────────────────────────────────────────────────────────

/// `true` if a failed fetch was blocked by a stale ref occupying the name a
/// new ref needs (a directory/file clash between `foo` and `foo/bar`).
///
/// Only that clash qualifies.  "cannot lock ref" alone also covers a lock
/// file left by a killed git ("Unable to create '...main.lock': File
/// exists"), which pruning cannot fix: it would delete kept refs and fail
/// again.
fn is_ref_name_conflict(stderr: &str) -> bool {
    stderr.lines().any(|line| {
        let line = line.to_ascii_lowercase();
        (line.contains("cannot lock ref")
            && (line.contains("' exists; cannot create '") || line.contains("blocking reference")))
            // git's own advice, printed for this clash only.
            || line.contains("'git remote prune")
    })
}

#[cfg(test)]
mod tests {
    use super::spy::SpyGitRunner;
    use super::*;
    use crate::cancel::CancelFlag;
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
        assert!(!opts.prune);
        assert_eq!(opts.stall_timeout_secs, DEFAULT_STALL_TIMEOUT_SECS);
        assert!(!opts.run_fsck);
    }

    #[test]
    fn clone_options_default_matches_unauthenticated() {
        let a = CloneOptions::default();
        let b = CloneOptions::unauthenticated();
        assert_eq!(a.stall_timeout_secs, b.stall_timeout_secs);
        assert_eq!(a.run_fsck, b.run_fsck);
        assert_eq!(a.prune, b.prune);
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

    // ── Process handling (stand-in `git` scripts, Unix only) ─────────────
    //
    // Cancellation (`CloneOptions::cancel`) is covered by the integration test
    // `tests/git_cancel.rs`.

    /// Writes a shell script into `dir` and returns a runner that runs it
    /// instead of the real `git`.
    #[cfg(unix)]
    fn runner_with_script(dir: &Path, body: &str) -> ProcessGitRunner {
        let script = dir.join("fake-git");
        // Like real git, a `clone` creates its destination (the last argument).
        let prelude = r#"if [ "$1" = clone ] || [ "$1" = lfs ]; then for d in "$@"; do :; done; mkdir -p "$d"; fi"#;
        std::fs::write(&script, format!("{prelude}\n{body}\n")).expect("write script");
        ProcessGitRunner::with_shell_script(script)
    }

    /// Drops the global `-c key=value` options git is run with, leaving the
    /// subcommand and its arguments.
    #[cfg(unix)]
    fn subcommand(line: &str) -> String {
        let mut rest = line;
        while let Some(r) = rest.strip_prefix("-c ") {
            rest = r.split_once(' ').map_or("", |(_, tail)| tail);
        }
        rest.to_owned()
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
        assert_eq!(
            staging_dirs(dir.path()),
            Vec::<String>::new(),
            "the staging directory must be removed too"
        );
    }

    #[cfg(unix)]
    fn staging_dirs(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .expect("read_dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(STAGING_MARKER))
            .collect()
    }

    /// A finished clone appears at the destination in one rename, with nothing
    /// left behind.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_successful_clone_is_renamed_into_place() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runner = runner_with_script(dir.path(), r#"touch "$d/HEAD"; exit 0"#);
        let dest = dir.path().join("sub").join("repo.git");
        runner
            .mirror_clone("https://example.invalid/r.git", &dest, &with_stall(10))
            .await
            .expect("clone");
        assert!(
            dest.join("HEAD").exists(),
            "content must be at the destination"
        );
        assert!(staging_dirs(dest.parent().unwrap()).is_empty());
    }

    /// Regression for the audit's critical finding: a process killed during
    /// the clone left a half-initialised repository at the destination, which
    /// the next run treated as complete and "updated" with `fetch` forever.
    /// Now the destination does not exist until the clone is finished, and a
    /// predecessor's staging directory is swept.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_killed_predecessors_partial_clone_is_swept_and_never_mistaken_for_a_repo() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("repo.git");
        // What a SIGKILLed run leaves: a staging directory, no destination.
        let leftover = dir.path().join(format!(".repo.git{STAGING_MARKER}4242"));
        std::fs::create_dir_all(leftover.join("objects")).expect("leftover");
        assert!(!dest.exists());

        let runner = runner_with_script(dir.path(), r#"touch "$d/HEAD"; exit 0"#);
        runner
            .mirror_clone("https://example.invalid/r.git", &dest, &with_stall(10))
            .await
            .expect("clone");

        assert!(dest.join("HEAD").exists());
        assert!(
            !leftover.exists(),
            "the stale staging directory must be removed"
        );
    }

    /// `git clone --bare` writes no fetch refspec, so updating with
    /// `fetch --all` fetched nothing, forever.  The update must name its refspecs.
    #[cfg(unix)]
    #[tokio::test]
    async fn updating_a_bare_clone_names_its_refspecs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let seen = dir.path().join("args.txt");
        let runner =
            runner_with_script(dir.path(), &format!(r#"echo "$@" > '{}'"#, seen.display()));
        let dest = dir.path().join("repo.git");
        std::fs::create_dir_all(&dest).expect("existing repo");
        runner
            .bare_clone("https://example.invalid/r.git", &dest, &with_stall(10))
            .await
            .expect("update");
        let args = std::fs::read_to_string(&seen).expect("args");
        let words: Vec<&str> = args.split_whitespace().collect();
        assert!(words.contains(&"+refs/heads/*:refs/heads/*"), "{args}");
        assert!(words.contains(&"+refs/tags/*:refs/tags/*"), "{args}");
        assert!(!words.contains(&"--prune"), "pruning is opt-in: {args}");
        assert!(words.contains(&"--no-prune"), "{args}");

        // With pruning requested the same update passes `--prune`.
        let opts = CloneOptions {
            prune: true,
            ..with_stall(10)
        };
        runner
            .bare_clone("https://example.invalid/r.git", &dest, &opts)
            .await
            .expect("update");
        let args = std::fs::read_to_string(&seen).expect("args");
        let words: Vec<&str> = args.split_whitespace().collect();
        assert!(words.contains(&"--prune"), "{args}");
        assert!(!words.contains(&"--no-prune"), "{args}");
    }

    /// LFS mode is a mirror (so refs advance on every run) plus the LFS objects.
    #[cfg(unix)]
    #[tokio::test]
    async fn lfs_update_fetches_refs_and_then_lfs_objects() {
        let dir = tempfile::tempdir().expect("tempdir");
        let seen = dir.path().join("calls.txt");
        let runner =
            runner_with_script(dir.path(), &format!(r#"echo "$@" >> '{}'"#, seen.display()));
        let dest = dir.path().join("repo.git");
        std::fs::create_dir_all(&dest).expect("existing repo");
        runner
            .lfs_clone("https://example.invalid/r.git", &dest, &with_stall(10))
            .await
            .expect("update");
        let calls = std::fs::read_to_string(&seen).expect("calls");
        let lines: Vec<String> = calls.lines().map(subcommand).collect();
        assert_eq!(
            lines[0], "fetch --progress --no-prune --no-prune-tags --all",
            "{lines:?}"
        );
        assert_eq!(lines[1], "lfs fetch --all origin", "{lines:?}");
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
echo "lfs=$GIT_LFS_FORCE_PROGRESS"; echo "ssh=$GIT_SSH_COMMAND"; \
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
        // git-lfs prints no progress without a terminal, which looked like a
        // stall to the supervisor.
        assert!(text.contains("lfs=1"), "{text}");
        assert!(
            text.contains("ssh=\n"),
            "HTTPS needs no SSH command: {text}"
        );
    }

    /// Regression: with `--prefer-ssh`, ssh asked about an unknown host key or
    /// a key passphrase on the controlling terminal, and git, in a background
    /// process group, was stopped by `SIGTTIN` for good.  ssh now runs in
    /// batch mode unless the user configured an SSH command.
    #[cfg(unix)]
    #[tokio::test]
    async fn ssh_runs_in_batch_mode_unless_the_user_chose_a_command() {
        if std::env::var_os("GIT_SSH_COMMAND").is_some() || std::env::var_os("GIT_SSH").is_some() {
            return; // the user's choice wins; nothing to observe here
        }
        for (configured, expected) in [(false, "ssh -o BatchMode=yes"), (true, "")] {
            let dir = tempfile::tempdir().expect("tempdir");
            let seen = dir.path().join("seen.txt");
            let config_status = if configured { 0 } else { 1 };
            let runner = runner_with_script(
                dir.path(),
                &format!(
                    r#"if [ "$1" = config ]; then exit {config_status}; fi
echo "ssh=$GIT_SSH_COMMAND" > '{}'"#,
                    seen.display()
                ),
            );
            runner
                .mirror_clone(
                    "git@github.com:o/r.git",
                    &dir.path().join("r.git"),
                    &with_stall(10),
                )
                .await
                .expect("ok");
            let text = std::fs::read_to_string(&seen).expect("script output");
            assert_eq!(
                text.trim(),
                format!("ssh={expected}"),
                "core.sshCommand configured: {configured}"
            );
        }
    }

    /// Regression: a stopped git was sent `SIGKILL` at once, so it never
    /// removed its `.lock` files and every later update of the repository
    /// failed.  The process group now gets `SIGTERM` and a grace period.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stopped_git_gets_sigterm_and_time_to_clean_up() {
        let dir = tempfile::tempdir().expect("tempdir");
        let started = dir.path().join("started");
        let cleaned = dir.path().join("cleaned");
        let runner = runner_with_script(
            dir.path(),
            &format!(
                "trap 'touch \"{}\"; exit 143' TERM\ntouch '{}'\nsleep 30 &\nwait",
                cleaned.display(),
                started.display()
            ),
        );
        let cancel = CancelFlag::new();
        let opts = CloneOptions {
            cancel: cancel.clone(),
            ..with_stall(60)
        };
        let dest = dir.path().join("repo.git");
        let task = {
            let runner = runner.clone();
            let dest = dest.clone();
            let opts = opts.clone();
            tokio::spawn(async move {
                runner
                    .mirror_clone("https://example.invalid/r.git", &dest, &opts)
                    .await
            })
        };
        let t0 = Instant::now();
        while !started.exists() {
            assert!(t0.elapsed() < Duration::from_secs(10), "never started");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        cancel.cancel();
        let result = task.await.expect("no panic");
        assert!(matches!(result, Err(CoreError::Interrupted)), "{result:?}");
        assert!(
            cleaned.exists(),
            "the child must get SIGTERM, not only SIGKILL"
        );
    }

    /// `run_fsck` checks every clone and update; a failed check is reported
    /// as `GitFsckFailed` and a fresh clone that fails is not kept.  Without
    /// it no check runs.  (It used to be logged only, after fresh clones only,
    /// unsupervised, and nothing could switch it on.)
    #[cfg(unix)]
    #[tokio::test]
    async fn verification_reports_a_failed_fsck_and_discards_a_bad_fresh_clone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = dir.path().join("calls.txt");
        let runner = runner_with_script(
            dir.path(),
            &format!(
                r#"echo "$@" >> '{}'
case " $* " in *" fsck "*) echo "error: object 1234 is corrupt" >&2; exit 2;; esac"#,
                log.display()
            ),
        );
        let checked = CloneOptions {
            run_fsck: true,
            ..with_stall(10)
        };
        let url = "https://example.invalid/r.git";

        let fresh = dir.path().join("fresh.git");
        let err = runner
            .mirror_clone(url, &fresh, &checked)
            .await
            .expect_err("a clone that fails its check fails");
        match &err {
            CoreError::GitFsckFailed { repo, output } => {
                assert_eq!(repo, &fresh.display().to_string());
                assert!(output.contains("is corrupt"), "{output}");
            }
            other => panic!("expected GitFsckFailed, got {other:?}"),
        }
        assert!(!fresh.exists(), "a clone that failed its check is not kept");
        assert!(staging_dirs(dir.path()).is_empty());

        let existing = dir.path().join("existing.git");
        std::fs::create_dir(&existing).expect("mkdir");
        for method in ["mirror", "bare", "full", "shallow"] {
            let result = match method {
                "mirror" => runner.mirror_clone(url, &existing, &checked).await,
                "bare" => runner.bare_clone(url, &existing, &checked).await,
                "full" => runner.full_clone(url, &existing, &checked).await,
                _ => runner.shallow_clone(url, &existing, &checked, 1).await,
            };
            assert!(
                matches!(result, Err(CoreError::GitFsckFailed { .. })),
                "{method}: {result:?}"
            );
        }

        std::fs::remove_file(&log).expect("reset log");
        runner
            .mirror_clone(url, &existing, &with_stall(10))
            .await
            .expect("no check, no failure");
        let calls = std::fs::read_to_string(&log).expect("calls");
        assert!(!calls.contains("fsck"), "{calls}");
    }

    /// A child that ignores `SIGTERM` is still stopped, after the grace period.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_child_ignoring_sigterm_is_killed_after_the_grace_period() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runner = runner_with_script(dir.path(), "trap '' TERM\nwhile :; do sleep 1; done");
        let started = Instant::now();
        let err = runner
            .mirror_clone(
                "https://example.invalid/r.git",
                &dir.path().join("r.git"),
                &with_stall(1),
            )
            .await
            .expect_err("stopped");
        assert!(matches!(err, CoreError::GitTimeout { .. }), "{err:?}");
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "{:?}",
            started.elapsed()
        );
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
        let lines: Vec<String> = argv.lines().map(subcommand).collect();
        assert_eq!(lines.len(), 2, "{argv}");
        assert!(lines[0].starts_with("clone --progress --mirror "), "{argv}");
        assert_eq!(
            lines[1], "fetch --progress --no-prune --no-prune-tags --all",
            "pruning is opt-in, and not pruning is explicit: {argv}"
        );

        // `prune` adds `--prune` to the update (and only to the update).
        let pruning = CloneOptions {
            prune: true,
            ..with_stall(10)
        };
        runner
            .mirror_clone("https://example.invalid/r.git", &existing, &pruning)
            .await
            .expect("pruning update");
        let argv = std::fs::read_to_string(&log).expect("argv log");
        let last = argv.lines().map(subcommand).next_back().expect("a call");
        assert_eq!(last, "fetch --progress --prune --all", "{argv}");
    }
}

// ── Credential tests ──────────────────────────────────────────────────────────

#[cfg(all(test, unix))]
mod credential_tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    const TOKEN: &str = "ghp_DUMMYTOKENforTESTSonly0000000000000";

    /// A one-purpose HTTP server: answers every request without credentials
    /// with `401 Basic` (or a redirect), and records each `Authorization`
    /// header it receives.
    struct Server {
        port: u16,
        auth: Arc<Mutex<Vec<String>>>,
    }

    impl Server {
        fn start(ip: &str, redirect_to: Option<String>) -> Self {
            let listener = TcpListener::bind((ip, 0)).expect("bind");
            let port = listener.local_addr().expect("addr").port();
            let auth = Arc::new(Mutex::new(Vec::new()));
            let seen = Arc::clone(&auth);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                    let mut has_auth = false;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                        if line.to_ascii_lowercase().starts_with("authorization:") {
                            seen.lock().unwrap().push(line.trim().to_owned());
                            has_auth = true;
                        }
                    }
                    let reply = match (&redirect_to, has_auth) {
                        (Some(to), false) => format!(
                            "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        ),
                        (_, false) => "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"x\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
                        (_, true) => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
                    };
                    let _ = stream.write_all(reply.as_bytes());
                    let mut sink = Vec::new();
                    let _ = reader.read_to_end(&mut sink);
                }
            });
            Self { port, auth }
        }

        fn auth_headers(&self) -> Vec<String> {
            self.auth.lock().unwrap().clone()
        }
    }

    fn basic(user: &str, pw: &str) -> String {
        // Minimal base64 for the expected header.
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let data = format!("{user}:{pw}").into_bytes();
        let mut out = String::new();
        for c in data.chunks(3) {
            let n = (u32::from(c[0]) << 16)
                | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
                | u32::from(*c.get(2).unwrap_or(&0));
            for i in 0..4 {
                if i <= c.len() {
                    out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        format!("Authorization: Basic {out}")
    }

    fn authed() -> CloneOptions {
        CloneOptions {
            token: Some(TOKEN.to_owned()),
            stall_timeout_secs: 20,
            ..CloneOptions::unauthenticated()
        }
    }

    /// Neither argv nor the temp dir ever holds the token; it travels only in
    /// the child's environment, and the helper list is reset first.
    #[tokio::test]
    async fn token_is_never_in_argv_and_no_file_is_created() {
        let dir = tempfile::tempdir().expect("tempdir");
        let seen = dir.path().join("seen.txt");
        let script = dir.path().join("fake-git");
        std::fs::write(
            &script,
            format!(
                "{{ printf '%s ' \"$@\"; echo; echo \"env=$GH_BACKUP_GIT_TOKEN\"; echo \"askpass=$GIT_ASKPASS\"; }} > '{}'\nfor d in \"$@\"; do :; done; mkdir -p \"$d\"\n",
                seen.display()
            ),
        )
        .expect("write");
        let runner = ProcessGitRunner::with_shell_script(&script);
        runner
            .mirror_clone(
                "https://ghes.example.com/o/r.git",
                &dir.path().join("r.git"),
                &authed(),
            )
            .await
            .expect("clone");
        let text = std::fs::read_to_string(&seen).expect("seen");
        let (cmdline, env) = text.split_once('\n').expect("two parts");
        assert!(!cmdline.contains(TOKEN), "token in cmdline: {cmdline}");
        assert!(cmdline.contains("-c credential.helper= "), "{cmdline}");
        assert!(
            cmdline.contains("credential.https://ghes.example.com.helper="),
            "{cmdline}"
        );
        assert!(env.contains(&format!("env={TOKEN}")), "{env}");
        assert!(env.contains("askpass=\n"), "no askpass is set: {env}");
        let leftover = std::env::temp_dir().join(format!("gh-backup-{}", std::process::id()));
        assert!(!leftover.exists(), "no askpass directory may be created");
    }

    /// Real git: a fresh clone offers the token (as `x-access-token`) to the
    /// clone host.
    #[tokio::test]
    async fn clone_offers_the_token_to_the_clone_host() {
        let server = Server::start("127.0.0.1", None);
        let dir = tempfile::tempdir().expect("tempdir");
        let url = format!("http://127.0.0.1:{}/o/r.git", server.port);
        let result = ProcessGitRunner::new()
            .mirror_clone(&url, &dir.path().join("r.git"), &authed())
            .await;
        assert!(result.is_err(), "the fake server has no repository");
        assert!(
            server
                .auth_headers()
                .contains(&basic("x-access-token", TOKEN)),
            "{:?}",
            server.auth_headers()
        );
    }

    /// Real git, update path: an existing repository is fetched with the
    /// token too.
    #[tokio::test]
    async fn update_offers_the_token_to_the_clone_host() {
        let server = Server::start("127.0.0.1", None);
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("r.git");
        let url = format!("http://127.0.0.1:{}/o/r.git", server.port);
        let d = dest.to_str().unwrap();
        for args in [
            vec!["init", "--bare", "-q", d],
            vec!["-C", d, "remote", "add", "origin", &url],
        ] {
            assert!(Command::new("git")
                .args(&args)
                .status()
                .expect("git")
                .success());
        }
        let result = ProcessGitRunner::new()
            .mirror_clone(&url, &dest, &authed())
            .await;
        assert!(result.is_err());
        assert!(
            server
                .auth_headers()
                .contains(&basic("x-access-token", TOKEN)),
            "{:?}",
            server.auth_headers()
        );
    }

    /// A redirect to another host (as a hostile server or `.lfsconfig` would
    /// cause) is never offered the token.
    #[tokio::test]
    async fn token_is_not_offered_to_a_different_host() {
        // `localhost` is a different host from `127.0.0.1` as far as git's
        // credential scoping goes, and (unlike 127.0.0.2) it is available on
        // every platform the tests run on, macOS included.
        let other = Server::start("127.0.0.1", None);
        let origin = Server::start(
            "127.0.0.1",
            Some(format!(
                "http://localhost:{}/o/r.git/info/refs?service=git-upload-pack",
                other.port
            )),
        );
        let dir = tempfile::tempdir().expect("tempdir");
        let url = format!("http://127.0.0.1:{}/o/r.git", origin.port);
        let result = ProcessGitRunner::new()
            .mirror_clone(&url, &dir.path().join("r.git"), &authed())
            .await;
        assert!(result.is_err());
        assert!(
            other.auth_headers().is_empty(),
            "token leaked to another host: {:?}",
            other.auth_headers()
        );
    }

    /// Without a token (public repos) no credential configuration is added.
    #[tokio::test]
    async fn unauthenticated_clone_sends_no_credentials() {
        let server = Server::start("127.0.0.1", None);
        let dir = tempfile::tempdir().expect("tempdir");
        let url = format!("http://127.0.0.1:{}/o/r.git", server.port);
        let opts = CloneOptions {
            stall_timeout_secs: 20,
            ..CloneOptions::unauthenticated()
        };
        let _ = ProcessGitRunner::new()
            .mirror_clone(&url, &dir.path().join("r.git"), &opts)
            .await;
        assert!(server.auth_headers().is_empty());
    }

    /// A runner whose `git` is the shell script `body`, run through `sh`.
    ///
    /// Regression (CI flake): the script used to be written, made executable
    /// and `exec`ed, which fails with "Text file busy" when another test forks
    /// while the file is still open for writing.
    fn fake_git(dir: &Path, body: &str) -> ProcessGitRunner {
        let script = dir.join("fake-git-2");
        std::fs::write(&script, format!("{body}\n")).expect("write");
        ProcessGitRunner::with_shell_script(script)
    }

    /// Updating a repository we manage trusts exactly that path (never `*`),
    /// so a repository owned by another uid does not fail with "dubious
    /// ownership" forever.
    #[tokio::test]
    async fn updates_trust_exactly_the_managed_repository() {
        let dir = tempfile::tempdir().expect("tempdir");
        let seen = dir.path().join("argv.txt");
        let runner = fake_git(dir.path(), &format!("echo \"$@\" > '{}'", seen.display()));
        let dest = dir.path().join("r.git");
        std::fs::create_dir(&dest).expect("mkdir");
        runner
            .mirror_clone("https://example.invalid/r.git", &dest, &authed())
            .await
            .expect("update");
        let argv = std::fs::read_to_string(&seen).expect("argv");
        let real = dest.canonicalize().expect("canonical");
        assert!(
            argv.contains(&format!("-c safe.directory={} ", real.display())),
            "{argv}"
        );
        assert!(!argv.contains("safe.directory=*"), "{argv}");
    }

    /// The "dubious ownership" failure carries an actionable hint.
    #[tokio::test]
    async fn dubious_ownership_error_gets_a_hint() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runner = fake_git(
            dir.path(),
            "echo \"fatal: detected dubious ownership in repository at '/x'\" >&2; exit 128",
        );
        let dest = dir.path().join("r.git");
        std::fs::create_dir(&dest).expect("mkdir");
        let err = runner
            .mirror_clone("https://example.invalid/r.git", &dest, &authed())
            .await
            .expect_err("fails");
        let text = err.to_string();
        assert!(text.contains("dubious ownership"), "{text}");
        assert!(text.contains("--user") && text.contains("chown"), "{text}");
    }
}

/// Real git against a local origin: what an update does with refs that were
/// deleted upstream.
#[cfg(test)]
mod prune_tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
            .args([
                "-c",
                "init.defaultBranch=main",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn origin_with_branches(dir: &Path, branches: &[&str]) -> String {
        let origin = dir.join("origin");
        std::fs::create_dir(&origin).expect("mkdir");
        git(&origin, &["init", "-q"]);
        git(&origin, &["commit", "-q", "--allow-empty", "-m", "base"]);
        for b in branches {
            git(&origin, &["branch", b]);
        }
        format!("file://{}", origin.display())
    }

    fn refs(repo: &Path) -> Vec<String> {
        git(
            repo,
            &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
        )
        .lines()
        .map(str::to_owned)
        .collect()
    }

    async fn mirror(url: &str, dest: &Path, prune: bool) -> Result<(), CoreError> {
        let opts = CloneOptions {
            prune,
            ..CloneOptions::unauthenticated()
        };
        ProcessGitRunner::new().mirror_clone(url, dest, &opts).await
    }

    #[tokio::test]
    async fn a_branch_deleted_upstream_is_kept_unless_pruning_is_asked_for() {
        for (prune, expect_kept) in [(false, true), (true, false)] {
            let dir = tempfile::tempdir().expect("tempdir");
            let url = origin_with_branches(dir.path(), &["feature"]);
            let dest = dir.path().join("mirror.git");
            mirror(&url, &dest, prune).await.expect("clone");
            assert!(refs(&dest).contains(&"feature".to_owned()));

            git(&dir.path().join("origin"), &["branch", "-D", "feature"]);
            mirror(&url, &dest, prune).await.expect("update");

            assert_eq!(
                refs(&dest).contains(&"feature".to_owned()),
                expect_kept,
                "prune={prune}: {:?}",
                refs(&dest)
            );
        }
    }

    /// Without pruning, `foo` (deleted) and `foo/bar` (new) cannot coexist as
    /// refs.  The update must not fail forever: it prunes once and goes on.
    #[tokio::test]
    async fn a_new_branch_under_a_deleted_branchs_name_does_not_wedge_the_update() {
        let dir = tempfile::tempdir().expect("tempdir");
        let url = origin_with_branches(dir.path(), &["foo"]);
        let dest = dir.path().join("mirror.git");
        mirror(&url, &dest, false).await.expect("clone");

        let origin = dir.path().join("origin");
        git(&origin, &["branch", "-D", "foo"]);
        git(&origin, &["branch", "foo/bar"]);
        mirror(&url, &dest, false)
            .await
            .expect("the update recovers instead of failing");

        let now = refs(&dest);
        assert!(now.contains(&"foo/bar".to_owned()), "{now:?}");
        assert!(!now.contains(&"foo".to_owned()), "{now:?}");
    }

    fn has_ref(repo: &Path, name: &str) -> bool {
        Command::new("git")
            .args(["rev-parse", "--verify", "--quiet", name])
            .current_dir(repo)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("run git")
            .success()
    }

    /// Updates `dest` with every clone type, the way the engine does.
    async fn update_as(
        runner: &ProcessGitRunner,
        kind: &str,
        url: &str,
        dest: &Path,
        opts: &CloneOptions,
    ) -> Result<(), CoreError> {
        match kind {
            "mirror" => runner.mirror_clone(url, dest, opts).await,
            "bare" => runner.bare_clone(url, dest, opts).await,
            "full" => runner.full_clone(url, dest, opts).await,
            "shallow" => runner.shallow_clone(url, dest, opts, 1).await,
            other => panic!("unknown clone type {other}"),
        }
    }

    /// Regression: with pruning off, updates passed no prune flag at all, so a
    /// `fetch.prune` / `fetch.pruneTags` / `remote.<name>.prune` setting in the
    /// user's git configuration silently deleted the refs the tool keeps.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_users_prune_settings_cannot_delete_kept_refs() {
        for kind in ["mirror", "bare", "full", "shallow"] {
            let dir = tempfile::tempdir().expect("tempdir");
            let url = origin_with_branches(dir.path(), &["feature"]);
            let origin = dir.path().join("origin");
            git(&origin, &["tag", "v1"]);
            let dest = dir.path().join("clone");
            let plain = ProcessGitRunner::new();
            let opts = CloneOptions::unauthenticated();
            update_as(&plain, kind, &url, &dest, &opts)
                .await
                .expect("clone");
            assert!(has_ref(&dest, "refs/tags/v1"), "{kind}: tag not cloned");
            let cloned = if kind == "full" {
                "refs/remotes/origin/feature"
            } else {
                "refs/heads/feature"
            };
            assert!(has_ref(&dest, cloned), "{kind}: branch not cloned");

            git(&origin, &["branch", "-D", "feature"]);
            git(&origin, &["tag", "-d", "v1"]);

            // git as the user has it configured.
            let config = dir.path().join("gitconfig");
            std::fs::write(
                &config,
                "[fetch]\n\tprune = true\n\tpruneTags = true\n[remote \"origin\"]\n\tprune = true\n\tpruneTags = true\n",
            )
            .expect("config");
            let wrapper = dir.path().join("git-with-user-config");
            std::fs::write(
                &wrapper,
                format!("GIT_CONFIG_GLOBAL='{}' exec git \"$@\"\n", config.display()),
            )
            .expect("wrapper");
            let configured = ProcessGitRunner::with_shell_script(&wrapper);
            update_as(&configured, kind, &url, &dest, &opts)
                .await
                .expect("update");

            let branch = if kind == "full" {
                "refs/remotes/origin/feature"
            } else {
                "refs/heads/feature"
            };
            assert!(has_ref(&dest, branch), "{kind}: {branch} was pruned");
            assert!(has_ref(&dest, "refs/tags/v1"), "{kind}: tag v1 was pruned");
        }
    }

    /// Regression: updating a full clone moved only `refs/remotes/origin/*`;
    /// the working tree, the local branches and moved tags stayed at the state
    /// of the first clone forever.
    #[tokio::test]
    async fn a_full_clone_update_brings_checkout_branches_and_tags_upstream() {
        let dir = tempfile::tempdir().expect("tempdir");
        let url = origin_with_branches(dir.path(), &[]);
        let origin = dir.path().join("origin");
        std::fs::write(origin.join("file.txt"), "one\n").expect("write");
        git(&origin, &["add", "file.txt"]);
        git(&origin, &["commit", "-q", "-m", "one"]);
        git(&origin, &["tag", "release"]);
        let dest = dir.path().join("full");
        let runner = ProcessGitRunner::new();
        let opts = CloneOptions::unauthenticated();
        runner.full_clone(&url, &dest, &opts).await.expect("clone");

        std::fs::write(origin.join("file.txt"), "two\n").expect("write");
        git(&origin, &["commit", "-q", "-am", "two"]);
        git(&origin, &["tag", "-f", "release"]);
        git(&origin, &["branch", "topic"]);
        runner.full_clone(&url, &dest, &opts).await.expect("update");

        let upstream = git(&origin, &["rev-parse", "HEAD"]);
        assert_eq!(
            std::fs::read_to_string(dest.join("file.txt")).expect("read"),
            "two\n",
            "the working tree must follow upstream"
        );
        assert_eq!(git(&dest, &["rev-parse", "HEAD"]), upstream);
        assert_eq!(git(&dest, &["rev-parse", "refs/heads/main"]), upstream);
        assert_eq!(
            git(&dest, &["rev-parse", "refs/tags/release^{commit}"]),
            upstream,
            "a moved tag must move"
        );
        assert!(has_ref(&dest, "refs/heads/topic"), "new branches appear");
        assert!(
            git(&dest, &["status", "--porcelain"]).is_empty(),
            "the checkout must be clean"
        );
    }

    /// With verification on, healthy clones and updates of every kind pass
    /// `git fsck` (the arguments are ones real git accepts, also in a
    /// working-tree clone).
    #[tokio::test]
    async fn verification_passes_healthy_repositories_with_real_git() {
        for kind in ["mirror", "bare", "full", "shallow"] {
            let dir = tempfile::tempdir().expect("tempdir");
            let url = origin_with_branches(dir.path(), &["feature"]);
            let dest = dir.path().join("clone");
            let opts = CloneOptions {
                run_fsck: true,
                ..CloneOptions::unauthenticated()
            };
            let runner = ProcessGitRunner::new();
            update_as(&runner, kind, &url, &dest, &opts)
                .await
                .unwrap_or_else(|e| panic!("{kind} clone: {e}"));
            update_as(&runner, kind, &url, &dest, &opts)
                .await
                .unwrap_or_else(|e| panic!("{kind} update: {e}"));
        }
    }

    /// An empty upstream repository has nothing to check out; updating its
    /// full clone must not fail.
    #[tokio::test]
    async fn a_full_clone_of_an_empty_repository_updates_cleanly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let origin = dir.path().join("empty.git");
        std::fs::create_dir(&origin).expect("mkdir");
        git(&origin, &["init", "-q", "--bare"]);
        let url = format!("file://{}", origin.display());
        let dest = dir.path().join("full");
        let runner = ProcessGitRunner::new();
        let opts = CloneOptions::unauthenticated();
        runner.full_clone(&url, &dest, &opts).await.expect("clone");
        runner.full_clone(&url, &dest, &opts).await.expect("update");
    }

    #[test]
    fn only_ref_name_clashes_trigger_the_pruning_retry() {
        // What git 2.43 prints for both directions of the clash.
        assert!(is_ref_name_conflict(
            "error: cannot lock ref 'refs/heads/foo/bar': 'refs/heads/foo' exists; cannot create 'refs/heads/foo/bar'"
        ));
        assert!(is_ref_name_conflict(
            "error: cannot lock ref 'refs/heads/x': 'refs/heads/x/y' exists; cannot create 'refs/heads/x'"
        ));
        assert!(is_ref_name_conflict(
            "error: cannot lock ref 'refs/heads/x': there is a non-empty directory '/b/refs/heads/x' blocking reference 'refs/heads/x'"
        ));
        assert!(is_ref_name_conflict(
            "error: some local refs could not be updated; try running\n 'git remote prune origin' to remove any old, conflicting branches"
        ));
        assert!(!is_ref_name_conflict("fatal: repository not found"));
        assert!(!is_ref_name_conflict(
            "fatal: unable to access: could not resolve host"
        ));
        // A lock file left by a killed git is not a name clash: pruning
        // would delete kept refs and fail again.
        assert!(!is_ref_name_conflict(
            "error: cannot lock ref 'refs/heads/main': Unable to create '/b/r.git/./refs/heads/main.lock': File exists.\n\
             \nAnother git process seems to be running in this repository, e.g.\n\
             an editor opened by 'git commit'. Please make sure all processes\n\
             are terminated then try again. If it still fails, a git process\n\
             may have crashed in this repository earlier:\n\
             remove the file manually to continue.\n\
              ! 05e71f7..93011c4  main       -> main  (unable to update local ref)"
        ));
    }

    /// Ages `path` by `secs` seconds.
    fn backdate(path: &Path, secs: u64) {
        let file = std::fs::File::options()
            .write(true)
            .open(path)
            .expect("open");
        file.set_modified(SystemTime::now() - Duration::from_secs(secs))
            .expect("set mtime");
    }

    /// Regression: a git killed with `SIGKILL` leaves `refs/heads/main.lock`;
    /// the "cannot lock ref" error was taken for a name clash, the update was
    /// retried with `--prune` (deleting kept refs) and failed again on every
    /// run.  A lock older than the stall limit is now removed first; a recent
    /// one is left alone and the failure is reported, refs intact.
    #[tokio::test]
    async fn a_stale_ref_lock_is_removed_and_a_recent_one_is_not_pruned_around() {
        let dir = tempfile::tempdir().expect("tempdir");
        let url = origin_with_branches(dir.path(), &["kept"]);
        let origin = dir.path().join("origin");
        let dest = dir.path().join("mirror.git");
        mirror(&url, &dest, false).await.expect("clone");
        git(&origin, &["branch", "-D", "kept"]);
        git(&origin, &["commit", "-q", "--allow-empty", "-m", "next"]);

        // A lock left by a git that is still (or was very recently) running.
        let lock = dest.join("refs").join("heads").join("main.lock");
        std::fs::write(&lock, "").expect("lock");
        let err = mirror(&url, &dest, false)
            .await
            .expect_err("a held lock fails the update");
        assert!(
            err.to_string().contains("hint: a git process holds"),
            "{err}"
        );
        assert!(lock.exists(), "a recent lock must not be removed");
        assert!(
            refs(&dest).contains(&"kept".to_owned()),
            "the failure must not be 'fixed' by pruning: {:?}",
            refs(&dest)
        );

        // The same lock, left by a git killed long ago.
        backdate(&lock, 2 * DEFAULT_STALL_TIMEOUT_SECS);
        mirror(&url, &dest, false).await.expect("update");
        assert!(!lock.exists());
        assert_eq!(
            git(&dest, &["rev-parse", "main"]),
            git(&origin, &["rev-parse", "main"])
        );
        assert!(refs(&dest).contains(&"kept".to_owned()));
    }

    #[test]
    fn only_old_lock_files_in_the_git_directory_are_removed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("work");
        let git_dir = repo.join(".git");
        std::fs::create_dir_all(git_dir.join("refs/heads/feature")).expect("mkdir");
        let old = [
            git_dir.join("index.lock"),
            git_dir.join("packed-refs.lock"),
            git_dir.join("refs/heads/feature/x.lock"),
        ];
        let young = git_dir.join("HEAD.lock");
        let not_a_lock = git_dir.join("refs/heads/main");
        let outside = repo.join("notes.lock");
        for path in old.iter().chain([&young, &not_a_lock, &outside]) {
            std::fs::write(path, "").expect("write");
        }
        for path in old.iter().chain([&not_a_lock, &outside]) {
            backdate(path, 3600);
        }

        remove_stale_locks(&repo, Duration::from_secs(600));

        for path in &old {
            assert!(!path.exists(), "{} must be removed", path.display());
        }
        assert!(young.exists(), "a recent lock may belong to a live git");
        assert!(not_a_lock.exists());
        assert!(outside.exists(), "the working tree is not touched");
    }
}
