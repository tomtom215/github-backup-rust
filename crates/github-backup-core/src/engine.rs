// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! The top-level [`BackupEngine`] that orchestrates all backup categories.

use tracing::{error, info};

use github_backup_client::GitHubClient;
use github_backup_types::config::{BackupOptions, OutputConfig};

use crate::{
    backup::{
        gist::backup_gists, issue::backup_issues, pull_request::backup_pull_requests,
        release::backup_releases, repository::backup_repository, user_data::backup_user_data,
        wiki::backup_wiki,
    },
    error::CoreError,
    git::GitRunner,
    storage::Storage,
};

/// Orchestrates a complete backup of a single GitHub owner (user or org).
///
/// The engine is intentionally not object-safe; it is generic over
/// [`Storage`] and [`GitRunner`] to enable zero-cost, compile-time dispatch
/// in the common path while remaining fully testable with stub implementations.
///
/// # Example
///
/// ```no_run
/// use github_backup_core::{BackupEngine, FsStorage, ProcessGitRunner};
/// use github_backup_client::GitHubClient;
/// use github_backup_types::config::{BackupOptions, Credential, OutputConfig};
///
/// # async fn example() -> Result<(), github_backup_core::CoreError> {
/// let cred = Credential::Token("ghp_xxx".to_string());
/// let client = GitHubClient::new(cred)?;
/// let storage = FsStorage::new();
/// let git = ProcessGitRunner::new();
/// let out = OutputConfig::new("/var/backup/github");
/// let opts = BackupOptions::all();
///
/// let engine = BackupEngine::new(client, storage, git, out, opts);
/// engine.run("octocat").await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct BackupEngine<S, G> {
    client: GitHubClient,
    storage: S,
    git: G,
    output: OutputConfig,
    opts: BackupOptions,
}

