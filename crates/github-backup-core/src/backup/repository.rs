// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository git clone / mirror backup.

use std::path::Path;

use tracing::info;

use github_backup_types::{
    config::{glob_match, BackupOptions, CloneType},
    Repository,
};

use url::Url;

use crate::{
    error::CoreError,
    git::{CloneOptions, GitRunner},
    storage::Storage,
};

/// Backs up a single repository by writing its metadata JSON and performing a
/// git clone (using the mode selected by `opts.clone_type`).
///
/// # Errors
///
/// Propagates [`CoreError`] from storage writes or git operations.
pub async fn backup_repository(
    repo: &Repository,
    opts: &BackupOptions,
    repos_dir: &Path,
    meta_dir: &Path,
    storage: &impl Storage,
    git: &impl GitRunner,
    clone_opts: &CloneOptions,
) -> Result<(), CoreError> {
    // Skip forks if not requested.
    if repo.fork && !opts.forks {
        info!(repo = %repo.full_name, "skipping fork");
        return Ok(());
    }

    // Skip private repos if not requested.
    if repo.private && !opts.private {
        info!(repo = %repo.full_name, "skipping private repository");
        return Ok(());
    }

    // Write repository metadata JSON.
    let meta_path = meta_dir.join("info.json");
    storage.write_json(&meta_path, repo)?;

    // Clone / update the repository using the configured clone strategy.
    if opts.repositories {
        clone_repo(repo, opts, repos_dir, git, clone_opts)?;
    }

    Ok(())
}

/// Rewrites the hostname in a URL, returning the modified URL string.
/// Exported as `pub(crate)` so that sibling backup modules (e.g. `wiki`) can
/// apply the same `--clone-host` override without duplicating this logic.
///
/// Used to support GHES deployments where the API host and clone host differ.
/// Returns the original URL unchanged if parsing or rewriting fails.
pub(crate) fn rewrite_host(url: &str, new_host: &str) -> String {
    // Handle ssh:// URLs and git@ URLs differently from HTTPS.
    // For git@host:path syntax we do a simple prefix replacement.
    if let Some(rest) = url.strip_prefix("git@") {
        // git@<host>:<path>  →  git@<new_host>:<path>
        if let Some(colon_pos) = rest.find(':') {
            return format!("git@{}:{}", new_host, &rest[colon_pos + 1..]);
        }
        return url.to_string();
    }
    // HTTPS / SSH URLs: parse and replace host.
    match Url::parse(url) {
        Ok(mut parsed) => {
            if parsed.set_host(Some(new_host)).is_ok() {
                parsed.to_string()
            } else {
                url.to_string()
            }
        }
        Err(_) => url.to_string(),
    }
}

/// Performs the git clone / update for a repository, dispatching on
/// [`BackupOptions::clone_type`] and [`BackupOptions::lfs`].
fn clone_repo(
    repo: &Repository,
    opts: &BackupOptions,
    repos_dir: &Path,
    git: &impl GitRunner,
    clone_opts: &CloneOptions,
) -> Result<(), CoreError> {
    let raw_clone_url = if opts.prefer_ssh {
        &repo.ssh_url
    } else {
        &repo.clone_url
    };

    // Apply --clone-host override (GHES split-hostname deployments).
    let rewritten;
    let clone_url: &str = if let Some(ref host) = opts.clone_host {
        rewritten = rewrite_host(raw_clone_url, host);
        &rewritten
    } else {
        raw_clone_url
    };

    if opts.lfs {
        // LFS cloning is independent of clone_type.
        let dest = repos_dir.join(format!("{}.git", repo.name));
        return git.lfs_clone(clone_url, &dest, clone_opts);
    }

    match &opts.clone_type {
        CloneType::Mirror => {
            let dest = repos_dir.join(format!("{}.git", repo.name));
            git.mirror_clone(clone_url, &dest, clone_opts)
        }
        CloneType::Bare => {
            let dest = repos_dir.join(format!("{}.git", repo.name));
            git.bare_clone(clone_url, &dest, clone_opts)
        }
        CloneType::Full => {
            // Full clones go in a directory without a `.git` suffix so they
            // look like normal working trees.
            let dest = repos_dir.join(&repo.name);
            git.full_clone(clone_url, &dest, clone_opts)
        }
        CloneType::Shallow(depth) => {
            let dest = repos_dir.join(format!("{}.git", repo.name));
            git.shallow_clone(clone_url, &dest, clone_opts, *depth)
        }
    }
}

/// Returns `true` if `repo` should be included given `opts`.
///
/// Checks fork/private visibility flags, then applies any
/// [`BackupOptions::include_repos`] and [`BackupOptions::exclude_repos`]
/// glob-pattern filters.  Does not modify state.
#[must_use]
pub fn should_include(repo: &Repository, opts: &BackupOptions) -> bool {
    if repo.fork && !opts.forks {
        return false;
    }
    if repo.private && !opts.private {
        return false;
    }

    // Include filter: if patterns are specified, the repo name must match
    // at least one of them.
    if !opts.include_repos.is_empty()
        && !opts.include_repos.iter().any(|p| glob_match(p, &repo.name))
    {
        return false;
    }

    // Exclude filter: repo name must NOT match any of these patterns.
    if opts.exclude_repos.iter().any(|p| glob_match(p, &repo.name)) {
        return false;
    }

    true
}

#[cfg(test)]
#[path = "repository_tests.rs"]
mod tests;
