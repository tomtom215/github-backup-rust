// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Backing up one repository: git data, wiki and every metadata category.
//!
//! Each piece is an isolated step (see [`Steps`]), so a failure in, say, the
//! deploy-key listing is recorded and reported without costing the clone or
//! the issues of the same repository.

use tracing::info;

use github_backup_client::BackupClient;
use github_backup_types::config::{BackupOptions, OutputConfig};
use github_backup_types::{Raw, Repository};

use super::steps::{RunControl, Steps};
use crate::{
    backup::{
        actions::backup_actions, branches::backup_branches, collaborators::backup_collaborators,
        deploy_keys::backup_deploy_keys, discussion::backup_discussions,
        environments::backup_environments, hooks::backup_hooks, issue::backup_issues,
        labels::backup_labels, milestones::backup_milestones, project::backup_projects,
        pull_request::backup_pull_requests, release::backup_releases,
        repository::backup_repository, repository::should_include,
        security_advisories::backup_security_advisories, topics::backup_topics, wiki::backup_wiki,
    },
    git::{CloneOptions, GitRunner},
    stats::BackupStats,
    storage::Storage,
};

/// Everything one repository task needs, borrowed from the task.
pub(super) struct RepoContext<'a, C, S, G> {
    pub client: &'a C,
    pub storage: &'a S,
    pub git: &'a G,
    pub output: &'a OutputConfig,
    pub opts: &'a BackupOptions,
    pub owner: &'a str,
    pub clone_opts: &'a CloneOptions,
    pub stats: &'a BackupStats,
    pub control: &'a RunControl,
    /// Exact secret values to scrub from recorded failure messages.
    pub secrets: &'a [String],
    /// Incremental watermark for this repository, if any.
    pub since: Option<&'a str>,
}

/// What happened to one repository.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum RepoResult {
    /// Not backed up on purpose: filtered out, or a dry run.
    Skipped,
    /// Every step succeeded.
    Clean,
    /// At least one step failed or was not run; carries the first failure as a
    /// one-line description.
    Failed(String),
}

/// Backs up a single repository.
///
/// Never returns an error: failures are recorded in the statistics, and a fatal
/// one is handed to the run control so the engine stops.
pub(super) async fn backup_one_repo<C, S, G>(
    ctx: &RepoContext<'_, C, S, G>,
    repo: &Raw<Repository>,
) -> RepoResult
where
    C: BackupClient,
    S: Storage,
    G: GitRunner,
{
    if !should_include(repo, ctx.opts) {
        return RepoResult::Skipped;
    }
    if ctx.opts.dry_run {
        info!(repo = %repo.full_name, "dry-run: would back up repository");
        return RepoResult::Skipped;
    }

    // The name becomes a directory name; never let it leave the output tree.
    if !crate::paths::is_safe_component(&repo.name) {
        let mut steps = Steps::new(&repo.full_name, ctx.stats, ctx.control, ctx.secrets);
        steps.record(
            "repository",
            "repository name is not a safe directory name; not backed up",
        );
        return RepoResult::Failed(steps.first_failure().unwrap_or_default().to_string());
    }

    let repos_dir = ctx.output.repos_dir(ctx.owner);
    let wikis_dir = ctx.output.wikis_dir(ctx.owner);
    let meta_dir = ctx.output.repo_meta_dir(ctx.owner, &repo.name);
    let (client, storage, opts, owner, name) = (
        ctx.client,
        ctx.storage,
        ctx.opts,
        ctx.owner,
        repo.name.as_str(),
    );

    let mut steps = Steps::new(&repo.full_name, ctx.stats, ctx.control, ctx.secrets);

    steps
        .run(
            "repository",
            backup_repository(
                repo,
                opts,
                &repos_dir,
                &meta_dir,
                storage,
                ctx.git,
                ctx.clone_opts,
            ),
        )
        .await;
    steps
        .run(
            "wiki",
            backup_wiki(repo, opts, &wikis_dir, ctx.git, ctx.clone_opts),
        )
        .await;

    // A repository with Issues switched off answers the issues API with
    // "410 Gone"; that is its normal state, not a failure.
    if repo.has_issues || !(opts.issues || opts.issue_comments || opts.issue_events) {
        let count = steps
            .run(
                "issues",
                backup_issues(client, owner, name, opts, ctx.since, &meta_dir, storage),
            )
            .await;
        if let Some(n) = count {
            ctx.stats.add_issues(n);
        }
    } else {
        info!(repo = %repo.full_name, "issues are disabled on this repository, skipping");
    }

    let count = steps
        .run(
            "pull requests",
            backup_pull_requests(client, owner, name, opts, ctx.since, &meta_dir, storage),
        )
        .await;
    if let Some(n) = count {
        ctx.stats.add_prs(n);
    }

    steps
        .run(
            "releases",
            backup_releases(client, owner, name, opts, &meta_dir, storage),
        )
        .await;
    steps
        .run(
            "labels",
            backup_labels(client, owner, name, opts, &meta_dir, storage),
        )
        .await;
    steps
        .run(
            "milestones",
            backup_milestones(client, owner, name, opts, &meta_dir, storage),
        )
        .await;
    steps
        .run(
            "hooks",
            backup_hooks(client, owner, name, opts, &meta_dir, storage),
        )
        .await;
    steps
        .run(
            "security advisories",
            backup_security_advisories(client, owner, name, opts, &meta_dir, storage),
        )
        .await;
    steps
        .run(
            "topics",
            backup_topics(client, owner, name, opts, &meta_dir, storage),
        )
        .await;
    steps
        .run(
            "branches",
            backup_branches(client, owner, name, opts, &meta_dir, storage),
        )
        .await;
    steps
        .run(
            "deploy keys",
            backup_deploy_keys(client, owner, name, opts, &meta_dir, storage),
        )
        .await;
    steps
        .run(
            "collaborators",
            backup_collaborators(client, owner, name, opts, &meta_dir, storage),
        )
        .await;

    let count = steps
        .run(
            "actions",
            backup_actions(client, owner, name, opts, &meta_dir, storage),
        )
        .await;
    if let Some(n) = count {
        ctx.stats.add_workflows(n);
    }

    steps
        .run(
            "environments",
            backup_environments(client, owner, name, opts, &meta_dir, storage),
        )
        .await;

    let count = steps
        .run(
            "discussions",
            backup_discussions(client, owner, name, opts, &meta_dir, storage),
        )
        .await;
    if let Some(n) = count {
        ctx.stats.add_discussions(n);
    }

    steps
        .run(
            "projects",
            backup_projects(client, owner, name, opts, &meta_dir, storage),
        )
        .await;

    if steps.is_clean() {
        RepoResult::Clean
    } else {
        RepoResult::Failed(
            steps
                .first_failure()
                .unwrap_or("not completed because the run was stopped")
                .to_string(),
        )
    }
}
