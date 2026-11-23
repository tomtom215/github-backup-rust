// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository metadata listing endpoints.
//!
//! Covers labels, milestones, releases, hooks, security advisories,
//! branches, branch protection, rulesets and commit comments.

use serde_json::Value;

use github_backup_types::{
    Branch, CommitComment, Hook, Label, Milestone, Page, Release, Ruleset, SecurityAdvisory,
};

use crate::error::ClientError;

use super::super::{GitHubClient, PER_PAGE};
use super::encode_path;

impl GitHubClient {
    // ── Repository metadata ───────────────────────────────────────────────

    /// Lists labels for a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_labels(&self, owner: &str, repo: &str) -> Result<Page<Label>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/labels?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists milestones for a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_milestones(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Page<Milestone>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/milestones?state=all&per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists releases for a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_releases(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Page<Release>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/releases?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists webhooks configured on a repository.
    ///
    /// Requires `admin` permission on the repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_hooks(&self, owner: &str, repo: &str) -> Result<Page<Hook>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/hooks?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists published security advisories for a repository.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_security_advisories(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Page<SecurityAdvisory>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/security-advisories?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Lists all branches for a repository.
    ///
    /// Returns branch names, their tip commit SHAs, and whether each branch
    /// has protection rules enabled.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_branches(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Page<Branch>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/branches?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns the protection rules of one branch exactly as the API sends them
    /// (`GET /repos/{owner}/{repo}/branches/{branch}/protection`).
    ///
    /// The branch name is percent-encoded except for `/`, which GitHub
    /// declares part of the route (`release#2` must not request `release`).
    /// GitHub answers 403 without admin access and 404 for a branch without
    /// protection.
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn get_branch_protection(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Value, ClientError> {
        let api = self.api();
        let branch = encode_path(branch);
        let url = format!("{api}/repos/{owner}/{repo}/branches/{branch}/protection");
        Ok(self.get_json_with_link::<Value>(&url).await?.0)
    }

    /// Lists the rulesets that apply to a repository, its own and those it
    /// inherits from its organisation (`GET /repos/{owner}/{repo}/rulesets`).
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_rulesets(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Page<Ruleset>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/rulesets?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }

    /// Returns one ruleset with its rules and conditions
    /// (`GET /repos/{owner}/{repo}/rulesets/{ruleset_id}`).
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn get_ruleset(
        &self,
        owner: &str,
        repo: &str,
        ruleset_id: u64,
    ) -> Result<Value, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/rulesets/{ruleset_id}");
        Ok(self.get_json_with_link::<Value>(&url).await?.0)
    }

    /// Lists the comments on commits of a repository
    /// (`GET /repos/{owner}/{repo}/comments`).
    ///
    /// # Errors
    ///
    /// Propagates [`ClientError`] on network, TLS, or API errors.
    pub async fn list_commit_comments(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Page<CommitComment>, ClientError> {
        let api = self.api();
        let url = format!("{api}/repos/{owner}/{repo}/comments?per_page={PER_PAGE}");
        self.get_all_pages(&url).await
    }
}
