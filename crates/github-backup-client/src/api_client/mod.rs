// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! [`BackupClient`] — abstract interface over the GitHub REST API.
//!
//! This trait covers every API method used by the backup engine.  The
//! production implementation is [`crate::GitHubClient`], but test code can
//! substitute a lightweight mock that returns pre-configured fixtures without
//! making any network requests.
//!
//! # Why a separate trait?
//!
//! Decoupling backup logic from the concrete HTTP client enables:
//!
//! 1. **Unit tests** that run without network access or live credentials.
//! 2. **Clearer API surface**: the engine only depends on what it actually uses.
//! 3. **Alternative implementations** (e.g. a caching proxy).
//!
//! # Lossless results
//!
//! Every list method returns a [`Page`]: the elements that fit the typed model
//! (as [`Raw`], which derefs to the model and serialises as the original JSON)
//! plus, verbatim, the elements that did not.  One unexpected object therefore
//! never fails a list, and writing a page to disk writes everything the API
//! returned.  Objects the backup only stores (branch protection, a ruleset)
//! are returned as plain [`Value`]s.
//!
//! # Object safety
//!
//! The trait uses `Pin<Box<dyn Future>>` returns so it is **object-safe** and
//! can be used with `dyn BackupClient` where dynamic dispatch is desired.
//!
//! # Modules
//!
//! - [`mod@impl_github`] — blanket `impl BackupClient for GitHubClient`.

mod impl_github;

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use github_backup_types::{
    Branch, Collaborator, CommitComment, DeployKey, Environment, Gist, GistComment, Hook, Issue,
    IssueComment, IssueEvent, Label, Milestone, Package, PackageVersion, Page, ProjectV2,
    ProjectV2Field, ProjectV2Item, PullRequest, PullRequestComment, PullRequestCommit,
    PullRequestReview, Raw, Release, Repository, Ruleset, SecurityAdvisory, Team, User, Workflow,
    WorkflowRun,
};

use crate::error::ClientError;

/// Boxed, pinned, send future returned by every [`BackupClient`] method.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Destination of a streamed release-asset download.
///
/// [`BackupClient::download_release_asset`] hands the body to the sink chunk by
/// chunk, so an asset of any size is never held in memory.
pub trait AssetSink: Send {
    /// Appends `chunk` to the destination.
    ///
    /// # Errors
    ///
    /// Returns the I/O error when the chunk cannot be stored (disk full, ...);
    /// the download is then abandoned with [`ClientError::Io`].
    fn write_chunk(&mut self, chunk: &[u8]) -> std::io::Result<()>;
}

/// What [`BackupClient::download_attachment`] delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentDownload {
    /// Bytes handed to the sink.
    pub bytes: u64,
    /// The media type of the final response (`Content-Type` without its
    /// parameters, lower case), if it named one.
    pub content_type: Option<String>,
}

/// One page of a GraphQL connection: its nodes, exactly as GitHub sent
/// them, and the cursor of the next page (`None` on the last page).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodePage {
    /// The nodes of this page.
    pub nodes: Vec<Value>,
    /// The cursor to ask for the next page with, if there is one.
    pub next: Option<String>,
}

/// All GitHub API operations required by the backup engine.
///
/// The production implementation is [`crate::GitHubClient`]. Tests substitute
/// a mock that returns pre-configured data.
///
/// Repository objects (listings and [`get_repository`](Self::get_repository))
/// never carry `temp_clone_token`, a short-lived credential GitHub may
/// include: the backup must not write a credential to a file.
pub trait BackupClient: Send + Sync {
    // ── Repositories ──────────────────────────────────────────────────────

    /// Lists repositories owned by a user.
    ///
    /// When the credential belongs to `username` the listing includes the
    /// account's private repositories (`GET /user/repos`); for anyone else,
    /// and without a credential, only public repositories are visible
    /// (`GET /users/{username}/repos`).
    fn list_user_repos<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Page<Repository>, ClientError>>;

