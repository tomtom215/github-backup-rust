// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! [`SpyGitRunner`] — a test-only [`GitRunner`] stub.
//!
//! Records every call made to it without executing any git processes.
//! Useful for unit-testing code that calls the [`GitRunner`] trait without
//! hitting the filesystem or network.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::{CloneOptions, GitRunner};
use crate::error::CoreError;

/// A [`GitRunner`] stub that records calls but does not invoke git.
///
/// All methods succeed immediately and push a [`GitCall`] entry to the
/// shared call log so tests can assert on what was called.  A failure can be
/// injected for URLs containing a given text with
/// [`SpyGitRunner::failing_when_url_contains`].
#[derive(Debug, Clone, Default)]
pub struct SpyGitRunner {
    /// All recorded git operation calls.
    pub calls: Arc<Mutex<Vec<GitCall>>>,
    failures: Arc<Mutex<Vec<(String, SpyFailure)>>>,
}

/// The kind of failure a [`SpyGitRunner`] can be told to produce.
#[derive(Debug, Clone)]
pub enum SpyFailure {
    /// git exited with `code` and printed `stderr`.
    Git {
        /// Exit status.
        code: i32,
        /// What git printed.
        stderr: String,
    },
    /// The run was cancelled.
    Interrupted,
}

/// A recorded call to a [`GitRunner`] method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCall {
    /// Method name, e.g. `"mirror_clone"`, `"full_clone"`, `"push_mirror"`.
    pub method: String,
    /// The URL or remote argument.
    pub url: String,
    /// The destination or source path argument.
    pub dest: PathBuf,
}

impl GitRunner for SpyGitRunner {
    async fn mirror_clone(
        &self,
        url: &str,
        dest: &Path,
        _opts: &CloneOptions,
    ) -> Result<(), CoreError> {
        self.record("mirror_clone", url, dest);
        self.outcome(url)
    }

    async fn bare_clone(
        &self,
        url: &str,
        dest: &Path,
        _opts: &CloneOptions,
    ) -> Result<(), CoreError> {
        self.record("bare_clone", url, dest);
        self.outcome(url)
    }

    async fn full_clone(
        &self,
        url: &str,
        dest: &Path,
        _opts: &CloneOptions,
    ) -> Result<(), CoreError> {
        self.record("full_clone", url, dest);
        self.outcome(url)
    }

    async fn shallow_clone(
        &self,
        url: &str,
        dest: &Path,
        _opts: &CloneOptions,
        _depth: u32,
    ) -> Result<(), CoreError> {
        self.record("shallow_clone", url, dest);
        self.outcome(url)
    }

    async fn lfs_clone(
        &self,
        url: &str,
        dest: &Path,
        _opts: &CloneOptions,
    ) -> Result<(), CoreError> {
        self.record("lfs_clone", url, dest);
        self.outcome(url)
    }

    async fn push_mirror(
        &self,
        src: &Path,
        remote_url: &str,
        _opts: &CloneOptions,
    ) -> Result<(), CoreError> {
        self.record("push_mirror", remote_url, src);
        self.outcome(remote_url)
    }
}

impl SpyGitRunner {
    /// Makes every call whose URL contains `needle` fail with `failure`.
    #[must_use]
    pub fn failing_when_url_contains(self, needle: &str, failure: SpyFailure) -> Self {
        self.failures
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((needle.to_string(), failure));
        self
    }

    fn outcome(&self, url: &str) -> Result<(), CoreError> {
        let failures = self.failures.lock().unwrap_or_else(|p| p.into_inner());
        match failures
            .iter()
            .find(|(needle, _)| url.contains(needle.as_str()))
        {
            None => Ok(()),
            Some((_, SpyFailure::Interrupted)) => Err(CoreError::Interrupted),
            Some((_, SpyFailure::Git { code, stderr })) => Err(CoreError::GitFailed {
                args: format!("clone {url}"),
                code: *code,
                stderr: stderr.clone(),
            }),
        }
    }

