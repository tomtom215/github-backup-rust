// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Shared [`MockBackupClient`] for unit tests across backup modules.
//!
//! Only compiled in test builds.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use github_backup_client::{AssetSink, BackupClient, BoxFuture, ClientError, NodePage};
use github_backup_types::{
    Branch, Collaborator, CommitComment, DeployKey, Environment, Gist, GistComment, Hook, Issue,
    IssueComment, IssueEvent, Label, Milestone, Package, PackageVersion, Page, ProjectV2,
    ProjectV2Field, ProjectV2Item, PullRequest, PullRequestComment, PullRequestCommit,
    PullRequestReview, Raw, Release, Repository, Ruleset, SecurityAdvisory, Team, User, Workflow,
    WorkflowRun,
};

/// Configurable [`BackupClient`] for unit tests.
///
/// All methods return empty collections by default; use builder methods to
/// pre-load specific fixtures, [`with_raw`](Self::with_raw) to serve raw
/// JSON and [`with_error`](Self::with_error) to make a call fail.  Every
/// call is recorded ([`calls`](Self::calls)).
#[derive(Default, Clone)]
pub struct MockBackupClient {
    inner: Arc<Mutex<MockData>>,
}

#[derive(Default)]
struct MockData {
    /// `list_user_repos` never completes (simulates a server that never answers).
    hang_repo_listing: bool,
    user_repos: Vec<Repository>,
    gists: Vec<Gist>,
    starred_gists: Vec<Gist>,
    issues: Vec<Issue>,
    issue_comments: Vec<IssueComment>,
    pull_requests: Vec<PullRequest>,
    labels: Vec<Label>,
    milestones: Vec<Milestone>,
    releases: Vec<Release>,
    hooks: Vec<Hook>,
    security_advisories: Vec<SecurityAdvisory>,
    followers: Vec<User>,
    following: Vec<User>,
    starred: Vec<Repository>,
    asset_bytes: Vec<u8>,
    branches: Vec<Branch>,
    /// Maps branch name → protection rules.  Returns 404 when not found.
    branch_protections: HashMap<String, Value>,
    deploy_keys: Vec<DeployKey>,
    collaborators: Vec<Collaborator>,
    org_members: Vec<User>,
    org_teams: Vec<Team>,
    workflows: Vec<Workflow>,
    environments: Vec<Environment>,
    packages: Vec<Package>,
    package_versions: Vec<PackageVersion>,
    /// Raw JSON served instead of the typed fixtures, keyed by endpoint name
    /// (the names [`MockBackupClient::calls`] records).  Lets tests serve
    /// what the API really sends: extra properties, nulls, garbage elements.
    raw: HashMap<&'static str, Vec<Value>>,
    /// HTTP status to fail with, keyed by `endpoint` (every call) or
    /// `endpoint:argument` (one call, e.g. `issue_comments:7`).
    errors: HashMap<String, u16>,
    /// Every call made, as `endpoint:argument`.
    calls: Vec<String>,
    /// The field ids each `list_project_items` call asked for.
    project_item_field_ids: Vec<Vec<u64>>,
    /// Pages of discussions: page `i` answers the cursor `page-i` (the
    /// first page answers no cursor).
    discussion_pages: Vec<Vec<Value>>,
    /// Comments (with their replies) by discussion node id.
    discussion_comments: HashMap<String, Vec<Value>>,
    /// GraphQL error type to fail with, keyed like `errors`.
    graphql_errors: HashMap<String, String>,
}

#[allow(dead_code)]
impl MockBackupClient {
    /// Creates a new empty [`MockBackupClient`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Serves `values` as the raw JSON elements of `endpoint`, decoded like
    /// the real client does: elements that do not fit the typed model end up
    /// in [`Page::unparsed`].
    pub fn with_raw(self, endpoint: &'static str, values: Vec<Value>) -> Self {
        self.inner.lock().unwrap().raw.insert(endpoint, values);
        self
    }

    /// Makes `key` (`endpoint` or `endpoint:argument`) answer with HTTP
    /// `status`.
    pub fn with_error(self, key: &str, status: u16) -> Self {
        self.inner
            .lock()
            .unwrap()
            .errors
            .insert(key.to_string(), status);
        self
    }

    /// Every call made so far, as `endpoint:argument`.
    pub fn calls(&self) -> Vec<String> {
        self.inner.lock().unwrap().calls.clone()
    }