    /// Lists repositories belonging to an organisation.
    fn list_org_repos<'a>(
        &'a self,
        org: &'a str,
    ) -> BoxFuture<'a, Result<Page<Repository>, ClientError>>;

    /// Returns one repository in full (`GET /repos/{owner}/{repo}`), with the
    /// properties the listings leave out (`parent`, `source`, merge settings).
    fn get_repository<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Raw<Repository>, ClientError>>;

    // ── User social graph ─────────────────────────────────────────────────

    /// Returns the followers of a user.
    fn list_followers<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Page<User>, ClientError>>;

    /// Returns the users that `username` is following.
    fn list_following<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Page<User>, ClientError>>;

    /// Returns repositories starred by `username`.
    fn list_starred<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Page<Repository>, ClientError>>;

    /// Returns repositories watched by `username`.
    fn list_watched<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Page<Repository>, ClientError>>;

    // ── Gists ─────────────────────────────────────────────────────────────

    /// Returns gists owned by `username`.
    ///
    /// When the credential belongs to `username` the listing includes the
    /// account's secret gists (`GET /gists`); otherwise only public gists are
    /// visible (`GET /users/{username}/gists`).
    fn list_gists<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Page<Gist>, ClientError>>;

    /// Returns gists starred by the authenticated user.
    fn list_starred_gists<'a>(&'a self) -> BoxFuture<'a, Result<Page<Gist>, ClientError>>;

    /// Lists the comments on a gist.
    fn list_gist_comments<'a>(
        &'a self,
        gist_id: &'a str,
    ) -> BoxFuture<'a, Result<Page<GistComment>, ClientError>>;

    // ── Issues ────────────────────────────────────────────────────────────

    /// Lists all issues for a repository, in every state, **including pull
    /// requests**.
    ///
    /// GitHub's issues API returns every pull request as an issue too (with a
    /// `pull_request` stub); `Issue::is_pull_request` tells them apart.
    ///
    /// With `since` (ISO 8601) only the issues updated at or after it.
    fn list_issues<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        since: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Page<Issue>, ClientError>>;

    /// Lists comments on a specific issue.
    ///
    /// Issues and pull requests share one number space, so for a pull request
    /// this returns its conversation comments (the discussion thread); inline
    /// review comments are [`list_pull_comments`](Self::list_pull_comments).
    fn list_issue_comments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        issue_number: u64,
    ) -> BoxFuture<'a, Result<Page<IssueComment>, ClientError>>;

    /// Lists the events (`closed`, `labeled`, `assigned`, ...) of a specific
    /// issue or pull request (`GET .../issues/{n}/events`; the richer
    /// `/timeline` is not used).
    fn list_issue_events<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        issue_number: u64,
    ) -> BoxFuture<'a, Result<Page<IssueEvent>, ClientError>>;

    /// Lists the issue and pull-request conversation comments of a whole
    /// repository created or edited at or after `since` (ISO 8601).
    fn list_repo_issue_comments_since<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        since: &'a str,
    ) -> BoxFuture<'a, Result<Page<IssueComment>, ClientError>>;

    // ── Pull Requests ─────────────────────────────────────────────────────

    /// Lists the pull requests of a repository, in every state: all of
    /// them, or with `updated_since` (`YYYY-MM-DDTHH:MM:SSZ`) at least every
    /// one updated at or after it (possibly some older ones too).
    fn list_pull_requests<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        updated_since: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Page<PullRequest>, ClientError>>;

    /// Lists review comments on a specific pull request.
    fn list_pull_comments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> BoxFuture<'a, Result<Page<PullRequestComment>, ClientError>>;

    /// Lists commits included in a specific pull request; GitHub returns at
    /// most 250 here.
    fn list_pull_commits<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> BoxFuture<'a, Result<Page<PullRequestCommit>, ClientError>>;

    /// Lists every commit reachable from `head` but not from `base`, without
    /// the 250-commit cap of [`list_pull_commits`](Self::list_pull_commits).
    fn compare_commits<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        base: &'a str,
        head: &'a str,
    ) -> BoxFuture<'a, Result<Page<PullRequestCommit>, ClientError>>;

    /// Lists reviews submitted on a specific pull request.
    fn list_pull_reviews<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> BoxFuture<'a, Result<Page<PullRequestReview>, ClientError>>;

    /// Lists the review comments of a whole repository created or edited at
    /// or after `since` (ISO 8601).
    fn list_repo_pull_comments_since<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        since: &'a str,
    ) -> BoxFuture<'a, Result<Page<PullRequestComment>, ClientError>>;

    // ── Repository metadata ───────────────────────────────────────────────

    /// Lists labels for a repository.
    fn list_labels<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<Label>, ClientError>>;

    /// Lists milestones for a repository.
    fn list_milestones<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<Milestone>, ClientError>>;

    /// Lists releases for a repository.
    fn list_releases<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<Release>, ClientError>>;

    /// Lists webhooks configured on a repository (needs admin access).
    fn list_hooks<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<Hook>, ClientError>>;

    /// Lists published security advisories for a repository.
    fn list_security_advisories<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<SecurityAdvisory>, ClientError>>;

    /// Lists all branches for a repository.
    fn list_branches<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<Branch>, ClientError>>;

    /// Returns the protection rules of a single branch, as the API sent them.
    ///
    /// GitHub answers 403 without admin access and 404 for a branch without
    /// protection rules.
    fn get_branch_protection<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        branch: &'a str,
    ) -> BoxFuture<'a, Result<Value, ClientError>>;

    /// Lists the rulesets that apply to a repository (its own and inherited).
    fn list_rulesets<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<Ruleset>, ClientError>>;

    /// Returns one ruleset with its rules, as the API sent it.
    fn get_ruleset<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        ruleset_id: u64,
    ) -> BoxFuture<'a, Result<Value, ClientError>>;

    /// Lists the comments on commits of a repository.
    fn list_commit_comments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<CommitComment>, ClientError>>;

    // ── Assets ────────────────────────────────────────────────────────────

    /// Streams a release asset into `sink` and returns the number of bytes
    /// delivered.
    ///
    /// The `Authorization` header is sent to `asset_url` only; a redirect to
    /// another host, port or scheme (GitHub redirects to its storage) is
    /// followed *without* it, and a redirect from HTTPS to HTTP is refused.
    /// Nothing is buffered: the body goes to the sink chunk by chunk.
    fn download_release_asset<'a>(
        &'a self,
        asset_url: &'a str,
        sink: &'a mut dyn AssetSink,
    ) -> BoxFuture<'a, Result<u64, ClientError>>;

    /// Streams a file attached to an issue, pull request, comment or release
    /// note into `sink`.
    ///
    /// `url` is an attachment URL as it appears in a body.  The credential is
    /// sent, as HTTP Basic authentication, to `https://github.com` only and
    /// only by a client for github.com; every other host, and every redirect
    /// that leaves the origin, gets none.  Nothing is buffered.
    fn download_attachment<'a>(
        &'a self,
        url: &'a str,
        sink: &'a mut dyn AssetSink,
    ) -> BoxFuture<'a, Result<AttachmentDownload, ClientError>>;

    // ── Deploy keys and collaborators ─────────────────────────────────────

    /// Lists deploy keys configured on a repository (needs admin access).
    fn list_deploy_keys<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<DeployKey>, ClientError>>;

    /// Lists collaborators on a repository (needs push access).
    fn list_collaborators<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<Collaborator>, ClientError>>;

    // ── Organisation data ─────────────────────────────────────────────────

    /// Lists members of a GitHub organisation.
    fn list_org_members<'a>(
        &'a self,
        org: &'a str,
    ) -> BoxFuture<'a, Result<Page<User>, ClientError>>;

    /// Lists teams in a GitHub organisation.
    fn list_org_teams<'a>(&'a self, org: &'a str)
        -> BoxFuture<'a, Result<Page<Team>, ClientError>>;

    // ── GitHub Actions ────────────────────────────────────────────────────

    /// Lists GitHub Actions workflows defined in a repository.
    fn list_workflows<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<Workflow>, ClientError>>;

    /// Lists the runs of a specific workflow, following every page.
    fn list_workflow_runs<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        workflow_id: u64,
    ) -> BoxFuture<'a, Result<Page<WorkflowRun>, ClientError>>;

    // ── Deployment environments ───────────────────────────────────────────

    /// Lists deployment environments configured on a repository.
    fn list_environments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Page<Environment>, ClientError>>;

    // ── Projects (v2) ─────────────────────────────────────────────────────

    /// Lists the Projects (v2) of a user (`org == false`) or an organisation.
    fn list_projects<'a>(
        &'a self,
        owner: &'a str,
        org: bool,
    ) -> BoxFuture<'a, Result<Page<ProjectV2>, ClientError>>;

    /// Lists the fields of one project.
    fn list_project_fields<'a>(
        &'a self,
        owner: &'a str,
        org: bool,
        project_number: u64,
    ) -> BoxFuture<'a, Result<Page<ProjectV2Field>, ClientError>>;

    /// Lists the items of one project with the values of `field_ids` (GitHub
    /// accepts at most 50; without any it returns only the title).
    fn list_project_items<'a>(
        &'a self,
        owner: &'a str,
        org: bool,
        project_number: u64,
        field_ids: &'a [u64],
    ) -> BoxFuture<'a, Result<Page<ProjectV2Item>, ClientError>>;

    // ── Discussions (GraphQL) ─────────────────────────────────────────────

    /// Lists the discussion categories of a repository (GraphQL `Repository.discussionCategories`,
    /// every page).
    ///
    /// GraphQL failures are [`ClientError::GraphQl`]; its `kinds` tell a
    /// repository that cannot be seen (`NOT_FOUND`) from a refusal
    /// (`FORBIDDEN`).
    fn list_discussion_categories<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Value>, ClientError>>;

    /// Returns one page of a repository's discussions, most recently updated
    /// first (`orderBy: {field: UPDATED_AT, direction: DESC}`), after the
    /// cursor `after` (`None`: the first page).  Each discussion's labels and
    /// poll options are complete; its comments are only counted.
    fn list_discussions_page<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        after: Option<&'a str>,
    ) -> BoxFuture<'a, Result<NodePage, ClientError>>;

    /// Lists every comment of the discussion with GraphQL node id
    /// `discussion_id`, each with all of its replies.
    fn list_discussion_comments<'a>(
        &'a self,
        discussion_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Value>, ClientError>>;

    // ── GitHub Packages ───────────────────────────────────────────────────

    /// Lists the packages of one ecosystem owned by a user (`org == false`;
    /// the token owner's own listing includes private packages) or an
    /// organisation.  Requires the `read:packages` scope.
    fn list_packages<'a>(
        &'a self,
        owner: &'a str,
        org: bool,
        package_type: &'a str,
    ) -> BoxFuture<'a, Result<Page<Package>, ClientError>>;

    /// Lists versions of a specific package.
    fn list_package_versions<'a>(
        &'a self,
        owner: &'a str,
        org: bool,
        package_type: &'a str,
        package_name: &'a str,
    ) -> BoxFuture<'a, Result<Page<PackageVersion>, ClientError>>;

    // ── Credentials ───────────────────────────────────────────────────────

    /// The access token in use, if any.
    ///
    /// The engine hands it to git (through a `GIT_ASKPASS` helper, never on the
    /// command line) so HTTPS clones of private repositories authenticate.
    /// The default, for clients without a token, is `None`.
    fn token(&self) -> Option<String> {
        None
    }

    /// The credential the engine hands to git for HTTPS remotes; the git
    /// runner asks it for the token each time it starts `git`, so a renewed
    /// GitHub App installation token reaches every later process.  The
    /// default is the fixed [`token`](Self::token).
    fn git_token(&self) -> Option<crate::GitToken> {
        self.token().map(crate::GitToken::Fixed)
    }
}
