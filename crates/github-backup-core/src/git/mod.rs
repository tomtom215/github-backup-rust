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
//! - **Clone timeout** — each git subprocess is killed if it exceeds
//!   `CloneOptions::clone_timeout_secs` (default 600 s).  This prevents a
//!   single stalled clone from blocking a worker indefinitely.
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
//! - `spy` — test-only `SpyGitRunner` stub (available under `test_support` in tests)

mod askpass;
pub mod spy;

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

use crate::error::CoreError;
use askpass::AskpassScript;

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

/// Default per-clone timeout: 10 minutes.
const DEFAULT_CLONE_TIMEOUT_SECS: u64 = 600;

/// Git clone options passed to the runner.
#[derive(Debug, Clone)]
pub struct CloneOptions {
    /// Token to inject for HTTPS authentication, or `None` for unauthenticated
    /// (public repos) or SSH-based cloning.
    pub token: Option<String>,
    /// When `true`, skip `--prune` during updates.
    pub no_prune: bool,
    /// Maximum seconds to wait for any single git subprocess before killing
    /// it and returning [`CoreError::GitTimeout`].
    ///
    /// Defaults to [`DEFAULT_CLONE_TIMEOUT_SECS`] (600 s).
    pub clone_timeout_secs: u64,
    /// When `true`, run `git fsck --no-dangling` after every *fresh* clone to
    /// detect repository corruption early.
    pub run_fsck: bool,
}

impl CloneOptions {
    /// No authentication, prune enabled, default timeout, fsck disabled.
    #[must_use]
    pub fn unauthenticated() -> Self {
        Self {
            token: None,
            no_prune: false,
            clone_timeout_secs: DEFAULT_CLONE_TIMEOUT_SECS,
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
pub trait GitRunner: Send + Sync {
    /// Clones `url` into `dest` as a bare mirror (`git clone --mirror`).
    ///
    /// If `dest` already exists, updates with `git remote update` (pruning
    /// deleted refs unless `opts.no_prune` is set).
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::GitFailed`] if git exits non-zero, or
    /// [`CoreError::GitSpawn`] if the binary cannot be started.
    fn mirror_clone(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), CoreError>;

    /// Clones `url` into `dest` as a bare clone (`git clone --bare`).
    ///
    /// Similar to `mirror_clone` but does not configure remote-tracking refs.
    /// If `dest` already exists, updates refs with `git fetch --all`.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::GitFailed`] or [`CoreError::GitSpawn`].
    fn bare_clone(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), CoreError>;

    /// Clones `url` into `dest` as a full working-tree clone.
    ///
    /// Use when you need to browse or build the backed-up source code.
    /// If `dest` already exists, updates with `git fetch --all --prune`.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::GitFailed`] or [`CoreError::GitSpawn`].
    fn full_clone(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), CoreError>;

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
    ) -> Result<(), CoreError>;

    /// Clones `url` into `dest` using Git LFS.
    ///
    /// Fetches LFS objects in addition to regular git objects.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::GitFailed`] or [`CoreError::GitSpawn`].
    fn lfs_clone(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), CoreError>;

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
    ) -> Result<(), CoreError>;
}

// ── Production implementation ─────────────────────────────────────────────────

/// Production [`GitRunner`] that shells out to the system `git` binary.
#[derive(Debug, Clone, Default)]
pub struct ProcessGitRunner;

