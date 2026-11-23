// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Blanket `impl BackupClient for GitHubClient`.
//!
//! Each method simply wraps the corresponding inherent method on
//! [`GitHubClient`] in a `Box::pin(...)` future, satisfying the
//! object-safe [`BackupClient`] trait contract.

use serde_json::Value;

use github_backup_types::{
    Branch, Collaborator, CommitComment, DeployKey, Environment, Gist, GistComment, Hook, Issue,
    IssueComment, IssueEvent, Label, Milestone, Package, PackageVersion, Page, ProjectV2,
    ProjectV2Field, ProjectV2Item, PullRequest, PullRequestComment, PullRequestCommit,
    PullRequestReview, Raw, Release, Repository, Ruleset, SecurityAdvisory, Team, User, Workflow,
    WorkflowRun,
};

use crate::error::ClientError;
use crate::GitHubClient;

use super::{AssetSink, AttachmentDownload, BackupClient, BoxFuture, NodePage};

/// `Result` of a boxed client future.
type Fut<'a, T> = BoxFuture<'a, Result<T, ClientError>>;

impl BackupClient for GitHubClient {
    fn list_user_repos<'a>(&'a self, username: &'a str) -> Fut<'a, Page<Repository>> {
        Box::pin(GitHubClient::list_user_repos(self, username))
    }

    fn list_org_repos<'a>(&'a self, org: &'a str) -> Fut<'a, Page<Repository>> {
        Box::pin(GitHubClient::list_org_repos(self, org))
    }