impl<S, G> BackupEngine<S, G>
where
    S: Storage,
    G: GitRunner,
{
    /// Creates a new [`BackupEngine`].
    #[must_use]
    pub fn new(
        client: GitHubClient,
        storage: S,
        git: G,
        output: OutputConfig,
        opts: BackupOptions,
    ) -> Self {
        Self {
            client,
            storage,
            git,
            output,
            opts,
        }
    }

    /// Runs the full backup for `owner`.
    ///
    /// Fetches repositories from the API, then for each qualifying repository
    /// runs the enabled backup categories in sequence. Per-repository errors
    /// are logged but do not abort the entire run.
    ///
    /// # Errors
    ///
    /// Returns the first fatal error (e.g. authentication failure on the
    /// repository list call). Per-repository failures are treated as warnings.
    pub async fn run(&self, owner: &str) -> Result<(), CoreError> {
        info!(owner, "starting backup");

        // ── User-level data ────────────────────────────────────────────────
        let owner_json_dir = self.output.owner_json(owner, "");
        backup_user_data(
            &self.client,
            owner,
            &self.opts,
            std::path::Path::new(owner_json_dir.to_str().unwrap_or("")),
            &self.storage,
        )
        .await?;

        // ── Gists ──────────────────────────────────────────────────────────
        backup_gists(
            &self.client,
            owner,
            &self.opts,
            &self.output.gists_git_dir(owner),
            &self.output.gists_meta_dir(owner),
            &self.storage,
            &self.git,
        )
        .await?;

        // ── Repositories ───────────────────────────────────────────────────
        let repos = self.client.list_user_repos(owner).await?;
        info!(owner, count = repos.len(), "fetched repository list");

        for repo in &repos {
            if let Err(e) = self.backup_one_repo(owner, repo).await {
                error!(
                    repo = %repo.full_name,
                    error = %e,
                    "repository backup failed, continuing"
                );
            }
        }

        info!(owner, "backup complete");
        Ok(())
    }

    /// Backs up a single repository: git clone + all enabled metadata.
    async fn backup_one_repo(
        &self,
        owner: &str,
        repo: &github_backup_types::Repository,
    ) -> Result<(), CoreError> {
        use crate::backup::repository::should_include;

        if !should_include(repo, &self.opts) {
            return Ok(());
        }

        let repos_dir = self.output.repos_dir(owner);
        let wikis_dir = self.output.wikis_dir(owner);
        let meta_dir = self.output.repo_meta_dir(owner, &repo.name);

        // Repository git mirror.
        backup_repository(
            repo,
            &self.opts,
            &repos_dir,
            &meta_dir,
            &self.storage,
            &self.git,
        )
        .await?;

        // Wiki git mirror.
        backup_wiki(repo, &self.opts, &wikis_dir, &self.git).await?;

        // Repository metadata.
        self.backup_repo_metadata(owner, repo, &meta_dir).await?;

        Ok(())
    }

    /// Backs up all enabled JSON metadata for a repository.
    async fn backup_repo_metadata(
        &self,
        owner: &str,
        repo: &github_backup_types::Repository,
        meta_dir: &std::path::Path,
    ) -> Result<(), CoreError> {
        // Issues (+ comments + events)
        backup_issues(
            &self.client,
            owner,
            &repo.name,
            &self.opts,
            meta_dir,
            &self.storage,
        )
        .await?;

        // Pull requests (+ comments + commits + reviews)
        backup_pull_requests(
            &self.client,
            owner,
            &repo.name,
            &self.opts,
            meta_dir,
            &self.storage,
        )
        .await?;

        // Releases (+ asset downloads)
        backup_releases(
            &self.client,
            owner,
            &repo.name,
            &self.opts,
            meta_dir,
            &self.storage,
        )
        .await?;

        // Labels
        if self.opts.labels {
            let labels = self.client.list_labels(owner, &repo.name).await?;
            self.storage
                .write_json(&meta_dir.join("labels.json"), &labels)?;
        }

        // Milestones
        if self.opts.milestones {
            let milestones = self.client.list_milestones(owner, &repo.name).await?;
            self.storage
                .write_json(&meta_dir.join("milestones.json"), &milestones)?;
        }

        // Hooks
        if self.opts.hooks {
            match self.client.list_hooks(owner, &repo.name).await {
                Ok(hooks) => {
                    self.storage
                        .write_json(&meta_dir.join("hooks.json"), &hooks)?;
                }
                Err(github_backup_client::ClientError::ApiError { status: 404, .. }) => {
                    // Hooks require admin access; skip silently if insufficient.
                    info!(repo = %repo.full_name, "skipping hooks (no admin access)");
                }
                Err(e) => return Err(e.into()),
            }
        }

        // Security advisories
        if self.opts.security_advisories {
            match self
                .client
                .list_security_advisories(owner, &repo.name)
                .await
            {
                Ok(advisories) => {
                    self.storage
                        .write_json(&meta_dir.join("security_advisories.json"), &advisories)?;
                }
                Err(github_backup_client::ClientError::ApiError {
                    status: 404 | 403, ..
                }) => {
                    info!(repo = %repo.full_name, "skipping security advisories (not available)");
                }
                Err(e) => return Err(e.into()),
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // Engine integration tests require a full mock API server. The engine
    // logic is validated indirectly through the per-category backup tests.
    // The compile-time generic constraints are verified by the existence of
    // this module.

    use super::*;
    use crate::git::test_support::SpyGitRunner;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::config::{BackupOptions, Credential, OutputConfig};

    fn make_engine() -> BackupEngine<MemStorage, SpyGitRunner> {
        let cred = Credential::Token("test".to_string());
        let client = GitHubClient::new(cred).expect("client");
        BackupEngine::new(
            client,
            MemStorage::default(),
            SpyGitRunner::default(),
            OutputConfig::new("/backup"),
            BackupOptions::default(),
        )
    }

    #[test]
    fn backup_engine_constructs_without_panic() {
        let _engine = make_engine();
    }
}
