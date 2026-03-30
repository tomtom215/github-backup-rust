// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Blanket `impl BackupClient for GitHubClient`.
//!
//! Each method simply wraps the corresponding inherent method on
//! [`GitHubClient`] in a `Box::pin(...)` future, satisfying the
//! object-safe [`BackupClient`] trait contract.

use bytes::Bytes;

use github_backup_types::{
    Branch, BranchProtection, ClassicProject, Collaborator, DeployKey, Discussion,
    DiscussionComment, Environment, Gist, Hook, Issue, IssueComment, IssueEvent, Label, Milestone,
    Package, PackageVersion, ProjectColumn, PullRequest, PullRequestComment, PullRequestCommit,
    PullRequestReview, Release, Repository, SecurityAdvisory, Team, User, Workflow, WorkflowRun,
};

use crate::error::ClientError;
use crate::GitHubClient;

use super::{BackupClient, BoxFuture};

impl BackupClient for GitHubClient {
    fn list_user_repos<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Repository>, ClientError>> {
        Box::pin(GitHubClient::list_user_repos(self, username))
    }

    fn list_org_repos<'a>(
        &'a self,
        org: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Repository>, ClientError>> {
        Box::pin(GitHubClient::list_org_repos(self, org))
    }

    fn list_followers<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<User>, ClientError>> {
        Box::pin(GitHubClient::list_followers(self, username))
    }

    fn list_following<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<User>, ClientError>> {
        Box::pin(GitHubClient::list_following(self, username))
    }

    fn list_starred<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Repository>, ClientError>> {
        Box::pin(GitHubClient::list_starred(self, username))
    }

    fn list_watched<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Repository>, ClientError>> {
        Box::pin(GitHubClient::list_watched(self, username))
    }

    fn list_gists<'a>(
        &'a self,
        username: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Gist>, ClientError>> {
        Box::pin(GitHubClient::list_gists(self, username))
    }

