// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Push-mirror runner: discovers local git repos and mirrors them to a remote
//! Git host (Gitea, Codeberg, Forgejo, etc.).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tracing::{error, info, warn};

use crate::config::GiteaConfig;
use crate::error::MirrorError;
use crate::push::{push_refs, wants_private};

/// Discovers all bare git repositories under `repos_dir` and pushes each
/// one as a mirror to the configured Gitea destination.
///
/// # Algorithm
///
/// For each `*.git` directory found directly under `repos_dir`:
/// 1. Extract the repository name (strip the `.git` suffix).
/// 2. Ensure the repository exists on the Gitea instance (creates it if not).
/// 3. Push branches and tags (pruning deleted ones) from the local bare clone.
///
/// A repository is created private unless the user forced `private` or the
/// source is listed in `public_repos`.  An existing destination repository
/// that this tool did not create (its description is not `description_prefix`
/// plus the name) is never pushed into.
///
/// Errors per repository are logged as warnings and collected in
/// [`MirrorStats::failures`]; the function continues with the remaining
/// repositories, and the caller decides how a non-empty failure list ends the
/// run.
///
/// # Errors
///
/// Returns [`MirrorError`] only on fatal errors (e.g. configuration problems).
/// Per-repo push failures are logged and counted in `errored`.
pub async fn push_mirrors(
    client: &crate::client::GiteaClient,
    config: &GiteaConfig,
    repos_dir: &Path,
    description_prefix: &str,
    public_repos: &HashSet<String>,
) -> Result<MirrorStats, MirrorError> {
    let mut stats = MirrorStats::default();

    let repos = discover_git_repos(repos_dir);
    if repos.is_empty() {
        info!(dir = %repos_dir.display(), "no git repositories found to mirror");
        return Ok(stats);
    }

    info!(
        count = repos.len(),
        dest = %config.base_url,
        "pushing mirrors to remote"
    );

    for (repo_path, repo_name) in &repos {
        let description = format!("{description_prefix}{repo_name}");
        let private = wants_private(config.private, public_repos, repo_name);
        match push_one_mirror(client, config, repo_path, repo_name, &description, private).await {
            Ok(()) => {
                stats.pushed += 1;
                info!(repo = %repo_name, "mirror pushed successfully");
            }
            Err(e) => {
                stats.errored += 1;
                stats.failures.push(format!("{repo_name}: {e}"));
                warn!(repo = %repo_name, error = %e, "mirror push failed, continuing");
            }
        }
    }

    Ok(stats)
}

/// Statistics from a mirror push run.
#[derive(Debug, Default, Clone)]
pub struct MirrorStats {
    /// Number of repositories successfully pushed.
    pub pushed: usize,
    /// Number of repositories that failed to push.
    pub errored: usize,
    /// One `name: error` line per failed repository.
    pub failures: Vec<String>,
}

/// Pushes a single repository to the Gitea mirror.
async fn push_one_mirror(
    client: &crate::client::GiteaClient,
    config: &GiteaConfig,
    repo_path: &Path,
    repo_name: &str,
    description: &str,
    private: bool,
) -> Result<(), MirrorError> {
    // Ensure the destination repo exists (creates it if needed) and is ours.
    client
        .ensure_repo_exists(repo_name, description, private)
        .await?;

    let remote_url = config.repo_clone_url(repo_name);

    info!(
        repo = %repo_name,
        remote = %config.base_url,
        "pushing branches and tags"
    );

    push_refs(repo_path, &remote_url, &config.token, "x-access-token")
}

/// Discovers all `*.git` directories directly under `dir`.
///
/// Returns a list of `(path, repo_name)` pairs where `repo_name` is the
/// directory name with the `.git` suffix stripped.
fn discover_git_repos(dir: &Path) -> Vec<(PathBuf, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }
            let name = path.file_name()?.to_string_lossy().into_owned();
            let repo_name = name.strip_suffix(".git")?.to_string();
            Some((path, repo_name))
        })
        .collect()
}

/// Retries an async operation up to `max_attempts` times with exponential
/// back-off, starting at `base_delay`.
///
/// Returns the result of the last attempt.
pub async fn with_retry<F, Fut, T, E>(
    max_attempts: u32,
    base_delay: Duration,
    mut op: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut last_err = None;
    for attempt in 0..max_attempts {
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if attempt + 1 < max_attempts {
                    let delay = base_delay * 2u32.pow(attempt);
                    error!(attempt = attempt + 1, max = max_attempts, delay_ms = delay.as_millis(), error = %e, "retrying after error");
                    tokio::time::sleep(delay).await;
                }
                last_err = Some(e);
            }
        }
    }
    Err(last_err.expect("max_attempts > 0"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn discover_git_repos_finds_dot_git_directories() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("my-repo.git")).unwrap();
        fs::create_dir(dir.path().join("another.git")).unwrap();
        // A directory without .git should be ignored.
        fs::create_dir(dir.path().join("not-a-repo")).unwrap();
        // A file with .git should be ignored (must be dir).
        fs::write(dir.path().join("file.git"), b"").unwrap();

        let repos = discover_git_repos(dir.path());
        let names: Vec<&str> = repos.iter().map(|(_, n)| n.as_str()).collect();
        assert!(names.contains(&"my-repo"), "should find my-repo");
        assert!(names.contains(&"another"), "should find another");
        assert!(
            !names.contains(&"not-a-repo"),
            "should ignore non-.git dirs"
        );
        assert_eq!(repos.len(), 2);
    }

    #[test]
    fn discover_git_repos_returns_empty_for_missing_dir() {
        let repos = discover_git_repos(Path::new("/nonexistent/path/that/does/not/exist"));
        assert!(repos.is_empty());
    }

    #[tokio::test]
    async fn with_retry_succeeds_on_first_attempt() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c = calls.clone();
        let result = with_retry(3, Duration::from_millis(1), || {
            let c = c.clone();
            async move {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok::<u32, String>(42)
            }
        })
        .await;
        assert_eq!(result.unwrap(), 42);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn with_retry_retries_on_error() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c = calls.clone();
        let result = with_retry(3, Duration::from_millis(1), || {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n < 2 {
                    Err("not yet".to_string())
                } else {
                    Ok(n)
                }
            }
        })
        .await;
        assert!(result.is_ok());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }
}
