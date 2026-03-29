// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! The top-level [`BackupEngine`] that orchestrates all backup categories.

use std::sync::Arc;

use tokio::sync::Semaphore;
use tracing::{error, info, warn};

use github_backup_client::GitHubClient;
use github_backup_types::config::{BackupOptions, BackupTarget, OutputConfig};
use github_backup_types::Repository;

use crate::{
    backup::{
        actions::backup_actions, collaborators::backup_collaborators,
        deploy_keys::backup_deploy_keys, environments::backup_environments, gist::backup_gists,
        issue::backup_issues, pull_request::backup_pull_requests, release::backup_releases,
        repository::backup_repository, starred_repos::backup_starred_repos,
        user_data::backup_user_data, wiki::backup_wiki,
    },
    error::CoreError,
    git::{CloneOptions, GitRunner},
    stats::BackupStats,
    storage::Storage,
};

/// Orchestrates a complete backup of a single GitHub owner (user or org).
///
/// The engine is generic over [`Storage`] and [`GitRunner`] for zero-cost,
/// compile-time dispatch and full testability via stub implementations.
///
/// # Concurrency
///
/// Repository backups run in parallel up to `opts.concurrency`. Set it to `1`
/// for fully sequential operation. The API client, storage, and git runner must
/// all be `Send + Sync` (the production implementations satisfy this).
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
/// let stats = engine.run("octocat").await?;
/// println!("{stats}");
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
    S: Storage + Clone + 'static,
    G: GitRunner + Clone + 'static,
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
    /// - For user targets, fetches repositories via the user repos API.
    /// - For org targets, fetches repositories via the org repos API.
    ///
    /// Per-repository errors are logged as warnings but do not abort the run.
    /// Returns [`BackupStats`] with counters for everything that was processed.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError`] on fatal errors (auth, network, filesystem).
    pub async fn run(&self, owner: &str) -> Result<BackupStats, CoreError> {
        let stats = BackupStats::new();
        info!(owner, dry_run = self.opts.dry_run, "starting backup");

        if self.opts.dry_run {
            warn!("dry-run mode: no files will be written and no git commands will be run");
        }

        // ── User-level data ────────────────────────────────────────────────
        let owner_json_dir = self.output.owner_json_dir(owner);
        backup_user_data(
            &self.client,
            owner,
            &self.opts,
            &owner_json_dir,
            &self.storage,
        )
        .await?;

        // ── Starred repos clone (durable queue) ───────────────────────────
        let clone_opts = self.make_clone_opts();
        backup_starred_repos(
            &self.client,
            &self.git,
            owner,
            &self.opts,
            &self.output.starred_repos_dir(owner),
            &self.output.starred_queue_path(owner),
            &clone_opts,
        )
        .await?;

        // ── Gists ──────────────────────────────────────────────────────────
        let gist_count = backup_gists(
            &self.client,
            owner,
            &self.opts,
            &self.output.gists_git_dir(owner),
            &self.output.gists_meta_dir(owner),
            &self.storage,
            &self.git,
            &clone_opts,
        )
        .await?;
        for _ in 0..gist_count {
            stats.inc_gists();
        }

        // ── Repositories ───────────────────────────────────────────────────
        let repos = self.fetch_repos(owner).await?;
        info!(owner, count = repos.len(), "fetched repository list");
        stats.add_discovered(repos.len() as u64);

        self.backup_repos_concurrent(owner, repos, &stats).await;

        info!(owner, %stats, "backup complete");
        Ok(stats)
    }

    /// Fetches the repository list using the user or org API as appropriate.
    async fn fetch_repos(&self, owner: &str) -> Result<Vec<Repository>, CoreError> {
        match self.opts.target {
            BackupTarget::User => Ok(self.client.list_user_repos(owner).await?),
            BackupTarget::Org => Ok(self.client.list_org_repos(owner).await?),
        }
    }

    /// Backs up repositories concurrently, honouring `opts.concurrency`.
    async fn backup_repos_concurrent(
        &self,
        owner: &str,
        repos: Vec<Repository>,
        stats: &BackupStats,
    ) {
        let concurrency = self.opts.concurrency.max(1);
        let sem = Arc::new(Semaphore::new(concurrency));

        let mut handles = Vec::with_capacity(repos.len());

        for repo in repos {
            let permit = Arc::clone(&sem)
                .acquire_owned()
                .await
                .expect("semaphore closed");

            // Clone fields needed by the spawned task.
            let client = self.client.clone();
            let storage = self.storage.clone();
            let git = self.git.clone();
            let output = self.output.clone();
            let opts = self.opts.clone();
            let owner = owner.to_string();
            let clone_opts = self.make_clone_opts();
            let task_stats = stats.handle();

            let handle = tokio::spawn(async move {
                let _permit = permit; // released when task completes
                let result = backup_one_repo(
                    &client,
                    &storage,
                    &git,
                    &output,
                    &opts,
                    &owner,
                    &repo,
                    &clone_opts,
                    &task_stats,
                )
                .await;
                match result {
                    Ok(backed_up) => {
                        if backed_up {
                            task_stats.inc_backed_up();
                        } else {
                            task_stats.inc_skipped();
                        }
                    }
                    Err(e) => {
                        task_stats.inc_errored();
                        error!(
                            repo = %repo.full_name,
                            error = %e,
                            "repository backup failed, continuing"
                        );
                    }
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            // Individual repo panics should not abort the whole backup.
            if let Err(e) = handle.await {
                error!(error = %e, "repository backup task panicked");
            }
        }
    }

    /// Builds [`CloneOptions`] from the current `BackupOptions`.
    fn make_clone_opts(&self) -> CloneOptions {
        let token = match self.opts.prefer_ssh {
            // SSH uses key-based auth; no token needed in clone opts.
            true => None,
            false => {
                // Extract token from the client's credential for injection.
                // We expose a helper on GitHubClient for this purpose.
                self.client.token()
            }
        };
        CloneOptions {
            token,
            no_prune: self.opts.no_prune,
        }
    }
}

/// Backs up a single repository: metadata JSON + git mirror + sub-categories.
///
/// Returns `true` if the repository was backed up, `false` if it was skipped
/// (filtered out by fork/private settings or dry-run mode).
///
/// Extracted as a free function so it can be spawned as an independent task.
#[allow(clippy::too_many_arguments)]
async fn backup_one_repo<S, G>(
    client: &GitHubClient,
    storage: &S,
    git: &G,
    output: &OutputConfig,
    opts: &BackupOptions,
    owner: &str,
    repo: &Repository,
    clone_opts: &CloneOptions,
    stats: &BackupStats,
) -> Result<bool, CoreError>
where
    S: Storage,
    G: GitRunner,
{
    use crate::backup::repository::should_include;

    if !should_include(repo, opts) {
        return Ok(false);
    }

    if opts.dry_run {
        info!(repo = %repo.full_name, "dry-run: would back up repository");
        return Ok(false);
    }

    let repos_dir = output.repos_dir(owner);
    let wikis_dir = output.wikis_dir(owner);
    let meta_dir = output.repo_meta_dir(owner, &repo.name);

    backup_repository(repo, opts, &repos_dir, &meta_dir, storage, git, clone_opts).await?;
    backup_wiki(repo, opts, &wikis_dir, git, clone_opts).await?;
    backup_repo_metadata(client, storage, opts, owner, repo, &meta_dir, stats).await?;

    Ok(true)
}

/// Backs up all enabled JSON metadata for a repository.
async fn backup_repo_metadata<S>(
    client: &GitHubClient,
    storage: &S,
    opts: &BackupOptions,
    owner: &str,
    repo: &Repository,
    meta_dir: &std::path::Path,
    stats: &BackupStats,
) -> Result<(), CoreError>
where
    S: Storage,
{
    let issues_count = backup_issues(client, owner, &repo.name, opts, meta_dir, storage).await?;
    stats.add_issues(issues_count);

    let prs_count =
        backup_pull_requests(client, owner, &repo.name, opts, meta_dir, storage).await?;
    stats.add_prs(prs_count);
    backup_releases(client, owner, &repo.name, opts, meta_dir, storage).await?;

    if opts.labels {
        let labels = client.list_labels(owner, &repo.name).await?;
        storage.write_json(&meta_dir.join("labels.json"), &labels)?;
    }

    if opts.milestones {
        let milestones = client.list_milestones(owner, &repo.name).await?;
        storage.write_json(&meta_dir.join("milestones.json"), &milestones)?;
    }

    if opts.hooks {
        match client.list_hooks(owner, &repo.name).await {
            Ok(hooks) => {
                storage.write_json(&meta_dir.join("hooks.json"), &hooks)?;
            }
            Err(github_backup_client::ClientError::ApiError { status: 404, .. }) => {
                info!(repo = %repo.full_name, "skipping hooks (no admin access)");
            }
            Err(e) => return Err(e.into()),
        }
    }

    if opts.security_advisories {
        match client.list_security_advisories(owner, &repo.name).await {
            Ok(advisories) => {
                storage.write_json(&meta_dir.join("security_advisories.json"), &advisories)?;
            }
            Err(github_backup_client::ClientError::ApiError {
                status: 404 | 403, ..
            }) => {
                info!(repo = %repo.full_name, "skipping security advisories (not available)");
            }
            Err(e) => return Err(e.into()),
        }
    }

    if opts.topics {
        match client.list_repo_topics(owner, &repo.name).await {
            Ok(topics) => {
                storage.write_json(&meta_dir.join("topics.json"), &topics)?;
            }
            Err(github_backup_client::ClientError::ApiError {
                status: 404 | 403, ..
            }) => {
                info!(repo = %repo.full_name, "skipping topics (not available)");
            }
            Err(e) => return Err(e.into()),
        }
    }

    if opts.branches {
        let branches = client.list_branches(owner, &repo.name).await?;
        storage.write_json(&meta_dir.join("branches.json"), &branches)?;
    }

    backup_deploy_keys(client, owner, &repo.name, opts, meta_dir, storage).await?;
    backup_collaborators(client, owner, &repo.name, opts, meta_dir, storage).await?;

    let actions_count = backup_actions(client, owner, &repo.name, opts, meta_dir, storage).await?;
    stats.add_workflows(actions_count);

    backup_environments(client, owner, &repo.name, opts, meta_dir, storage).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn backup_engine_make_clone_opts_has_no_token_when_prefer_ssh() {
        let cred = Credential::Token("ghp_test".to_string());
        let client = GitHubClient::new(cred).expect("client");
        let opts = BackupOptions {
            prefer_ssh: true,
            ..Default::default()
        };
        let engine = BackupEngine::new(
            client,
            MemStorage::default(),
            SpyGitRunner::default(),
            OutputConfig::new("/backup"),
            opts,
        );
        let clone_opts = engine.make_clone_opts();
        assert!(
            clone_opts.token.is_none(),
            "SSH mode should not inject token"
        );
    }

    #[test]
    fn backup_engine_make_clone_opts_injects_token_for_https() {
        let cred = Credential::Token("ghp_test".to_string());
        let client = GitHubClient::new(cred).expect("client");
        let opts = BackupOptions {
            prefer_ssh: false,
            ..Default::default()
        };
        let engine = BackupEngine::new(
            client,
            MemStorage::default(),
            SpyGitRunner::default(),
            OutputConfig::new("/backup"),
            opts,
        );
        let clone_opts = engine.make_clone_opts();
        assert_eq!(clone_opts.token.as_deref(), Some("ghp_test"));
    }
}
