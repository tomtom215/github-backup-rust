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
mod tests {
    use super::*;
    use crate::git::{test_support::SpyGitRunner, CloneOptions};
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
            &CloneOptions::unauthenticated(),
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
    async fn backup_repository_mirror_clone_by_default() {
        let repo = make_repo("Hello-World", false, false);
        let opts = BackupOptions {
            repositories: true,
            clone_type: CloneType::Mirror,
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
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("backup");

        let calls = git.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "mirror_clone");
    }

    #[tokio::test]
    async fn backup_repository_bare_clone_when_configured() {
        let repo = make_repo("Hello-World", false, false);
        let opts = BackupOptions {
            repositories: true,
            clone_type: CloneType::Bare,
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
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("backup");

        let calls = git.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "bare_clone");
    }

    #[tokio::test]
    async fn backup_repository_full_clone_uses_no_git_suffix() {
        let repo = make_repo("Hello-World", false, false);
        let opts = BackupOptions {
            repositories: true,
            clone_type: CloneType::Full,
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
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("backup");

        let calls = git.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "full_clone");
        // Full clones should NOT have the .git suffix.
        assert!(
            !calls[0].dest.to_string_lossy().ends_with(".git"),
            "full clone destination should not end with .git"
        );
    }

    #[tokio::test]
    async fn backup_repository_shallow_clone_when_configured() {
        let repo = make_repo("Hello-World", false, false);
        let opts = BackupOptions {
            repositories: true,
            clone_type: CloneType::Shallow(5),
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
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("backup");

        let calls = git.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "shallow_clone");
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
            &CloneOptions::unauthenticated(),
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
            &CloneOptions::unauthenticated(),
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

    #[test]
    fn should_include_include_filter_allows_matching_repo() {
        let repo = make_repo("rust-backup", false, false);
        let opts = BackupOptions {
            include_repos: vec!["rust-*".to_string()],
            ..Default::default()
        };
        assert!(should_include(&repo, &opts));
    }

    #[test]
    fn should_include_include_filter_blocks_non_matching_repo() {
        let repo = make_repo("python-tool", false, false);
        let opts = BackupOptions {
            include_repos: vec!["rust-*".to_string()],
            ..Default::default()
        };
        assert!(!should_include(&repo, &opts));
    }

    #[test]
    fn should_include_exclude_filter_blocks_matching_repo() {
        let repo = make_repo("archived-old-thing", false, false);
        let opts = BackupOptions {
            exclude_repos: vec!["archived-*".to_string()],
            ..Default::default()
        };
        assert!(!should_include(&repo, &opts));
    }

    #[test]
    fn should_include_exclude_filter_allows_non_matching_repo() {
        let repo = make_repo("live-project", false, false);
        let opts = BackupOptions {
            exclude_repos: vec!["archived-*".to_string()],
            ..Default::default()
        };
        assert!(should_include(&repo, &opts));
    }

    #[test]
    fn should_include_exclude_overrides_include() {
        let repo = make_repo("rust-archived", false, false);
        let opts = BackupOptions {
            include_repos: vec!["rust-*".to_string()],
            exclude_repos: vec!["*archived*".to_string()],
            ..Default::default()
        };
        assert!(!should_include(&repo, &opts));
    }

    #[test]
    fn should_include_empty_filters_includes_all() {
        let repo = make_repo("anything", false, false);
        let opts = BackupOptions::default();
        assert!(should_include(&repo, &opts));
    }

    // ── rewrite_host tests ────────────────────────────────────────────────

    #[test]
    fn rewrite_host_https_url() {
        let result = rewrite_host(
            "https://github.example.com/owner/repo.git",
            "git.example.com",
        );
        assert_eq!(result, "https://git.example.com/owner/repo.git");
    }

    #[test]
    fn rewrite_host_https_url_no_path() {
        let result = rewrite_host("https://github.example.com/", "git.example.com");
        assert_eq!(result, "https://git.example.com/");
    }

    #[test]
    fn rewrite_host_ssh_git_at_syntax() {
        let result = rewrite_host("git@github.example.com:owner/repo.git", "git.example.com");
        assert_eq!(result, "git@git.example.com:owner/repo.git");
    }

    #[test]
    fn rewrite_host_ssh_url_scheme() {
        let result = rewrite_host(
            "ssh://git@github.example.com/owner/repo.git",
            "other.example.com",
        );
        assert_eq!(result, "ssh://git@other.example.com/owner/repo.git");
    }

    #[test]
    fn rewrite_host_preserves_unknown_format() {
        // Malformed URL — should be returned unchanged.
        let url = "not-a-url";
        assert_eq!(rewrite_host(url, "host.example.com"), url);
    }

    #[tokio::test]
    async fn backup_repository_applies_clone_host_override() {
        let repo = make_repo("Hello-World", false, false);
        let opts = BackupOptions {
            repositories: true,
            clone_type: CloneType::Mirror,
            clone_host: Some("git.example.com".to_string()),
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
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("backup");

        let calls = git.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "mirror_clone");
        assert!(
            calls[0].url.contains("git.example.com"),
            "clone URL should have overridden host, got: {}",
            calls[0].url
        );
    }
}