    fn list_starred_gists<'a>(&'a self) -> BoxFuture<'a, Result<Vec<Gist>, ClientError>> {
        Box::pin(GitHubClient::list_starred_gists(self))
    }

    fn list_issues<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        since: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<Issue>, ClientError>> {
        Box::pin(GitHubClient::list_issues(self, owner, repo, since))
    }

    fn list_issue_comments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        issue_number: u64,
    ) -> BoxFuture<'a, Result<Vec<IssueComment>, ClientError>> {
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
    ) -> BoxFuture<'a, Result<Vec<IssueEvent>, ClientError>> {
        Box::pin(GitHubClient::list_issue_events(
            self,
            owner,
            repo,
            issue_number,
        ))
    }

    fn list_pull_requests<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        since: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<PullRequest>, ClientError>> {
        Box::pin(GitHubClient::list_pull_requests(self, owner, repo, since))
    }

    fn list_pull_comments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> BoxFuture<'a, Result<Vec<PullRequestComment>, ClientError>> {
        Box::pin(GitHubClient::list_pull_comments(
            self, owner, repo, pr_number,
        ))
    }

    fn list_pull_commits<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> BoxFuture<'a, Result<Vec<PullRequestCommit>, ClientError>> {
        Box::pin(GitHubClient::list_pull_commits(
            self, owner, repo, pr_number,
        ))
    }

    fn list_pull_reviews<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        pr_number: u64,
    ) -> BoxFuture<'a, Result<Vec<PullRequestReview>, ClientError>> {
        Box::pin(GitHubClient::list_pull_reviews(
            self, owner, repo, pr_number,
        ))
    }

    fn list_labels<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Label>, ClientError>> {
        Box::pin(GitHubClient::list_labels(self, owner, repo))
    }

    fn list_milestones<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Milestone>, ClientError>> {
        Box::pin(GitHubClient::list_milestones(self, owner, repo))
    }

    fn list_releases<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Release>, ClientError>> {
        Box::pin(GitHubClient::list_releases(self, owner, repo))
    }

    fn list_hooks<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Hook>, ClientError>> {
        Box::pin(GitHubClient::list_hooks(self, owner, repo))
    }

    fn list_security_advisories<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<SecurityAdvisory>, ClientError>> {
        Box::pin(GitHubClient::list_security_advisories(self, owner, repo))
    }

    fn list_repo_topics<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<String>, ClientError>> {
        Box::pin(GitHubClient::list_repo_topics(self, owner, repo))
    }

    fn list_branches<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Branch>, ClientError>> {
        Box::pin(GitHubClient::list_branches(self, owner, repo))
    }

    fn get_branch_protection<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        branch: &'a str,
    ) -> BoxFuture<'a, Result<BranchProtection, ClientError>> {
        Box::pin(GitHubClient::get_branch_protection(self, owner, repo, branch))
    }

    fn download_release_asset<'a>(
        &'a self,
        asset_url: &'a str,
    ) -> BoxFuture<'a, Result<Bytes, ClientError>> {
        Box::pin(GitHubClient::download_release_asset(self, asset_url))
    }

    fn list_deploy_keys<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<DeployKey>, ClientError>> {
        Box::pin(GitHubClient::list_deploy_keys(self, owner, repo))
    }

    fn list_collaborators<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Collaborator>, ClientError>> {
        Box::pin(GitHubClient::list_collaborators(self, owner, repo))
    }

    fn list_org_members<'a>(
        &'a self,
        org: &'a str,
    ) -> BoxFuture<'a, Result<Vec<User>, ClientError>> {
        Box::pin(GitHubClient::list_org_members(self, org))
    }

    fn list_org_teams<'a>(&'a self, org: &'a str) -> BoxFuture<'a, Result<Vec<Team>, ClientError>> {
        Box::pin(GitHubClient::list_org_teams(self, org))
    }

    fn list_workflows<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Workflow>, ClientError>> {
        Box::pin(GitHubClient::list_workflows(self, owner, repo))
    }

    fn list_workflow_runs<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        workflow_id: u64,
    ) -> BoxFuture<'a, Result<Vec<WorkflowRun>, ClientError>> {
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
    ) -> BoxFuture<'a, Result<Vec<Environment>, ClientError>> {
        Box::pin(GitHubClient::list_environments(self, owner, repo))
    }

    fn list_discussions<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Discussion>, ClientError>> {
        Box::pin(GitHubClient::list_discussions(self, owner, repo))
    }

    fn list_discussion_comments<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
        discussion_number: u64,
    ) -> BoxFuture<'a, Result<Vec<DiscussionComment>, ClientError>> {
        Box::pin(GitHubClient::list_discussion_comments(
            self,
            owner,
            repo,
            discussion_number,
        ))
    }

    fn list_repo_projects<'a>(
        &'a self,
        owner: &'a str,
        repo: &'a str,
    ) -> BoxFuture<'a, Result<Vec<ClassicProject>, ClientError>> {
        Box::pin(GitHubClient::list_repo_projects(self, owner, repo))
    }

    fn list_project_columns<'a>(
        &'a self,
        project_id: u64,
    ) -> BoxFuture<'a, Result<Vec<ProjectColumn>, ClientError>> {
        Box::pin(GitHubClient::list_project_columns(self, project_id))
    }

    fn list_user_packages<'a>(
        &'a self,
        username: &'a str,
        package_type: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Package>, ClientError>> {
        Box::pin(GitHubClient::list_user_packages(
            self,
            username,
            package_type,
        ))
    }

    fn list_package_versions<'a>(
        &'a self,
        username: &'a str,
        package_type: &'a str,
        package_name: &'a str,
    ) -> BoxFuture<'a, Result<Vec<PackageVersion>, ClientError>> {
        Box::pin(GitHubClient::list_package_versions(
            self,
            username,
            package_type,
            package_name,
        ))
    }
}
