// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Push-mirror runner for GitLab destinations.
//!
//! Discovers local bare git repositories and mirrors them to a GitLab instance
//! using `git push --mirror`.  Uses the same `AskpassScript` guard used by the
//! Gitea runner so the token is never exposed in process listings.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use tracing::{error, info, warn};

use crate::config::GitLabConfig;
use crate::error::MirrorError;
use crate::gitlab_client::GitLabClient;
use crate::runner::MirrorStats;

/// Discovers all bare git repositories under `repos_dir` and pushes each one
/// as a mirror to the configured GitLab destination.
///
/// For each `*.git` directory found directly under `repos_dir`:
/// 1. Extract the repository name (strip the `.git` suffix).
/// 2. Ensure the project exists on GitLab (creates it if not).
/// 3. Run `git push --mirror <remote_url>` from the local bare clone.
///
/// Per-repository errors are logged as warnings; the function continues with
/// remaining repositories.
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
        match push_one_mirror_gitlab(client, config, repo_path, repo_name, &description).await {
            Ok(()) => {
                stats.pushed += 1;
                info!(repo = %repo_name, "GitLab mirror pushed successfully");
            }
            Err(e) => {
                stats.errored += 1;
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
) -> Result<(), MirrorError> {
    client.ensure_repo_exists(repo_name, description).await?;

    let remote_url = config.repo_clone_url(repo_name);

    info!(
        repo = %repo_name,
        remote = %config.base_url,
        "running git push --mirror to GitLab"
    );

    run_git_push_mirror(repo_path, &remote_url, &config.token)
}

/// Runs `git push --mirror <remote_url>` from `repo_path`.
///
/// Injects the token via `GIT_ASKPASS` to keep it out of process listings.
fn run_git_push_mirror(repo_path: &Path, remote_url: &str, token: &str) -> Result<(), MirrorError> {
    let askpass = GitLabAskpassScript::create(token);

    let mut cmd = Command::new("git");
    cmd.args([
        "-C",
        &repo_path.to_string_lossy(),
        "push",
        "--mirror",
        remote_url,
    ]);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GIT_USERNAME", "oauth2");

    if let Some(ref script) = askpass {
        cmd.env("GIT_ASKPASS", script.path());
    }

    let output = cmd.output().map_err(MirrorError::GitSpawn)?;

    if output.status.success() {
        return Ok(());
    }

    let code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Err(MirrorError::GitFailed {
        args: format!("push --mirror {remote_url}"),
        code,
        stderr,
    })
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

/// RAII guard for a temporary `GIT_ASKPASS` shell script (GitLab variant).
struct GitLabAskpassScript {
    path: PathBuf,
}

impl GitLabAskpassScript {
    fn create(token: &str) -> Option<Self> {
        let script = format!("#!/bin/sh\necho '{}'", token.replace('\'', "'\\''"));
        let mut path = std::env::temp_dir();
        path.push(format!(
            "gh-mirror-gl-askpass-{}-{}.sh",
            std::process::id(),
            format!("{:?}", std::thread::current().id())
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>(),
        ));

        if std::fs::write(&path, script.as_bytes()).is_err() {
            return None;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
        }

        Some(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for GitLabAskpassScript {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
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
        assert!(!names.contains(&"not-a-repo"), "should ignore non-.git dirs");
        assert_eq!(repos.len(), 2);
    }

    #[test]
    fn discover_git_repos_returns_empty_for_missing_dir() {
        let repos = discover_git_repos(Path::new("/nonexistent/path/that/does/not/exist"));
        assert!(repos.is_empty());
    }
}