    /// The field ids of every `list_project_items` call.
    pub fn project_item_field_ids(&self) -> Vec<Vec<u64>> {
        self.inner.lock().unwrap().project_item_field_ids.clone()
    }

    /// Records the call and returns the injected error for it, if any.
    fn call(&self, endpoint: &str, argument: &str) -> Result<(), ClientError> {
        let mut data = self.inner.lock().unwrap();
        let key = format!("{endpoint}:{argument}");
        data.calls.push(key.clone());
        let status = data
            .errors
            .get(&key)
            .or_else(|| data.errors.get(endpoint))
            .copied();
        match status {
            Some(status) => Err(ClientError::ApiError {
                status,
                body: format!("mock {status} for {key}"),
            }),
            None => Ok(()),
        }
    }

    /// The page for `endpoint`: the raw override if one was set, otherwise the
    /// typed fixture selected by `typed` wrapped with [`Raw::from_typed`].
    fn page<T>(
        &self,
        endpoint: &str,
        argument: &str,
        typed: impl FnOnce(&MockData) -> Vec<T>,
    ) -> Result<Page<T>, ClientError>
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        self.call(endpoint, argument)?;
        let data = self.inner.lock().unwrap();
        Ok(match data.raw.get(endpoint) {
            Some(values) => Page::from_values(values.clone()),
            None => Page::from_typed(typed(&data)),
        })
    }

    /// Makes the repository listing hang forever, like an unresponsive server.
    pub fn with_repo_listing_that_never_answers(self) -> Self {
        self.inner.lock().unwrap().hang_repo_listing = true;
        self
    }

    /// Pre-loads the issues list.
    pub fn with_issues(self, issues: Vec<Issue>) -> Self {
        self.inner.lock().unwrap().issues = issues;
        self
    }

    /// Pre-loads the issue comments list.
    pub fn with_issue_comments(self, comments: Vec<IssueComment>) -> Self {
        self.inner.lock().unwrap().issue_comments = comments;
        self
    }

    /// Pre-loads the milestones list.
    pub fn with_milestones(self, milestones: Vec<Milestone>) -> Self {
        self.inner.lock().unwrap().milestones = milestones;
        self
    }

    /// Pre-loads the hooks list.
    pub fn with_hooks(self, hooks: Vec<Hook>) -> Self {
        self.inner.lock().unwrap().hooks = hooks;
        self
    }

    /// Pre-loads the security advisories list.
    pub fn with_security_advisories(self, advisories: Vec<SecurityAdvisory>) -> Self {
        self.inner.lock().unwrap().security_advisories = advisories;
        self
    }

    /// Pre-loads the user repositories list.
    pub fn with_user_repos(self, repos: Vec<Repository>) -> Self {
        self.inner.lock().unwrap().user_repos = repos;
        self
    }

    /// Pre-loads the pull requests list.
    pub fn with_pull_requests(self, prs: Vec<PullRequest>) -> Self {
        self.inner.lock().unwrap().pull_requests = prs;
        self
    }

    /// Pre-loads the releases list.
    pub fn with_releases(self, releases: Vec<Release>) -> Self {
        self.inner.lock().unwrap().releases = releases;
        self
    }

    /// Pre-loads the gists list.
    pub fn with_gists(self, gists: Vec<Gist>) -> Self {
        self.inner.lock().unwrap().gists = gists;
        self
    }

    /// Pre-loads the starred gists list.
    pub fn with_starred_gists(self, gists: Vec<Gist>) -> Self {
        self.inner.lock().unwrap().starred_gists = gists;
        self
    }

    /// Pre-loads followers.
    pub fn with_followers(self, followers: Vec<User>) -> Self {
        self.inner.lock().unwrap().followers = followers;
        self
    }

    /// Pre-loads following.
    pub fn with_following(self, following: Vec<User>) -> Self {
        self.inner.lock().unwrap().following = following;
        self
    }

    /// Pre-loads starred repositories.
    pub fn with_starred(self, repos: Vec<Repository>) -> Self {
        self.inner.lock().unwrap().starred = repos;
        self
    }

    /// Pre-loads asset bytes.
    pub fn with_asset_bytes(self, bytes: Vec<u8>) -> Self {
        self.inner.lock().unwrap().asset_bytes = bytes;
        self
    }

    /// Pre-loads labels.
    pub fn with_labels(self, labels: Vec<Label>) -> Self {
        self.inner.lock().unwrap().labels = labels;
        self
    }

    /// Pre-loads branches.
    pub fn with_branches(self, branches: Vec<Branch>) -> Self {
        self.inner.lock().unwrap().branches = branches;
        self
    }

    /// Pre-loads branch-protection rules (keyed by branch name).
    pub fn with_branch_protections(self, protections: HashMap<String, Value>) -> Self {
        self.inner.lock().unwrap().branch_protections = protections;
        self
    }

    /// Pre-loads deploy keys.
    pub fn with_deploy_keys(self, keys: Vec<DeployKey>) -> Self {
        self.inner.lock().unwrap().deploy_keys = keys;
        self
    }

    /// Pre-loads collaborators.
    pub fn with_collaborators(self, collaborators: Vec<Collaborator>) -> Self {
        self.inner.lock().unwrap().collaborators = collaborators;
        self
    }

    /// Pre-loads org members.
    pub fn with_org_members(self, members: Vec<User>) -> Self {
        self.inner.lock().unwrap().org_members = members;
        self
    }

    /// Pre-loads org teams.
    pub fn with_org_teams(self, teams: Vec<Team>) -> Self {
        self.inner.lock().unwrap().org_teams = teams;
        self
    }

    /// Pre-loads workflows.
    pub fn with_workflows(self, workflows: Vec<Workflow>) -> Self {
        self.inner.lock().unwrap().workflows = workflows;
        self
    }

    /// Pre-loads deployment environments.
    pub fn with_environments(self, envs: Vec<Environment>) -> Self {
        self.inner.lock().unwrap().environments = envs;
        self
    }

    /// Pre-loads packages.
    pub fn with_packages(self, packages: Vec<Package>) -> Self {
        self.inner.lock().unwrap().packages = packages;
        self
    }

    /// Serves `pages` as the pages of the discussion listing.
    pub fn with_discussion_pages(self, pages: Vec<Vec<Value>>) -> Self {
        self.inner.lock().unwrap().discussion_pages = pages;
        self
    }

    /// Serves `comments` as the comments of the discussion with node id `id`.
    pub fn with_discussion_comments(self, id: &str, comments: Vec<Value>) -> Self {
        self.inner
            .lock()
            .unwrap()
            .discussion_comments
            .insert(id.to_string(), comments);
        self
    }

    /// Makes `key` (`endpoint` or `endpoint:argument`) fail with a GraphQL
    /// error of type `kind` (`NOT_FOUND`, `FORBIDDEN`, ...).
    pub fn with_graphql_error(self, key: &str, kind: &str) -> Self {
        self.inner
            .lock()
            .unwrap()
            .graphql_errors
            .insert(key.to_string(), kind.to_string());
        self
    }

    /// [`call`](Self::call), then the injected GraphQL error, if any.
    fn graphql_call(&self, endpoint: &str, argument: &str) -> Result<(), ClientError> {
        self.call(endpoint, argument)?;
        let data = self.inner.lock().unwrap();
        let kind = data
            .graphql_errors
            .get(&format!("{endpoint}:{argument}"))
            .or_else(|| data.graphql_errors.get(endpoint));
        match kind {
            Some(kind) => Err(ClientError::GraphQl {
                message: format!("mock {kind} for {endpoint}:{argument}"),
                kinds: vec![kind.clone()],
            }),
            None => Ok(()),
        }
    }

    /// Pre-loads package versions.
    pub fn with_package_versions(self, versions: Vec<PackageVersion>) -> Self {
        self.inner.lock().unwrap().package_versions = versions;
        self
    }
}