impl ProcessGitRunner {
    /// Creates a new [`ProcessGitRunner`].
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl GitRunner for ProcessGitRunner {
    fn mirror_clone(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), CoreError> {
        let dest_str = path_to_str(dest)?;
        if dest.exists() {
            info!(dest = %dest.display(), "repository exists, updating mirror");
            let update_args: &[&str] = if opts.no_prune {
                &["remote", "update"]
            } else {
                &["remote", "update", "--prune"]
            };
            run_git(update_args, dest, opts.token.as_deref(), opts)
        } else {
            info!(url = %url, dest = %dest.display(), "cloning bare mirror");
            let args = &["clone", "--mirror", url, dest_str];
            clone_with_cleanup(args, dest, opts)
        }
    }

    fn bare_clone(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), CoreError> {
        let dest_str = path_to_str(dest)?;
        if dest.exists() {
            info!(dest = %dest.display(), "bare repository exists, fetching");
            let fetch_args: &[&str] = if opts.no_prune {
                &["fetch", "--all"]
            } else {
                &["fetch", "--all", "--prune"]
            };
            run_git(fetch_args, dest, opts.token.as_deref(), opts)
        } else {
            info!(url = %url, dest = %dest.display(), "cloning bare");
            let args = &["clone", "--bare", url, dest_str];
            clone_with_cleanup(args, dest, opts)
        }
    }

    fn full_clone(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), CoreError> {
        let dest_str = path_to_str(dest)?;
        if dest.exists() {
            info!(dest = %dest.display(), "full clone exists, fetching all branches");
            let fetch_args: &[&str] = if opts.no_prune {
                &["fetch", "--all"]
            } else {
                &["fetch", "--all", "--prune"]
            };
            run_git(fetch_args, dest, opts.token.as_deref(), opts)
        } else {
            info!(url = %url, dest = %dest.display(), "cloning full working tree");
            let args = &["clone", "--no-local", url, dest_str];
            clone_with_cleanup(args, dest, opts)
        }
    }

    fn shallow_clone(
        &self,
        url: &str,
        dest: &Path,
        opts: &CloneOptions,
        depth: u32,
    ) -> Result<(), CoreError> {
        let dest_str = path_to_str(dest)?;
        let depth_str = depth.to_string();
        if dest.exists() {
            info!(dest = %dest.display(), depth, "shallow clone exists, deepening fetch");
            run_git(
                &["fetch", "--depth", &depth_str],
                dest,
                opts.token.as_deref(),
                opts,
            )
        } else {
            info!(url = %url, dest = %dest.display(), depth, "cloning shallow");
            let args = &["clone", "--mirror", "--depth", &depth_str, url, dest_str];
            clone_with_cleanup(args, dest, opts)
        }
    }

    fn lfs_clone(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), CoreError> {
        let dest_str = path_to_str(dest)?;
        if dest.exists() {
            info!(dest = %dest.display(), "LFS repository exists, updating");
            run_git(
                &["lfs", "fetch", "--all"],
                dest,
                opts.token.as_deref(),
                opts,
            )
        } else {
            info!(url = %url, dest = %dest.display(), "cloning with LFS");
            let args = &["lfs", "clone", url, dest_str];
            clone_with_cleanup(args, dest, opts)
        }
    }

    fn push_mirror(
        &self,
        src: &Path,
        remote_url: &str,
        opts: &CloneOptions,
    ) -> Result<(), CoreError> {
        info!(
            src = %src.display(),
            remote = %remote_url,
            "pushing mirror to remote"
        );
        run_git(
            &["push", "--mirror", remote_url],
            src,
            opts.token.as_deref(),
            opts,
        )
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

/// Performs a **fresh** clone: runs `git` with `args`, then:
/// - On failure: removes the partially-written `dest` directory.
/// - On success (when `opts.run_fsck`): runs `git fsck` and logs issues.
fn clone_with_cleanup(args: &[&str], dest: &Path, opts: &CloneOptions) -> Result<(), CoreError> {
    match run_git(args, Path::new("."), opts.token.as_deref(), opts) {
        Ok(()) => {
            if opts.run_fsck && dest.exists() {
                run_fsck(dest);
            }
            Ok(())
        }
        Err(e) => {
            // Remove any partial clone directory so the next run starts fresh.
            if dest.exists() {
                if let Err(rm_err) = std::fs::remove_dir_all(dest) {
                    warn!(
                        dest = %dest.display(),
                        error = %rm_err,
                        "failed to remove partial clone directory after git failure"
                    );
                } else {
                    debug!(dest = %dest.display(), "removed partial clone directory");
                }
            }
            Err(e)
        }
    }
}

/// Runs `git fsck --no-dangling` on `repo_dir` and logs any issues found.
///
/// Corruption is not treated as a fatal error — the backup has already
/// completed — but the issues are logged at `warn` level so operators can
/// investigate.
fn run_fsck(repo_dir: &Path) {
    debug!(repo = %repo_dir.display(), "running git fsck");
    let output = Command::new("git")
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

/// Runs `git` with `args` in `cwd`, enforcing a per-process timeout.
///
/// If `token` is `Some`, `GIT_TERMINAL_PROMPT` is disabled and `GIT_ASKPASS`
/// is set to a small inline script that echoes the token.  This keeps the
/// credential out of the command line and the process list.
///
/// The subprocess is polled every 100 ms.  If `opts.clone_timeout_secs`
/// elapses before it exits, the process is killed and
/// [`CoreError::GitTimeout`] is returned.
fn run_git(
    args: &[&str],
    cwd: &Path,
    token: Option<&str>,
    opts: &CloneOptions,
) -> Result<(), CoreError> {
    debug!(args = ?args, cwd = %cwd.display(), "running git");

    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Inject token via GIT_ASKPASS to avoid embedding it in the URL.
    let _askpass_guard;
    if let Some(tok) = token {
        _askpass_guard = AskpassScript::create(tok);
        if let Some(ref script) = _askpass_guard {
            cmd.env("GIT_TERMINAL_PROMPT", "0");
            cmd.env("GIT_ASKPASS", script.path());
            cmd.env("GIT_USERNAME", "x-access-token");
        }
    } else {
        _askpass_guard = None;
    }

    let timeout = Duration::from_secs(opts.clone_timeout_secs);
    let mut child = cmd.spawn().map_err(CoreError::GitSpawn)?;
    // _askpass_guard kept alive until after process exits.

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait().map_err(CoreError::GitSpawn)? {
            Some(status) => {
                // Collect output from piped streams.
                let stderr = read_child_stream(child.stderr.take());
                let stdout = read_child_stream(child.stdout.take());

                if status.success() {
                    return Ok(());
                }
                let _ = stdout; // stdout rarely has useful info for errors
                return Err(CoreError::GitFailed {
                    args: args.join(" "),
                    code: status.code().unwrap_or(-1),
                    stderr,
                });
            }
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    // Reap the child to avoid zombies.
                    let _ = child.wait();
                    return Err(CoreError::GitTimeout {
                        args: args.join(" "),
                        timeout_secs: opts.clone_timeout_secs,
                    });
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// Reads all bytes from an optional piped stream into a lossy UTF-8 string.
fn read_child_stream(stream: Option<impl Read>) -> String {
    let Some(mut s) = stream else {
        return String::new();
    };
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::spy::SpyGitRunner;
    use super::*;
    use std::path::PathBuf;

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
        assert_eq!(opts.clone_timeout_secs, DEFAULT_CLONE_TIMEOUT_SECS);
        assert!(!opts.run_fsck);
    }

    #[test]
    fn clone_options_default_matches_unauthenticated() {
        let a = CloneOptions::default();
        let b = CloneOptions::unauthenticated();
        assert_eq!(a.clone_timeout_secs, b.clone_timeout_secs);
        assert_eq!(a.run_fsck, b.run_fsck);
        assert_eq!(a.no_prune, b.no_prune);
    }

    #[test]
    fn spy_runner_mirror_clone() {
        let runner = SpyGitRunner::default();
        let dest = PathBuf::from("/tmp/test.git");
        runner
            .mirror_clone("https://github.com/octocat/Hello-World.git", &dest, &opts())
            .expect("mirror clone");
        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "mirror_clone");
    }

    #[test]
    fn spy_runner_push_mirror() {
        let runner = SpyGitRunner::default();
        let src = PathBuf::from("/tmp/local.git");
        runner
            .push_mirror(&src, "https://gitea.example.com/user/repo.git", &opts())
            .expect("push mirror");
        let calls = runner.recorded_calls();
        assert_eq!(calls[0].method, "push_mirror");
    }
}
