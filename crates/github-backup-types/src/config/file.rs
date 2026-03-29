// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! TOML configuration file schema and parser.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// TOML configuration file schema.
///
/// Load from disk with [`ConfigFile::from_toml_str`] or
/// [`ConfigFile::from_path`].  All fields are optional; missing fields fall
/// back to CLI defaults.  A typical minimal config looks like:
///
/// ```toml
/// owner = "octocat"
/// output = "/var/backup/github"
/// concurrency = 8
///
/// repositories = true
/// issues = true
/// pulls = true
/// releases = true
/// wikis = true
/// ```
///
/// # Token security
///
/// Storing tokens in config files is convenient but less secure than providing
/// them via the `GITHUB_TOKEN` environment variable.  Prefer environment
/// variables in automated environments; restrict config file permissions to
/// `0600` when a token must be stored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigFile {
    /// GitHub username or organisation name to back up.
    pub owner: Option<String>,

    /// GitHub personal access token.
    ///
    /// Prefer the `GITHUB_TOKEN` environment variable over storing tokens here.
    pub token: Option<String>,

    /// Override the GitHub API base URL for GitHub Enterprise Server.
    ///
    /// Example: `"https://github.example.com/api/v3"`.
    /// Defaults to `https://api.github.com` when absent.
    pub api_url: Option<String>,

    /// Root directory where backup artefacts will be written.
    pub output: Option<PathBuf>,

    /// Maximum number of repositories to back up concurrently (default: 4).
    pub concurrency: Option<usize>,

    /// Treat `owner` as a GitHub organisation.
    pub org: Option<bool>,

    // ── Category flags ─────────────────────────────────────────────────────
    /// Enable all backup categories.
    pub all: Option<bool>,

    /// Clone/mirror repositories.
    pub repositories: Option<bool>,
    /// Include forked repositories.
    pub forks: Option<bool>,
    /// Include private repositories.
    pub private: Option<bool>,
    /// Back up issue metadata.
    pub issues: Option<bool>,
    /// Back up issue comment threads.
    pub issue_comments: Option<bool>,
    /// Back up issue timeline events.
    pub issue_events: Option<bool>,
    /// Back up pull request metadata.
    pub pulls: Option<bool>,
    /// Back up pull request review comments.
    pub pull_comments: Option<bool>,
    /// Back up pull request commit lists.
    pub pull_commits: Option<bool>,
    /// Back up pull request reviews.
    pub pull_reviews: Option<bool>,
    /// Back up repository labels.
    pub labels: Option<bool>,
    /// Back up repository milestones.
    pub milestones: Option<bool>,
    /// Back up release metadata.
    pub releases: Option<bool>,
    /// Download release binary assets.
    pub release_assets: Option<bool>,
    /// Back up webhook configurations.
    pub hooks: Option<bool>,
    /// Back up published security advisories.
    pub security_advisories: Option<bool>,
    /// Clone repository wikis.
    pub wikis: Option<bool>,
    /// Back up starred repositories (JSON list).
    pub starred: Option<bool>,
    /// Clone every starred repository as a git mirror.
    pub clone_starred: Option<bool>,
    /// Back up watched repositories.
    pub watched: Option<bool>,
    /// Back up follower list.
    pub followers: Option<bool>,
    /// Back up following list.
    pub following: Option<bool>,
    /// Back up owned gists.
    pub gists: Option<bool>,
    /// Back up starred gists.
    pub starred_gists: Option<bool>,

    /// Back up repository topics.
    pub topics: Option<bool>,
    /// Back up the list of repository branches.
    pub branches: Option<bool>,
    /// Back up deploy keys for each repository.
    pub deploy_keys: Option<bool>,
    /// Back up the list of repository collaborators.
    pub collaborators: Option<bool>,
    /// Back up the member list of the organisation.
    pub org_members: Option<bool>,
    /// Back up the team list of the organisation.
    pub org_teams: Option<bool>,

    /// Back up GitHub Actions workflow metadata.
    pub actions: Option<bool>,
    /// Back up GitHub Actions workflow run history.
    pub action_runs: Option<bool>,
    /// Back up repository deployment environment configurations.
    pub environments: Option<bool>,

    /// Only back up repositories matching these glob patterns (comma-separated
    /// or as a TOML array).
    pub include_repos: Option<Vec<String>>,
    /// Exclude repositories matching these glob patterns.
    pub exclude_repos: Option<Vec<String>>,

    /// Only fetch issues/PRs updated at or after this ISO 8601 timestamp.
    pub since: Option<String>,

    /// Override the hostname used in git clone URLs.
    ///
    /// Useful for GHES deployments where the API host and git clone host differ.
    pub clone_host: Option<String>,
}

impl ConfigFile {
    /// Parses a [`ConfigFile`] from a TOML string.
    ///
    /// # Errors
    ///
    /// Returns an error string if the TOML is malformed or contains
    /// type-incompatible values.
    pub fn from_toml_str(s: &str) -> Result<Self, String> {
        toml::from_str(s).map_err(|e| e.to_string())
    }

    /// Reads and parses a [`ConfigFile`] from a file on disk.
    ///
    /// # Errors
    ///
    /// Returns an error string if the file cannot be read or contains invalid
    /// TOML.
    pub fn from_path(path: &std::path::Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read config file '{}': {e}", path.display()))?;
        Self::from_toml_str(&content)
            .map_err(|e| format!("invalid config file '{}': {e}", path.display()))
    }
}
