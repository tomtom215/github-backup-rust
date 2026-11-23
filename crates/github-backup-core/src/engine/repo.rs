// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Backing up one repository: git data, wiki and every metadata category.
//!
//! Each piece is an isolated step (see [`Steps`]), so a failure in, say, the
//! deploy-key listing is recorded and reported without costing the clone or
//! the issues of the same repository.  Inside a category, an item that fails
//! (one issue's comments) is recorded too and the category carries on.
//!
//! How a category's answer is read:
//!
//! * 404 or 410 — the category does not exist for this repository (Issues
//!   switched off, no rulesets API): skipped, not a failure;
//! * 403 — the token is not permitted to read it: a failure naming what the
//!   token lacks, or, under `--all` (`BackupOptions::tolerate_forbidden`),
//!   one warning per category per run;
//! * anything else — a failure.

use std::collections::BTreeSet;
use std::future::Future;
use std::path::Path;
use std::sync::Mutex;

use serde_json::Value;
use tracing::{info, warn};

use github_backup_client::BackupClient;
use github_backup_types::config::{BackupOptions, CloneType, OutputConfig};
use github_backup_types::{Raw, Repository};

use super::incremental::RepoSince;
use super::steps::{RunControl, Steps};
use crate::{
    backup::{
        actions::backup_actions, all_json, branches::backup_branches,
        collaborators::backup_collaborators, commit_comments::backup_commit_comments,
        deploy_keys::backup_deploy_keys, discussion::backup_discussions,
        environments::backup_environments, hooks::backup_hooks, is_forbidden, is_gone,
        issue::backup_issues, json_str, labels::backup_labels, merge::unchanged_since,
        milestones::backup_milestones, pull_request::backup_pull_requests, refused,
        release::backup_releases, repository::backup_repository, repository::should_include,
        security_advisories::backup_security_advisories, topics::backup_topics, wiki::backup_wiki,
        Outcome,
    },
    error::CoreError,
    git::{CloneOptions, GitRunner},
    stats::BackupStats,
    storage::Storage,
};

/// The categories the token was refused (HTTP 403) during one run, so that
/// under `--all` each is reported with one warning per run rather than one
/// per repository.
#[derive(Debug, Default)]
pub(super) struct Refusals(Mutex<BTreeSet<String>>);

impl Refusals {
    /// `true` the first time `category` is refused in this run.
    fn first(&self, category: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(category.to_string())
    }
}

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
    /// What this repository's backup may take as already captured.
    pub since: &'a RepoSince,
    /// Categories refused so far in this run (shared by all repositories).
    pub refusals: &'a Refusals,
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

/// What a classic token needs for most categories.
const READ: &str = "read access to the repository; the `repo` scope for a private one";
const ADMIN: &str = "admin access to the repository";