    fn get_repository<'a>(&'a self, owner: &'a str, repo: &'a str) -> Fut<'a, Raw<Repository>> {
        Box::pin(GitHubClient::get_repository(self, owner, repo))
    }

    fn list_followers<'a>(&'a self, username: &'a str) -> Fut<'a, Page<User>> {
        Box::pin(GitHubClient::list_followers(self, username))
    }

    fn list_following<'a>(&'a self, username: &'a str) -> Fut<'a, Page<User>> {
        Box::pin(GitHubClient::list_following(self, username))
    }

    fn list_starred<'a>(&'a self, username: &'a str) -> Fut<'a, Page<Repository>> {
        Box::pin(GitHubClient::list_starred(self, username))
    }

    fn list_watched<'a>(&'a self, username: &'a str) -> Fut<'a, Page<Repository>> {
        Box::pin(GitHubClient::list_watched(self, username))
    }

    fn list_gists<'a>(&'a self, username: &'a str) -> Fut<'a, Page<Gist>> {
        Box::pin(GitHubClient::list_gists(self, username))
    }

    fn list_starred_gists<'a>(&'a self) -> Fut<'a, Page<Gist>> {
        Box::pin(GitHubClient::list_starred_gists(self))
    }

    fn list_gist_comments<'a>(&'a self, gist_id: &'a str) -> Fut<'a, Page<GistComment>> {
        Box::pin(GitHubClient::list_gist_comments(self, gist_id))
    }

    fn list_issues<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        since: Option<&'a str>,
    ) -> Fut<'a, Page<Issue>> {
        Box::pin(GitHubClient::list_issues(self, owner, repo, since))
    }

    fn list_issue_comments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        issue_number: u64,
    ) -> Fut<'a, Page<IssueComment>> {
        Box::pin(GitHubClient::list_issue_comments(
            self,
            owner,
            repo,
            issue_number,
        ))
    }

    fn list_issue_events<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        issue_number: u64,
    ) -> Fut<'a, Page<IssueEvent>> {
        Box::pin(GitHubClient::list_issue_events(
            self,
            owner,
            repo,
            issue_number,
        ))
    }

    fn list_repo_issue_comments_since<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        since: &'a str,
    ) -> Fut<'a, Page<IssueComment>> {
        Box::pin(GitHubClient::list_repo_issue_comments_since(
            self, owner, repo, since,
        ))
    }

    fn list_pull_requests<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        updated_since: Option<&'a str>,
    ) -> Fut<'a, Page<PullRequest>> {
        Box::pin(GitHubClient::list_pull_requests(
            self,
            owner,
            repo,
            updated_since,
        ))
    }

    fn list_pull_comments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> Fut<'a, Page<PullRequestComment>> {
        Box::pin(GitHubClient::list_pull_comments(
            self, owner, repo, pr_number,
        ))
    }

    fn list_pull_commits<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> Fut<'a, Page<PullRequestCommit>> {
        Box::pin(GitHubClient::list_pull_commits(
            self, owner, repo, pr_number,
        ))
    }

    fn compare_commits<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        base: &'a str,
        head: &'a str,
    ) -> Fut<'a, Page<PullRequestCommit>> {
        Box::pin(GitHubClient::compare_commits(self, owner, repo, base, head))
    }

    fn list_pull_reviews<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> Fut<'a, Page<PullRequestReview>> {
        Box::pin(GitHubClient::list_pull_reviews(
            self, owner, repo, pr_number,
        ))
    }

    fn list_repo_pull_comments_since<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        since: &'a str,
    ) -> Fut<'a, Page<PullRequestComment>> {
        Box::pin(GitHubClient::list_repo_pull_comments_since(
            self, owner, repo, since,
        ))
    }

    fn list_labels<'a>(&'a self, owner: &'a str, repo: &'a str) -> Fut<'a, Page<Label>> {
        Box::pin(GitHubClient::list_labels(self, owner, repo))
    }

    fn list_milestones<'a>(&'a self, owner: &'a str, repo: &'a str) -> Fut<'a, Page<Milestone>> {
        Box::pin(GitHubClient::list_milestones(self, owner, repo))
    }

    fn list_releases<'a>(&'a self, owner: &'a str, repo: &'a str) -> Fut<'a, Page<Release>> {
        Box::pin(GitHubClient::list_releases(self, owner, repo))
    }

    fn list_hooks<'a>(&'a self, owner: &'a str, repo: &'a str) -> Fut<'a, Page<Hook>> {
        Box::pin(GitHubClient::list_hooks(self, owner, repo))
    }

    fn list_security_advisories<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> Fut<'a, Page<SecurityAdvisory>> {
        Box::pin(GitHubClient::list_security_advisories(self, owner, repo))
    }

    fn list_branches<'a>(&'a self, owner: &'a str, repo: &'a str) -> Fut<'a, Page<Branch>> {
        Box::pin(GitHubClient::list_branches(self, owner, repo))
    }

    fn get_branch_protection<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        branch: &'a str,
    ) -> Fut<'a, Value> {
        Box::pin(GitHubClient::get_branch_protection(
            self, owner, repo, branch,
        ))
    }

    fn list_rulesets<'a>(&'a self, owner: &'a str, repo: &'a str) -> Fut<'a, Page<Ruleset>> {
        Box::pin(GitHubClient::list_rulesets(self, owner, repo))
    }

    fn get_ruleset<'a>(&'a self, owner: &'a str, repo: &'a str, ruleset_id: u64) -> Fut<'a, Value> {
        Box::pin(GitHubClient::get_ruleset(self, owner, repo, ruleset_id))
    }

    fn list_commit_comments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> Fut<'a, Page<CommitComment>> {
        Box::pin(GitHubClient::list_commit_comments(self, owner, repo))
    }

    fn download_release_asset<'a>(
        &'a self,
        asset_url: &'a str,
        sink: &'a mut dyn AssetSink,
    ) -> Fut<'a, u64> {
        Box::pin(GitHubClient::download_release_asset(self, asset_url, sink))
    }

    fn download_attachment<'a>(
        &'a self,
        url: &'a str,
        sink: &'a mut dyn AssetSink,
    ) -> Fut<'a, AttachmentDownload> {
        Box::pin(GitHubClient::download_attachment(self, url, sink))
    }

    fn list_deploy_keys<'a>(&'a self, owner: &'a str, repo: &'a str) -> Fut<'a, Page<DeployKey>> {
        Box::pin(GitHubClient::list_deploy_keys(self, owner, repo))
    }

    fn list_collaborators<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> Fut<'a, Page<Collaborator>> {
        Box::pin(GitHubClient::list_collaborators(self, owner, repo))
    }

    fn list_org_members<'a>(&'a self, org: &'a str) -> Fut<'a, Page<User>> {
        Box::pin(GitHubClient::list_org_members(self, org))
    }

    fn list_org_teams<'a>(&'a self, org: &'a str) -> Fut<'a, Page<Team>> {
        Box::pin(GitHubClient::list_org_teams(self, org))
    }

    fn list_workflows<'a>(&'a self, owner: &'a str, repo: &'a str) -> Fut<'a, Page<Workflow>> {
        Box::pin(GitHubClient::list_workflows(self, owner, repo))
    }

    fn list_workflow_runs<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        workflow_id: u64,
    ) -> Fut<'a, Page<WorkflowRun>> {
        Box::pin(GitHubClient::list_workflow_runs(
            self,
            owner,
            repo,
            workflow_id,
        ))
    }

    fn list_environments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> Fut<'a, Page<Environment>> {
        Box::pin(GitHubClient::list_environments(self, owner, repo))
    }

    fn list_projects<'a>(&'a self, owner: &'a str, org: bool) -> Fut<'a, Page<ProjectV2>> {
        Box::pin(GitHubClient::list_projects(self, owner, org))
    }

    fn list_project_fields<'a>(
        &'a self,
        owner: &'a str,
        org: bool,
        project_number: u64,
    ) -> Fut<'a, Page<ProjectV2Field>> {
        Box::pin(GitHubClient::list_project_fields(
            self,
            owner,
            org,
            project_number,
        ))
    }

    fn list_project_items<'a>(
        &'a self,
        owner: &'a str,
        org: bool,
        project_number: u64,
        field_ids: &'a [u64],
    ) -> Fut<'a, Page<ProjectV2Item>> {
        Box::pin(GitHubClient::list_project_items(
            self,
            owner,
            org,
            project_number,
            field_ids,
        ))
    }

    fn list_discussion_categories<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> Fut<'a, Vec<Value>> {
        Box::pin(GitHubClient::list_discussion_categories(self, owner, repo))
    }

    fn list_discussions_page<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        after: Option<&'a str>,
    ) -> Fut<'a, NodePage> {
        Box::pin(GitHubClient::list_discussions_page(
            self, owner, repo, after,
        ))
    }

    fn list_discussion_comments<'a>(&'a self, discussion_id: &'a str) -> Fut<'a, Vec<Value>> {
        Box::pin(GitHubClient::list_discussion_comments(self, discussion_id))
    }

    fn list_packages<'a>(
        &'a self,
        owner: &'a str,
        org: bool,
        package_type: &'a str,
    ) -> Fut<'a, Page<Package>> {
        Box::pin(GitHubClient::list_packages(self, owner, org, package_type))
    }

    fn list_package_versions<'a>(
        &'a self,
        owner: &'a str,
        org: bool,
        package_type: &'a str,
        package_name: &'a str,
    ) -> Fut<'a, Page<PackageVersion>> {
        Box::pin(GitHubClient::list_package_versions(
            self,
            owner,
            org,
            package_type,
            package_name,
        ))
    }

    fn token(&self) -> Option<String> {
        GitHubClient::token(self)
    }

    fn git_token(&self) -> Option<crate::GitToken> {
        GitHubClient::git_token(self)
    }
}
