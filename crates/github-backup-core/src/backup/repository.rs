// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository git clone / mirror backup.

use std::path::Path;

use tracing::info;

use github_backup_types::{config::BackupOptions, Repository};

use crate::{error::CoreError, git::GitRunner, storage::Storage};

/// Backs up a single repository by writing its metadata JSON and performing a
/// bare mirror clone (or update if already cloned).
///
/// # Arguments
///
/// - `client`  – authenticated GitHub API client
/// - `repo`    – repository metadata already fetched from the API
/// - `opts`    – which backup categories are enabled
/// - `repos_dir` – parent directory for bare git clones
/// - `meta_dir`  – parent directory for per-repository JSON metadata
/// - `storage` – storage backend
/// - `git`     – git runner for subprocess operations
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

    // Clone / update the bare mirror.
    if opts.repositories {
        let dest = repos_dir.join(format!("{}.git", repo.name));
        let clone_url = if opts.prefer_ssh {
            &repo.ssh_url
        } else {
            &repo.clone_url
        };

        if opts.lfs {
            git.lfs_clone(clone_url, &dest)?;
        } else {
            git.mirror_clone(clone_url, &dest)?;
        }
    }

    Ok(())
}

/// Returns `true` if `repo` should be included given `opts`.
///
/// Does not modify state; useful for filtering lists before issuing API calls.
#[must_use]
pub fn should_include(repo: &Repository, opts: &BackupOptions) -> bool {
    if repo.fork && !opts.forks {
        return false;
    }
    if repo.private && !opts.private {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::SpyGitRunner;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::{config::BackupOptions, user::User, Repository};
    use std::path::PathBuf;

    fn make_repo(name: &str, private: bool, fork: bool) -> Repository {
        Repository {
            id: 1,
            full_name: format!("octocat/{name}"),
            name: name.to_string(),
            owner: User {
                id: 1,
                login: "octocat".to_string(),
                user_type: "User".to_string(),
                avatar_url: "https://github.com/images/error/octocat_happy.gif".to_string(),
                html_url: "https://github.com/octocat".to_string(),
            },
            private,
            fork,
            archived: false,
            disabled: false,
            description: None,
            clone_url: format!("https://github.com/octocat/{name}.git"),
            ssh_url: format!("git@github.com:octocat/{name}.git"),
            default_branch: "main".to_string(),
            size: 0,
            has_issues: true,
            has_wiki: false,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            pushed_at: None,
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            html_url: format!("https://github.com/octocat/{name}"),
        }
    }

    #[tokio::test]
    async fn backup_repository_writes_info_json() {
        let repo = make_repo("Hello-World", false, false);
        let opts = BackupOptions {
            repositories: true,
            ..Default::default()
        };
        let storage = MemStorage::default();
        let git = SpyGitRunner::default();

        backup_repository(
            &repo,
            &opts,
            &PathBuf::from("/backup/git/repos"),
            &PathBuf::from("/backup/json/repos/Hello-World"),
            &storage,
            &git,
        )
        .await
        .expect("backup");

        let info_path = PathBuf::from("/backup/json/repos/Hello-World/info.json");
        assert!(
            storage.get(&info_path).is_some(),
            "info.json should be written"
        );
    }

    #[tokio::test]
    async fn backup_repository_clones_when_repositories_enabled() {
        let repo = make_repo("Hello-World", false, false);
        let opts = BackupOptions {
            repositories: true,
            ..Default::default()
        };
        let storage = MemStorage::default();
        let git = SpyGitRunner::default();

        backup_repository(
            &repo,
            &opts,
            &PathBuf::from("/backup/git/repos"),
            &PathBuf::from("/backup/json/repos/Hello-World"),
            &storage,
            &git,
        )
        .await
        .expect("backup");

        let calls = git.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "mirror_clone");
    }

    #[tokio::test]
    async fn backup_repository_skips_fork_when_forks_disabled() {
        let repo = make_repo("forked-repo", false, true);
        let opts = BackupOptions {
            repositories: true,
            forks: false,
            ..Default::default()
        };
        let storage = MemStorage::default();
        let git = SpyGitRunner::default();

        backup_repository(
            &repo,
            &opts,
            &PathBuf::from("/backup/git/repos"),
            &PathBuf::from("/backup/json/repos/forked-repo"),
            &storage,
            &git,
        )
        .await
        .expect("backup");

        // No git calls and no files written for skipped fork.
        assert_eq!(git.recorded_calls().len(), 0);
        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_repository_skips_private_when_private_disabled() {
        let repo = make_repo("secret", true, false);
        let opts = BackupOptions {
            repositories: true,
            private: false,
            ..Default::default()
        };
        let storage = MemStorage::default();
        let git = SpyGitRunner::default();

        backup_repository(
            &repo,
            &opts,
            &PathBuf::from("/backup/git/repos"),
            &PathBuf::from("/backup/json/repos/secret"),
            &storage,
            &git,
        )
        .await
        .expect("backup");

        assert_eq!(git.recorded_calls().len(), 0);
        assert_eq!(storage.len(), 0);
    }

    #[test]
    fn should_include_returns_false_for_fork_when_disabled() {
        let repo = make_repo("fork", false, true);
        let opts = BackupOptions {
            forks: false,
            ..Default::default()
        };
        assert!(!should_include(&repo, &opts));
    }

    #[test]
    fn should_include_returns_true_for_fork_when_enabled() {
        let repo = make_repo("fork", false, true);
        let opts = BackupOptions {
            forks: true,
            ..Default::default()
        };
        assert!(should_include(&repo, &opts));
    }
}
