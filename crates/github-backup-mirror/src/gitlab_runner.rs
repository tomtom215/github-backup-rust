// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Push-mirror runner for GitLab destinations.
//!
//! Discovers local bare git repositories and mirrors them to a GitLab instance
//! by pushing branches and tags (see [`crate::push`]); the token is never
//! exposed in process listings or written to disk.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tracing::{error, info, warn};

use crate::config::GitLabConfig;
use crate::error::MirrorError;
use crate::gitlab_client::GitLabClient;
use crate::push::{push_refs, wants_private};
use crate::runner::MirrorStats;

/// Discovers all bare git repositories under `repos_dir` and pushes each one
/// as a mirror to the configured GitLab destination.
///
/// For each `*.git` directory found directly under `repos_dir`:
/// 1. Extract the repository name (strip the `.git` suffix).
/// 2. Ensure the project exists on GitLab (creates it if not).
/// 3. Push branches and tags (pruning deleted ones) from the local bare clone.
///
/// A project is created private unless the user forced `private` or the source
/// is listed in `public_repos`.  An existing project that this tool did not
/// create is never pushed into.  Per-repository errors are logged and collected
/// in [`MirrorStats::failures`]; the function continues with the remaining
/// repositories.
///
/// # Errors
///
/// Returns [`MirrorError`] only on fatal errors (e.g. configuration problems).
/// Per-repo push failures are logged and counted in `errored`.
pub async fn push_mirrors_gitlab(
    client: &GitLabClient,
    config: &GitLabConfig,
    repos_dir: &Path,
    description_prefix: &str,
    public_repos: &HashSet<String>,
) -> Result<MirrorStats, MirrorError> {
    let mut stats = MirrorStats::default();

    let repos = discover_git_repos(repos_dir);
    if repos.is_empty() {
        info!(dir = %repos_dir.display(), "no git repositories found to mirror to GitLab");
        return Ok(stats);
    }

    info!(
        count = repos.len(),
        dest = %config.base_url,
        "pushing mirrors to GitLab"
    );

    for (repo_path, repo_name) in &repos {
        let description = format!("{description_prefix}{repo_name}");
        let private = wants_private(config.private, public_repos, repo_name);
        match push_one_mirror_gitlab(client, config, repo_path, repo_name, &description, private)
            .await
        {
            Ok(()) => {
                stats.pushed += 1;
                info!(repo = %repo_name, "GitLab mirror pushed successfully");
            }
            Err(e) => {
                stats.errored += 1;
                stats.failures.push(format!("{repo_name}: {e}"));
                warn!(repo = %repo_name, error = %e, "GitLab mirror push failed, continuing");
            }
        }
    }

    Ok(stats)
}

/// Pushes a single repository to the GitLab mirror.
async fn push_one_mirror_gitlab(
    client: &GitLabClient,
    config: &GitLabConfig,
    repo_path: &Path,
    repo_name: &str,
    description: &str,
    private: bool,
) -> Result<(), MirrorError> {
    client
        .ensure_repo_exists(repo_name, description, private)
        .await?;

    let remote_url = config.repo_clone_url(repo_name);

    info!(
        repo = %repo_name,
        remote = %config.base_url,
        "pushing branches and tags to GitLab"
    );

    push_refs(repo_path, &remote_url, &config.token, "oauth2")
}

/// Discovers all `*.git` directories directly under `dir`.
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
pub async fn with_retry_gitlab<F, Fut, T, E>(
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
        fs::create_dir(dir.path().join("not-a-repo")).unwrap();
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
}