    fn record(&self, method: &str, url: &str, dest: &Path) {
        self.calls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(GitCall {
                method: method.to_string(),
                url: url.to_string(),
                dest: dest.to_path_buf(),
            });
    }

    /// Returns all recorded calls.
    pub fn recorded_calls(&self) -> Vec<GitCall> {
        self.calls.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> CloneOptions {
        CloneOptions::unauthenticated()
    }

    #[tokio::test]
    async fn mirror_clone_records_call() {
        let runner = SpyGitRunner::default();
        let dest = PathBuf::from("/tmp/test.git");
        runner
            .mirror_clone("https://github.com/octocat/Hello-World.git", &dest, &opts())
            .await
            .expect("mirror clone");

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "mirror_clone");
        assert_eq!(calls[0].dest, dest);
    }

    #[tokio::test]
    async fn bare_clone_records_call() {
        let runner = SpyGitRunner::default();
        let dest = PathBuf::from("/tmp/bare.git");
        runner
            .bare_clone("https://github.com/octocat/Hello-World.git", &dest, &opts())
            .await
            .expect("bare clone");

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "bare_clone");
    }

    #[tokio::test]
    async fn full_clone_records_call() {
        let runner = SpyGitRunner::default();
        let dest = PathBuf::from("/tmp/full");
        runner
            .full_clone("https://github.com/octocat/Hello-World.git", &dest, &opts())
            .await
            .expect("full clone");

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "full_clone");
    }

    #[tokio::test]
    async fn shallow_clone_records_call() {
        let runner = SpyGitRunner::default();
        let dest = PathBuf::from("/tmp/shallow.git");
        runner
            .shallow_clone(
                "https://github.com/octocat/Hello-World.git",
                &dest,
                &opts(),
                10,
            )
            .await
            .expect("shallow clone");

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "shallow_clone");
    }

    #[tokio::test]
    async fn lfs_clone_records_call() {
        let runner = SpyGitRunner::default();
        let dest = PathBuf::from("/tmp/lfs.git");
        runner
            .lfs_clone("https://github.com/octocat/Hello-World.git", &dest, &opts())
            .await
            .expect("lfs clone");

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "lfs_clone");
    }

    #[tokio::test]
    async fn push_mirror_records_call() {
        let runner = SpyGitRunner::default();
        let src = PathBuf::from("/tmp/local.git");
        runner
            .push_mirror(&src, "https://gitea.example.com/user/repo.git", &opts())
            .await
            .expect("push mirror");

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "push_mirror");
    }

    #[tokio::test]
    async fn multiple_calls_all_recorded() {
        let runner = SpyGitRunner::default();
        let dest = PathBuf::from("/tmp/repo.git");
        runner
            .mirror_clone("https://github.com/a/b.git", &dest, &opts())
            .await
            .unwrap();
        runner
            .bare_clone("https://github.com/c/d.git", &dest, &opts())
            .await
            .unwrap();
        assert_eq!(runner.recorded_calls().len(), 2);
    }

    #[tokio::test]
    async fn injected_failures_apply_only_to_matching_urls() {
        let runner = SpyGitRunner::default().failing_when_url_contains(
            "/bad.git",
            SpyFailure::Git {
                code: 128,
                stderr: "fatal: boom".into(),
            },
        );
        let dest = PathBuf::from("/tmp/x.git");
        runner
            .mirror_clone("https://h/good.git", &dest, &opts())
            .await
            .expect("non-matching URL succeeds");
        let err = runner
            .mirror_clone("https://h/bad.git", &dest, &opts())
            .await
            .expect_err("matching URL fails");
        assert!(
            matches!(err, CoreError::GitFailed { code: 128, .. }),
            "{err:?}"
        );
        assert_eq!(
            runner.recorded_calls().len(),
            2,
            "failed calls are recorded too"
        );

        let cancelled =
            SpyGitRunner::default().failing_when_url_contains("", SpyFailure::Interrupted);
        assert!(matches!(
            cancelled
                .push_mirror(&dest, "https://h/any.git", &opts())
                .await,
            Err(CoreError::Interrupted)
        ));
    }
}
