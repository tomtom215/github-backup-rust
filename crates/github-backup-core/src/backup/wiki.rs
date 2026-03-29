// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Wiki git clone backup.

use std::path::Path;

use tracing::info;

use github_backup_types::{config::BackupOptions, Repository};

use crate::{
    backup::repository::rewrite_host,
    error::CoreError,
    git::{CloneOptions, GitRunner},
};

/// Backs up a repository's wiki as a bare git mirror.
///
/// GitHub wiki URLs follow the pattern
/// `https://github.com/<owner>/<repo>.wiki.git`. This function attempts to
/// clone only when the repository has `has_wiki == true`; GitHub will return
/// a 128 error code when the wiki has no content, which is treated as a
/// non-fatal warning rather than an error.
///
/// # Errors
///
/// Returns [`CoreError::GitFailed`] only for unexpected git failures; a wiki
/// that exists but has no commits is silently skipped.
pub async fn backup_wiki(
    repo: &Repository,
    opts: &BackupOptions,
    wikis_dir: &Path,
    git: &impl GitRunner,
    clone_opts: &CloneOptions,
) -> Result<(), CoreError> {
    if !opts.wikis || !repo.has_wiki {
        return Ok(());
    }

    let raw_wiki_url = format!("{}.wiki.git", repo.clone_url.trim_end_matches(".git"));
    let rewritten;
    let wiki_url: &str = if let Some(ref host) = opts.clone_host {
        rewritten = rewrite_host(&raw_wiki_url, host);
        &rewritten
    } else {
        &raw_wiki_url
    };
    let dest = wikis_dir.join(format!("{}.wiki.git", repo.name));

    info!(repo = %repo.full_name, dest = %dest.display(), "cloning wiki");

    match git.mirror_clone(wiki_url, &dest, clone_opts) {
        Ok(()) => Ok(()),
        Err(CoreError::GitFailed { code: 128, .. }) => {
            // Code 128 is returned when the wiki exists but is empty.
            info!(repo = %repo.full_name, "wiki is empty or has no commits, skipping");
            Ok(())
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{test_support::SpyGitRunner, CloneOptions};
    use github_backup_types::{config::BackupOptions, user::User, Repository};
    use std::path::PathBuf;

    fn make_repo(has_wiki: bool) -> Repository {
        Repository {
            id: 1,
            full_name: "octocat/Hello-World".to_string(),
            name: "Hello-World".to_string(),
            owner: User {
                id: 1,
                login: "octocat".to_string(),
                user_type: "User".to_string(),
                avatar_url: "https://github.com/images/error/octocat_happy.gif".to_string(),
                html_url: "https://github.com/octocat".to_string(),
            },
            private: false,
            fork: false,
            archived: false,
            disabled: false,
            description: None,
            clone_url: "https://github.com/octocat/Hello-World.git".to_string(),
            ssh_url: "git@github.com:octocat/Hello-World.git".to_string(),
            default_branch: "main".to_string(),
            size: 0,
            has_issues: true,
            has_wiki,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            pushed_at: None,
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            html_url: "https://github.com/octocat/Hello-World".to_string(),
        }
    }

    #[tokio::test]
    async fn backup_wiki_clones_when_wiki_enabled_and_has_wiki() {
        let repo = make_repo(true);
        let opts = BackupOptions {
            wikis: true,
            ..Default::default()
        };
        let git = SpyGitRunner::default();

        backup_wiki(
            &repo,
            &opts,
            &PathBuf::from("/backup/git/wikis"),
            &git,
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("wiki backup");

        let calls = git.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "mirror_clone");
        assert!(
            calls[0].url.ends_with(".wiki.git"),
            "wiki URL should end with .wiki.git"
        );
    }

    #[tokio::test]
    async fn backup_wiki_skips_when_has_wiki_false() {
        let repo = make_repo(false);
        let opts = BackupOptions {
            wikis: true,
            ..Default::default()
        };
        let git = SpyGitRunner::default();

        backup_wiki(
            &repo,
            &opts,
            &PathBuf::from("/backup/git/wikis"),
            &git,
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("wiki backup");

        assert_eq!(git.recorded_calls().len(), 0);
    }

    #[tokio::test]
    async fn backup_wiki_skips_when_wikis_opt_disabled() {
        let repo = make_repo(true);
        let opts = BackupOptions {
            wikis: false,
            ..Default::default()
        };
        let git = SpyGitRunner::default();

        backup_wiki(
            &repo,
            &opts,
            &PathBuf::from("/backup/git/wikis"),
            &git,
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("wiki backup");

        assert_eq!(git.recorded_calls().len(), 0);
    }
}