/// Backs up a single repository.
///
/// Never returns an error: failures are recorded in the statistics, and a fatal
/// one is handed to the run control so the engine stops.
pub(super) async fn backup_one_repo<C, S, G>(
    ctx: &RepoContext<'_, C, S, G>,
    listed: &Raw<Repository>,
) -> RepoResult
where
    C: BackupClient,
    S: Storage,
    G: GitRunner,
{
    if !should_include(listed, ctx.opts) {
        return RepoResult::Skipped;
    }
    if ctx.opts.dry_run {
        info!(repo = %listed.full_name, "dry-run: would back up repository");
        return RepoResult::Skipped;
    }

    let mut steps = Steps::new(&listed.full_name, ctx.stats, ctx.control, ctx.secrets);

    // The name becomes a directory name; never let it leave the output tree.
    if !crate::paths::is_safe_component(&listed.name) {
        steps.record(
            "repository",
            "repository name is not a safe directory name; not backed up",
        );
        return finish(&steps);
    }

    let repos_dir = ctx.output.repos_dir(ctx.owner);
    let wikis_dir = ctx.output.wikis_dir(ctx.owner);
    let meta_dir = ctx.output.repo_meta_dir(ctx.owner, &listed.name);
    let (client, storage, opts, owner, name) = (
        ctx.client,
        ctx.storage,
        ctx.opts,
        ctx.owner,
        listed.name.as_str(),
    );

    // A repository deleted and recreated under the same name is another
    // repository: never mix the two backups.
    if steps
        .run("repository", async {
            set_aside_replaced(ctx, listed, &meta_dir, &repos_dir, &wikis_dir)
        })
        .await
        .is_none()
    {
        return finish(&steps);
    }

    // The listing leaves out `parent`, `source`, the merge settings, ...; the
    // repository's own endpoint has everything.  Without it, the listed
    // object is still a usable (smaller) `info.json`.
    let details = run_category(
        ctx,
        &mut steps,
        &listed.full_name,
        "repository details",
        READ,
        async { Ok(Some(client.get_repository(owner, name).await?)) },
    )
    .await
    .flatten();
    let repo = details.as_ref().unwrap_or(listed);

    let without_fetch;
    let repo_opts = if fetch_skippable(ctx, repo).await {
        without_fetch = BackupOptions {
            repositories: false,
            ..opts.clone()
        };
        &without_fetch
    } else {
        opts
    };
    steps
        .run(
            "repository",
            backup_repository(
                repo,
                repo_opts,
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
    steps
        .run("topics", async {
            backup_topics(repo, opts, &meta_dir, storage)
        })
        .await;

    let since = ctx.since.items.as_deref();
    // Issues switched off answer "410 Gone"; the issue backup then still
    // fetches the conversation of the pull requests.
    if let Some(n) = category(
        ctx,
        &mut steps,
        &listed.full_name,
        "issues",
        READ,
        backup_issues(
            client,
            owner,
            name,
            opts,
            since,
            ctx.since.issue_list.as_deref(),
            &meta_dir,
            storage,
        ),
    )
    .await
    {
        ctx.stats.add_issues(n);
    }
    if let Some(n) = category(
        ctx,
        &mut steps,
        &listed.full_name,
        "pull requests",
        READ,
        backup_pull_requests(
            client,
            owner,
            name,
            opts,
            since,
            ctx.since.pull_list.as_deref(),
            &meta_dir,
            storage,
        ),
    )
    .await
    {
        ctx.stats.add_prs(n);
    }

    category(
        ctx,
        &mut steps,
        &listed.full_name,
        "releases",
        READ,
        backup_releases(client, owner, name, opts, &meta_dir, storage),
    )
    .await;
    category(
        ctx,
        &mut steps,
        &listed.full_name,
        "labels",
        READ,
        backup_labels(client, owner, name, opts, &meta_dir, storage),
    )
    .await;
    category(
        ctx,
        &mut steps,
        &listed.full_name,
        "milestones",
        READ,
        backup_milestones(client, owner, name, opts, &meta_dir, storage),
    )
    .await;
    category(
        ctx,
        &mut steps,
        &listed.full_name,
        "hooks",
        ADMIN,
        backup_hooks(client, owner, name, opts, &meta_dir, storage),
    )
    .await;
    category(
        ctx,
        &mut steps,
        &listed.full_name,
        "security advisories",
        "the `repo` or `repository_advisories:read` scope",
        backup_security_advisories(client, owner, name, opts, &meta_dir, storage),
    )
    .await;
    category(
        ctx,
        &mut steps,
        &listed.full_name,
        "branches",
        "admin access to the repository for branch protection rules",
        backup_branches(client, owner, name, opts, &meta_dir, storage),
    )
    .await;
    category(
        ctx,
        &mut steps,
        &listed.full_name,
        "deploy keys",
        ADMIN,
        backup_deploy_keys(client, owner, name, opts, &meta_dir, storage),
    )
    .await;
    category(
        ctx,
        &mut steps,
        &listed.full_name,
        "collaborators",
        "write access to the repository; `read:org` as well for an organisation's",
        backup_collaborators(client, owner, name, opts, &meta_dir, storage),
    )
    .await;
    if let Some(n) = category(
        ctx,
        &mut steps,
        &listed.full_name,
        "actions",
        READ,
        backup_actions(client, owner, name, opts, &meta_dir, storage),
    )
    .await
    {
        ctx.stats.add_workflows(n);
    }
    category(
        ctx,
        &mut steps,
        &listed.full_name,
        "environments",
        READ,
        backup_environments(client, owner, name, opts, &meta_dir, storage),
    )
    .await;
    category(
        ctx,
        &mut steps,
        &listed.full_name,
        "commit comments",
        READ,
        backup_commit_comments(client, owner, name, opts, &meta_dir, storage),
    )
    .await;
    if let Some(n) = category(
        ctx,
        &mut steps,
        &listed.full_name,
        "discussions",
        "read access to the repository; for a private one the `repo` scope \
         (classic token) or the Discussions read permission (fine-grained)",
        backup_discussions(client, owner, repo, opts, since, &meta_dir, storage),
    )
    .await
    {
        ctx.stats.add_discussions(n);
    }

    finish(&steps)
}

/// Whether the `git fetch` of `repo` can be skipped this run.
///
/// [`RepoSince::git`] says nothing was pushed since the clone was last brought
/// up to date; the clone must also still be on disk.  A clone that holds the
/// `refs/pull/*` refs (mirror, shallow and LFS clones) can change without a
/// push to the repository itself (a push to a fork's pull request branch), so
/// it also needs no pull request updated since: one request for the pull
/// requests updated since, newest first.  When in doubt (that request fails),
/// fetch.
async fn fetch_skippable<C, S, G>(ctx: &RepoContext<'_, C, S, G>, repo: &Repository) -> bool
where
    C: BackupClient,
{
    let Some(since) = ctx.since.git.as_deref() else {
        return false;
    };
    let opts = ctx.opts;
    let working_tree = !opts.lfs && opts.clone_type == CloneType::Full;
    let clone = ctx.output.repos_dir(ctx.owner).join(if working_tree {
        repo.name.clone()
    } else {
        format!("{}.git", repo.name)
    });
    if !clone.is_dir() {
        return false;
    }
    let pull_refs = opts.lfs || !matches!(opts.clone_type, CloneType::Bare | CloneType::Full);
    if pull_refs {
        match ctx
            .client
            .list_pull_requests(ctx.owner, &repo.name, Some(since))
            .await
        {
            Ok(pulls) => {
                let changed = all_json(&pulls).any(|pr| {
                    !unchanged_since(json_str(pr, "updated_at").unwrap_or_default(), Some(since))
                });
                if changed {
                    return false;
                }
            }
            Err(e) => {
                info!(repo = %repo.full_name, error = %e, "cannot tell whether a pull request changed; fetching");
                return false;
            }
        }
    }
    info!(repo = %repo.full_name, since, "nothing pushed since the last backup; not fetching");
    true
}

fn finish(steps: &Steps<'_>) -> RepoResult {
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

/// Runs one category as a step, reading its answer as the module
/// documentation describes, and records every item it carried on past.
///
/// Returns the category's count, or `None` if it failed or was skipped.
async fn category<C, S, G, T>(
    ctx: &RepoContext<'_, C, S, G>,
    steps: &mut Steps<'_>,
    repo: &str,
    step: &str,
    needs: &str,
    work: impl Future<Output = Result<T, CoreError>>,
) -> Option<u64>
where
    T: Into<Outcome>,
{
    let outcome: Outcome = run_category(ctx, steps, repo, step, needs, async {
        work.await.map(|v| Some(v.into()))
    })
    .await
    .flatten()?;
    for (what, message) in &outcome.failed {
        steps.record(step, &format!("{what}: {message}"));
    }
    Some(outcome.count)
}

/// [`category`] without the item failures: `Some(value)` on success,
/// `Some(None)` when the category is absent or its refusal tolerated, `None`
/// when the step failed.
async fn run_category<C, S, G, T>(
    ctx: &RepoContext<'_, C, S, G>,
    steps: &mut Steps<'_>,
    repo: &str,
    step: &str,
    needs: &str,
    work: impl Future<Output = Result<Option<T>, CoreError>>,
) -> Option<Option<T>> {
    steps
        .run(step, async {
            match work.await {
                Err(e) if is_gone(&e) => {
                    info!(repo, step, "not available for this repository; skipping");
                    Ok(None)
                }
                Err(e) if is_forbidden(&e) => {
                    refused(ctx.opts, step, needs, e, ctx.refusals.first(step))?;
                    Ok(None)
                }
                other => other,
            }
        })
        .await
}

/// Sets the backup of a repository aside when the repository with this name
/// is not the one that was backed up before (its `info.json` has another
/// `id`): the old repository was deleted and a new one created with the
/// same name.
///
/// The metadata directory, the clone and the wiki clone are renamed to
/// `<name>@<old id>` (a character GitHub does not allow in repository names,
/// so it cannot collide with a real repository) and the new repository is
/// backed up from scratch.  Nothing is deleted.
fn set_aside_replaced<C, S, G>(
    ctx: &RepoContext<'_, C, S, G>,
    repo: &Repository,
    meta_dir: &Path,
    repos_dir: &Path,
    wikis_dir: &Path,
) -> Result<(), CoreError>
where
    S: Storage,
{
    let Some(bytes) = ctx.storage.read(&meta_dir.join("info.json"))? else {
        return Ok(());
    };
    let stored_id = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|info| info.get("id").and_then(Value::as_u64));
    let Some(old_id) = stored_id.filter(|id| *id != repo.id) else {
        return Ok(());
    };

    let aside = format!("{}@{old_id}", repo.name);
    warn!(
        repo = %repo.full_name,
        old_id,
        new_id = repo.id,
        set_aside_as = %aside,
        "this repository replaced an earlier one of the same name; the earlier \
         backup is kept under the new name and this one starts afresh"
    );
    ctx.storage
        .rename(meta_dir, &ctx.output.repo_meta_dir(ctx.owner, &aside))?;
    for (dir, suffix) in [
        (repos_dir, ".git"),
        (repos_dir, ""),
        (wikis_dir, ".wiki.git"),
    ] {
        ctx.storage.rename(
            &dir.join(format!("{}{suffix}", repo.name)),
            &dir.join(format!("{aside}{suffix}")),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::git::test_support::SpyGitRunner;
    use crate::storage::test_support::MemStorage;
    use serde_json::json;
    use std::path::PathBuf;

    fn repo_json(id: u64) -> Value {
        json!({
            "id": id, "name": "r", "full_name": "o/r",
            "owner": {"id": 1, "login": "o", "type": "User", "avatar_url": "", "html_url": ""},
            "private": false, "fork": false, "has_issues": true, "has_wiki": false,
            "clone_url": "https://github.com/o/r.git", "ssh_url": "git@github.com:o/r.git",
            "html_url": "https://github.com/o/r", "topics": ["listed"]
        })
    }

    struct Fixture {
        client: MockBackupClient,
        storage: MemStorage,
        git: SpyGitRunner,
        output: OutputConfig,
        opts: BackupOptions,
        stats: BackupStats,
        control: RunControl,
        refusals: Refusals,
        clone_opts: CloneOptions,
        since: RepoSince,
    }

    impl Fixture {
        fn new(client: MockBackupClient, opts: BackupOptions) -> Self {
            Self {
                client,
                storage: MemStorage::default(),
                git: SpyGitRunner::default(),
                output: OutputConfig::new("/out"),
                opts,
                stats: BackupStats::new(),
                control: RunControl::default(),
                refusals: Refusals::default(),
                clone_opts: CloneOptions::unauthenticated(),
                since: RepoSince::default(),
            }
        }

        async fn run(&self, listed: &Raw<Repository>) -> RepoResult {
            let ctx = RepoContext {
                client: &self.client,
                storage: &self.storage,
                git: &self.git,
                output: &self.output,
                opts: &self.opts,
                owner: "o",
                clone_opts: &self.clone_opts,
                stats: &self.stats,
                control: &self.control,
                secrets: &[],
                since: &self.since,
                refusals: &self.refusals,
            };
            backup_one_repo(&ctx, listed).await
        }

        fn file(&self, path: &str) -> Option<Value> {
            self.storage
                .get(&PathBuf::from("/out/o/json/repos").join(path))
                .map(|b| serde_json::from_slice(&b).expect("json"))
        }
    }

    fn listed(id: u64) -> Raw<Repository> {
        Raw::from_value(repo_json(id)).expect("repository")
    }

    /// `info.json` and `topics.json` come from `GET /repos/{owner}/{repo}`,
    /// which has `parent`, `source` and the merge settings the listing lacks.
    #[tokio::test]
    async fn info_json_is_the_full_repository_object() {
        let mut full = repo_json(1);
        full["parent"] = json!({"full_name": "upstream/r"});
        full["allow_squash_merge"] = json!(true);
        full["topics"] = json!(["full"]);
        let client = MockBackupClient::new().with_raw("repository", vec![full]);
        let opts = BackupOptions {
            topics: true,
            ..Default::default()
        };
        let fx = Fixture::new(client, opts);

        assert_eq!(fx.run(&listed(1)).await, RepoResult::Clean);
        let info = fx.file("r/info.json").expect("info.json");
        assert_eq!(info["parent"]["full_name"], "upstream/r");
        assert_eq!(info["allow_squash_merge"], true);
        assert_eq!(fx.file("r/topics.json"), Some(json!(["full"])));
    }

    /// Without the full object the listed one is still stored, and the
    /// failure to get it is reported.
    #[tokio::test]
    async fn without_the_full_object_the_listing_is_stored_and_the_failure_reported() {
        let client = MockBackupClient::new().with_error("repository", 502);
        let fx = Fixture::new(client, BackupOptions::default());
        assert!(matches!(fx.run(&listed(1)).await, RepoResult::Failed(_)));
        assert_eq!(fx.file("r/info.json").expect("info.json")["id"], 1);
        assert_eq!(fx.stats.failures()[0].step, "repository details");
    }

    /// A repository deleted and recreated under the same name must not be
    /// merged into the old one's backup: the old one is set aside.
    #[tokio::test]
    async fn a_recreated_repository_sets_the_old_backup_aside() {
        let client = MockBackupClient::new()
            .with_raw("repository", vec![repo_json(2)])
            .with_raw(
                "issues",
                vec![json!({"number": 1, "title": "new repo's #1"})],
            );
        let opts = BackupOptions {
            issues: true,
            ..Default::default()
        };
        let fx = Fixture::new(client, opts);
        fx.storage
            .write_json(
                &PathBuf::from("/out/o/json/repos/r/info.json"),
                &repo_json(1),
            )
            .expect("seed");
        fx.storage
            .write_json(
                &PathBuf::from("/out/o/json/repos/r/issues.json"),
                &json!([{"number": 1, "title": "old repo's #1"}, {"number": 2}]),
            )
            .expect("seed");

        assert_eq!(fx.run(&listed(2)).await, RepoResult::Clean);

        assert_eq!(fx.file("r/info.json").expect("new")["id"], 2);
        assert_eq!(
            fx.file("r/issues.json").expect("new"),
            json!([{"number": 1, "title": "new repo's #1"}])
        );
        assert_eq!(fx.file("r@1/info.json").expect("kept")["id"], 1);
        assert_eq!(
            fx.file("r@1/issues.json")
                .expect("kept")
                .as_array()
                .map(Vec::len),
            Some(2)
        );
    }

    #[tokio::test]
    async fn the_same_repository_is_not_set_aside() {
        let fx = Fixture::new(MockBackupClient::new(), BackupOptions::default());
        fx.storage
            .write_json(
                &PathBuf::from("/out/o/json/repos/r/info.json"),
                &repo_json(1),
            )
            .expect("seed");
        assert_eq!(fx.run(&listed(1)).await, RepoResult::Clean);
        assert!(fx.file("r@1/info.json").is_none());
    }

    /// Explicitly requested, a category the token may not read is a failure
    /// that names what is missing.
    #[tokio::test]
    async fn a_refused_category_is_a_named_failure_when_requested() {
        let client = MockBackupClient::new().with_error("hooks", 403);
        let opts = BackupOptions {
            hooks: true,
            ..Default::default()
        };
        let fx = Fixture::new(client, opts);
        assert!(matches!(fx.run(&listed(1)).await, RepoResult::Failed(_)));
        let failures = fx.stats.failures();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].step, "hooks");
        assert!(
            failures[0].message.contains("admin access"),
            "{}",
            failures[0].message
        );
    }

    /// Under `--all` the same refusal is a warning (once per run), not a
    /// failure, so a token without that permission does not fail every run.
    #[tokio::test]
    async fn a_refused_category_is_tolerated_under_all() {
        let client = MockBackupClient::new().with_error("hooks", 403);
        let opts = BackupOptions {
            hooks: true,
            tolerate_forbidden: true,
            ..Default::default()
        };
        let fx = Fixture::new(client, opts);
        assert_eq!(fx.run(&listed(1)).await, RepoResult::Clean);
        assert!(!fx.refusals.first("hooks"), "already reported in this run");
    }

    /// A category that does not exist for the repository (404/410) is never
    /// a failure.
    #[tokio::test]
    async fn an_absent_category_is_not_a_failure() {
        let client = MockBackupClient::new()
            .with_error("hooks", 404)
            .with_error("issues", 410);
        let opts = BackupOptions {
            hooks: true,
            issues: true,
            ..Default::default()
        };
        let fx = Fixture::new(client, opts);
        assert_eq!(fx.run(&listed(1)).await, RepoResult::Clean);
    }

    /// Items a category carried on past are recorded, one failure each.
    #[tokio::test]
    async fn item_failures_inside_a_category_are_recorded() {
        let client = MockBackupClient::new()
            .with_raw("issues", vec![json!({"number": 1}), json!({"number": 2})])
            .with_error("issue_comments:1", 500);
        let opts = BackupOptions {
            issue_comments: true,
            ..Default::default()
        };
        let fx = Fixture::new(client, opts);
        assert!(matches!(fx.run(&listed(1)).await, RepoResult::Failed(_)));
        let failures = fx.stats.failures();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].step, "issues");
        assert!(failures[0].message.starts_with("issue #1 comments: "));
        assert!(fx.file("r/issue_comments/2.json").is_some());
    }

    /// GraphQL answers for discussions read like the REST statuses: a
    /// refusal fails the requested category (or is one warning under
    /// `--all`), `NOT_FOUND` is an absent category, anything else fails.
    #[tokio::test]
    async fn graphql_refusals_and_absences_of_discussions() {
        for (kind, tolerate, clean) in [
            ("FORBIDDEN", false, false),
            ("FORBIDDEN", true, true),
            ("NOT_FOUND", false, true),
            ("INTERNAL", true, false),
        ] {
            let client = MockBackupClient::new().with_graphql_error("discussion_categories", kind);
            let opts = BackupOptions {
                discussions: true,
                tolerate_forbidden: tolerate,
                ..Default::default()
            };
            let fx = Fixture::new(client, opts);
            let result = fx.run(&listed(1)).await;
            assert_eq!(
                result == RepoResult::Clean,
                clean,
                "{kind} {tolerate}: {result:?}"
            );
            if !clean {
                assert_eq!(fx.stats.failures()[0].step, "discussions");
            }
        }
    }

    #[tokio::test]
    async fn discussions_are_stored_and_counted() {
        let client = MockBackupClient::new().with_discussion_pages(vec![vec![
            json!({"id": "D_1", "number": 1, "updatedAt": "2024-01-01T00:00:00Z"}),
        ]]);
        let opts = BackupOptions {
            discussions: true,
            ..Default::default()
        };
        let fx = Fixture::new(client, opts);
        assert_eq!(fx.run(&listed(1)).await, RepoResult::Clean);
        assert_eq!(fx.stats.discussions_fetched(), 1);
        assert!(fx.file("r/discussions.json").is_some());
        assert_eq!(fx.file("r/discussion_comments/1.json"), Some(json!([])));
    }
}