/// Boxes a ready result.
fn ready<'a, T: Send + 'a>(
    result: Result<T, ClientError>,
) -> BoxFuture<'a, Result<T, ClientError>> {
    Box::pin(async move { result })
}

type Fut<'a, T> = BoxFuture<'a, Result<T, ClientError>>;

impl BackupClient for MockBackupClient {
    fn list_user_repos<'a>(&'a self, username: &'a str) -> Fut<'a, Page<Repository>> {
        if self.inner.lock().unwrap().hang_repo_listing {
            return Box::pin(std::future::pending());
        }
        ready(self.page("user_repos", username, |m| m.user_repos.clone()))
    }

    fn list_org_repos<'a>(&'a self, org: &'a str) -> Fut<'a, Page<Repository>> {
        ready(self.page("org_repos", org, |_| Vec::new()))
    }

    /// The `"repository"` raw element whose `name` matches, else the listed
    /// repository of that name with its JSON, else 404.
    fn get_repository<'a>(&'a self, _owner: &'a str, repo: &'a str) -> Fut<'a, Raw<Repository>> {
        let result = self.call("repository", repo).and_then(|()| {
            let data = self.inner.lock().unwrap();
            let raw = data
                .raw
                .get("repository")
                .and_then(|values| values.iter().find(|v| v["name"] == repo).cloned());
            let listed = data
                .user_repos
                .iter()
                .find(|r| r.name == repo)
                .map(|r| serde_json::to_value(r).expect("serialise"));
            match raw.or(listed) {
                Some(value) => Ok(Raw::from_value(value)?),
                None => Err(ClientError::ApiError {
                    status: 404,
                    body: format!("no repository {repo}"),
                }),
            }
        });
        ready(result)
    }

    fn list_followers<'a>(&'a self, username: &'a str) -> Fut<'a, Page<User>> {
        ready(self.page("followers", username, |m| m.followers.clone()))
    }

    fn list_following<'a>(&'a self, username: &'a str) -> Fut<'a, Page<User>> {
        ready(self.page("following", username, |m| m.following.clone()))
    }

    fn list_starred<'a>(&'a self, username: &'a str) -> Fut<'a, Page<Repository>> {
        ready(self.page("starred", username, |m| m.starred.clone()))
    }

    fn list_watched<'a>(&'a self, username: &'a str) -> Fut<'a, Page<Repository>> {
        ready(self.page("watched", username, |_| Vec::new()))
    }

    fn list_gists<'a>(&'a self, username: &'a str) -> Fut<'a, Page<Gist>> {
        ready(self.page("gists", username, |m| m.gists.clone()))
    }

    fn list_starred_gists<'a>(&'a self) -> Fut<'a, Page<Gist>> {
        ready(self.page("starred_gists", "", |m| m.starred_gists.clone()))
    }

    fn list_gist_comments<'a>(&'a self, gist_id: &'a str) -> Fut<'a, Page<GistComment>> {
        ready(self.page("gist_comments", gist_id, |_| Vec::new()))
    }

    /// Like GitHub, answers "410 Gone" for a listed repository whose Issues
    /// are switched off.
    fn list_issues<'a>(&'a self, _owner: &'a str, repo: &'a str) -> Fut<'a, Page<Issue>> {
        let disabled = self
            .inner
            .lock()
            .unwrap()
            .user_repos
            .iter()
            .any(|r| r.name == repo && !r.has_issues);
        if disabled {
            let _ = self.call("issues", repo);
            return ready(Err(ClientError::ApiError {
                status: 410,
                body: "Issues are disabled for this repo".into(),
            }));
        }
        ready(self.page("issues", repo, |m| m.issues.clone()))
    }

    fn list_issue_comments<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        issue_number: u64,
    ) -> Fut<'a, Page<IssueComment>> {
        ready(self.page("issue_comments", &issue_number.to_string(), |m| {
            m.issue_comments.clone()
        }))
    }

    fn list_issue_events<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        issue_number: u64,
    ) -> Fut<'a, Page<IssueEvent>> {
        ready(self.page("issue_events", &issue_number.to_string(), |_| Vec::new()))
    }

    fn list_repo_issue_comments_since<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        since: &'a str,
    ) -> Fut<'a, Page<IssueComment>> {
        ready(self.page("repo_issue_comments", since, |_| Vec::new()))
    }

    fn list_pull_requests<'a>(
        &'a self,
        _owner: &'a str,
        repo: &'a str,
    ) -> Fut<'a, Page<PullRequest>> {
        ready(self.page("pull_requests", repo, |m| m.pull_requests.clone()))
    }

    fn list_pull_comments<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        pr_number: u64,
    ) -> Fut<'a, Page<PullRequestComment>> {
        ready(self.page("pull_comments", &pr_number.to_string(), |_| Vec::new()))
    }

    fn list_pull_commits<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        pr_number: u64,
    ) -> Fut<'a, Page<PullRequestCommit>> {
        ready(self.page("pull_commits", &pr_number.to_string(), |_| Vec::new()))
    }

    fn compare_commits<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        base: &'a str,
        head: &'a str,
    ) -> Fut<'a, Page<PullRequestCommit>> {
        ready(
            self.page("compare_commits", &format!("{base}...{head}"), |_| {
                Vec::new()
            }),
        )
    }

    fn list_pull_reviews<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        pr_number: u64,
    ) -> Fut<'a, Page<PullRequestReview>> {
        ready(self.page("pull_reviews", &pr_number.to_string(), |_| Vec::new()))
    }

    fn list_repo_pull_comments_since<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        since: &'a str,
    ) -> Fut<'a, Page<PullRequestComment>> {
        ready(self.page("repo_pull_comments", since, |_| Vec::new()))
    }

    fn list_labels<'a>(&'a self, _owner: &'a str, repo: &'a str) -> Fut<'a, Page<Label>> {
        ready(self.page("labels", repo, |m| m.labels.clone()))
    }

    fn list_milestones<'a>(&'a self, _owner: &'a str, repo: &'a str) -> Fut<'a, Page<Milestone>> {
        ready(self.page("milestones", repo, |m| m.milestones.clone()))
    }

    fn list_releases<'a>(&'a self, _owner: &'a str, repo: &'a str) -> Fut<'a, Page<Release>> {
        ready(self.page("releases", repo, |m| m.releases.clone()))
    }

    fn list_hooks<'a>(&'a self, _owner: &'a str, repo: &'a str) -> Fut<'a, Page<Hook>> {
        ready(self.page("hooks", repo, |m| m.hooks.clone()))
    }

    fn list_security_advisories<'a>(
        &'a self,
        _owner: &'a str,
        repo: &'a str,
    ) -> Fut<'a, Page<SecurityAdvisory>> {
        ready(self.page("security_advisories", repo, |m| {
            m.security_advisories.clone()
        }))
    }

    fn list_branches<'a>(&'a self, _owner: &'a str, repo: &'a str) -> Fut<'a, Page<Branch>> {
        ready(self.page("branches", repo, |m| m.branches.clone()))
    }

    fn get_branch_protection<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        branch: &'a str,
    ) -> Fut<'a, Value> {
        let result = self.call("branch_protection", branch).and_then(|()| {
            self.inner
                .lock()
                .unwrap()
                .branch_protections
                .get(branch)
                .cloned()
                .ok_or_else(|| ClientError::ApiError {
                    status: 404,
                    body: format!("no protection rules found for branch '{branch}'"),
                })
        });
        ready(result)
    }

    fn list_rulesets<'a>(&'a self, _owner: &'a str, repo: &'a str) -> Fut<'a, Page<Ruleset>> {
        ready(self.page("rulesets", repo, |_| Vec::new()))
    }

    /// The listed ruleset with that id, with `"rules": []` added.
    fn get_ruleset<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        ruleset_id: u64,
    ) -> Fut<'a, Value> {
        let result = self.call("ruleset", &ruleset_id.to_string()).map(|()| {
            let data = self.inner.lock().unwrap();
            let mut summary = data
                .raw
                .get("rulesets")
                .and_then(|all| all.iter().find(|r| r["id"] == ruleset_id).cloned())
                .unwrap_or_else(|| json!({"id": ruleset_id}));
            summary["rules"] = json!([]);
            summary
        });
        ready(result)
    }

    fn list_commit_comments<'a>(
        &'a self,
        _owner: &'a str,
        repo: &'a str,
    ) -> Fut<'a, Page<CommitComment>> {
        ready(self.page("commit_comments", repo, |_| Vec::new()))
    }

    fn download_release_asset<'a>(
        &'a self,
        asset_url: &'a str,
        sink: &'a mut dyn AssetSink,
    ) -> Fut<'a, u64> {
        let called = self.call("asset", asset_url);
        let d = self.inner.lock().unwrap().asset_bytes.clone();
        Box::pin(async move {
            called?;
            // Deliver in small chunks, as the real client does.
            for chunk in d.chunks(4) {
                sink.write_chunk(chunk)?;
            }
            Ok(d.len() as u64)
        })
    }

    fn list_deploy_keys<'a>(&'a self, _owner: &'a str, repo: &'a str) -> Fut<'a, Page<DeployKey>> {
        ready(self.page("deploy_keys", repo, |m| m.deploy_keys.clone()))
    }

    fn list_collaborators<'a>(
        &'a self,
        _owner: &'a str,
        repo: &'a str,
    ) -> Fut<'a, Page<Collaborator>> {
        ready(self.page("collaborators", repo, |m| m.collaborators.clone()))
    }

    fn list_org_members<'a>(&'a self, org: &'a str) -> Fut<'a, Page<User>> {
        ready(self.page("org_members", org, |m| m.org_members.clone()))
    }

    fn list_org_teams<'a>(&'a self, org: &'a str) -> Fut<'a, Page<Team>> {
        ready(self.page("org_teams", org, |m| m.org_teams.clone()))
    }

    fn list_workflows<'a>(&'a self, _owner: &'a str, repo: &'a str) -> Fut<'a, Page<Workflow>> {
        ready(self.page("workflows", repo, |m| m.workflows.clone()))
    }

    fn list_workflow_runs<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        workflow_id: u64,
    ) -> Fut<'a, Page<WorkflowRun>> {
        ready(self.page("workflow_runs", &workflow_id.to_string(), |_| Vec::new()))
    }

    fn list_environments<'a>(
        &'a self,
        _owner: &'a str,
        repo: &'a str,
    ) -> Fut<'a, Page<Environment>> {
        ready(self.page("environments", repo, |m| m.environments.clone()))
    }

    fn list_projects<'a>(&'a self, owner: &'a str, _org: bool) -> Fut<'a, Page<ProjectV2>> {
        ready(self.page("projects", owner, |_| Vec::new()))
    }

    fn list_project_fields<'a>(
        &'a self,
        _owner: &'a str,
        _org: bool,
        project_number: u64,
    ) -> Fut<'a, Page<ProjectV2Field>> {
        ready(
            self.page("project_fields", &project_number.to_string(), |_| {
                Vec::new()
            }),
        )
    }

    fn list_project_items<'a>(
        &'a self,
        _owner: &'a str,
        _org: bool,
        project_number: u64,
        field_ids: &'a [u64],
    ) -> Fut<'a, Page<ProjectV2Item>> {
        self.inner
            .lock()
            .unwrap()
            .project_item_field_ids
            .push(field_ids.to_vec());
        ready(self.page("project_items", &project_number.to_string(), |_| Vec::new()))
    }

    fn list_discussion_categories<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
    ) -> Fut<'a, Vec<Value>> {
        ready(self.graphql_call("discussion_categories", "").map(|()| {
            let data = self.inner.lock().unwrap();
            data.raw
                .get("discussion_categories")
                .cloned()
                .unwrap_or_default()
        }))
    }

    fn list_discussions_page<'a>(
        &'a self,
        _owner: &'a str,
        _repo: &'a str,
        after: Option<&'a str>,
    ) -> Fut<'a, NodePage> {
        let result = self
            .graphql_call("discussions", after.unwrap_or_default())
            .and_then(|()| {
                let data = self.inner.lock().unwrap();
                let index = match after {
                    None => 0,
                    Some(cursor) => cursor
                        .strip_prefix("page-")
                        .and_then(|i| i.parse().ok())
                        .ok_or_else(|| ClientError::ApiError {
                            status: 400,
                            body: format!("mock: unknown cursor {cursor}"),
                        })?,
                };
                let pages = &data.discussion_pages;
                Ok(NodePage {
                    nodes: pages.get(index).cloned().unwrap_or_default(),
                    next: (index + 1 < pages.len()).then(|| format!("page-{}", index + 1)),
                })
            });
        ready(result)
    }

    fn list_discussion_comments<'a>(&'a self, discussion_id: &'a str) -> Fut<'a, Vec<Value>> {
        ready(
            self.graphql_call("discussion_comments", discussion_id)
                .map(|()| {
                    let data = self.inner.lock().unwrap();
                    data.discussion_comments
                        .get(discussion_id)
                        .cloned()
                        .unwrap_or_default()
                }),
        )
    }

    fn list_packages<'a>(
        &'a self,
        _owner: &'a str,
        _org: bool,
        package_type: &'a str,
    ) -> Fut<'a, Page<Package>> {
        ready(self.page("packages", package_type, |m| m.packages.clone()))
    }

    fn list_package_versions<'a>(
        &'a self,
        _owner: &'a str,
        _org: bool,
        _package_type: &'a str,
        package_name: &'a str,
    ) -> Fut<'a, Page<PackageVersion>> {
        ready(self.page("package_versions", package_name, |m| {
            m.package_versions.clone()
        }))
    }
}
