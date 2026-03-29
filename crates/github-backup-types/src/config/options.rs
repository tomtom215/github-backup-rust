// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Backup target selection and per-category enable/disable flags.

use serde::{Deserialize, Serialize};

use super::clone_type::CloneType;

/// Whether the backup target is a user account or an organisation.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BackupTarget {
    /// GitHub user account (default).
    #[default]
    User,
    /// GitHub organisation.
    Org,
}

/// Selects which data categories to include in the backup.
///
/// All fields default to `false`; enable only what you need.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackupOptions {
    // ── Target ────────────────────────────────────────────────────────────
    /// Whether the target is a user account or an organisation.
    pub target: BackupTarget,

    // ── Repository git clones ──────────────────────────────────────────────
    /// Clone/mirror all repositories (default: true when not overridden).
    pub repositories: bool,
    /// Include forked repositories.
    pub forks: bool,
    /// Include private repositories (requires appropriate token scope).
    pub private: bool,
    /// Clone using SSH URLs instead of HTTPS.
    pub prefer_ssh: bool,
    /// How to clone repositories.
    ///
    /// Defaults to [`CloneType::Mirror`] for complete, restorable backups.
    #[serde(default)]
    pub clone_type: CloneType,
    /// Enable Git LFS when cloning.
    pub lfs: bool,
    /// Do not prune deleted remote refs during updates.
    pub no_prune: bool,

    // ── Issues ────────────────────────────────────────────────────────────
    /// Backup issue metadata (title, body, state, labels, etc.).
    pub issues: bool,
    /// Backup issue comment threads.
    pub issue_comments: bool,
    /// Backup issue timeline events.
    pub issue_events: bool,

    // ── Pull Requests ─────────────────────────────────────────────────────
    /// Backup pull request metadata.
    pub pulls: bool,
    /// Backup pull request review comments.
    pub pull_comments: bool,
    /// Backup pull request commit lists.
    pub pull_commits: bool,
    /// Backup pull request reviews (requires an extra API call per PR).
    pub pull_reviews: bool,

    // ── Repository metadata ───────────────────────────────────────────────
    /// Backup repository labels.
    pub labels: bool,
    /// Backup repository milestones.
    pub milestones: bool,
    /// Backup repository releases (metadata + asset download).
    pub releases: bool,
    /// Backup release binary assets.
    pub release_assets: bool,
    /// Backup repository webhook configurations.
    pub hooks: bool,
    /// Backup security advisories (public repositories only).
    pub security_advisories: bool,
    /// Clone repository wikis.
    pub wikis: bool,

    // ── User / organisation data ──────────────────────────────────────────
    /// Backup the list of repositories starred by the target user (JSON list).
    pub starred: bool,
    /// Clone every starred repository as a bare mirror.
    ///
    /// Uses a durable queue at
    /// `<output>/<owner>/json/starred_clone_queue.json` that persists across
    /// runs, enabling pause and resume.  Re-running with this flag set will
    /// continue from where the previous run stopped.
    ///
    /// Not included in [`BackupOptions::all`] because it can consume
    /// substantial disk space and time for users with many starred repos.
    pub clone_starred: bool,
    /// Backup the list of repositories watched by the target user.
    pub watched: bool,
    /// Backup the target user's follower list.
    pub followers: bool,
    /// Backup the target user's following list.
    pub following: bool,
    /// Backup gists owned by the target user.
    pub gists: bool,
    /// Backup gists starred by the target user.
    pub starred_gists: bool,

    // ── Additional repository metadata ────────────────────────────────────
    /// Backup the list of repository topics (tags).
    pub topics: bool,
    /// Backup the list of repository branches and their protection status.
    pub branches: bool,
    /// Backup the deploy keys configured on each repository.
    ///
    /// Requires admin access to the repository; non-admin repos are skipped
    /// with an informational log message (not an error).
    pub deploy_keys: bool,
    /// Backup the list of repository collaborators and their permissions.
    ///
    /// Requires admin access to the repository; non-admin repos are skipped
    /// with an informational log message.
    pub collaborators: bool,

    // ── Organisation data ─────────────────────────────────────────────────
    /// Backup the member list of the organisation.
    ///
    /// Only meaningful when [`target`] is [`BackupTarget::Org`]; ignored for
    /// user targets.
    ///
    /// [`target`]: BackupOptions::target
    pub org_members: bool,
    /// Backup the team list of the organisation.
    ///
    /// Only meaningful when [`target`] is [`BackupTarget::Org`]; ignored for
    /// user targets.
    ///
    /// [`target`]: BackupOptions::target
    pub org_teams: bool,

    // ── GitHub Actions ────────────────────────────────────────────────────
    /// Backup the list of GitHub Actions workflows defined in each repository.
    ///
    /// Saves `workflows.json` to each repository's metadata directory.
    /// Workflow YAML files are already captured by the git clone; this option
    /// additionally records the API metadata (IDs, states, badge URLs).
    pub actions: bool,
    /// Backup recent workflow run history for each Actions workflow.
    ///
    /// For each workflow, saves `workflow_runs_<id>.json`.  Requires
    /// [`actions`] to be enabled; run history can be very large for active
    /// repositories.
    ///
    /// [`actions`]: BackupOptions::actions
    pub action_runs: bool,

    // ── Deployment environments ───────────────────────────────────────────
    /// Backup deployment environment configurations for each repository.
    ///
    /// Environments (e.g. `staging`, `production`) may have protection rules,
    /// required reviewers, and branch policies.  Saves `environments.json`
    /// to each repository's metadata directory.
    pub environments: bool,

    // ── Repository name filters ───────────────────────────────────────────
    /// Only back up repositories whose names match at least one of these glob
    /// patterns.  An empty list means *all* repositories are included.
    ///
    /// Pattern syntax: `*` matches any sequence of characters, `?` matches
    /// exactly one character.  Matching is case-insensitive.
    ///
    /// # Example
    ///
    /// ```
    /// # use github_backup_types::config::BackupOptions;
    /// let opts = BackupOptions {
    ///     include_repos: vec!["rust-*".to_string(), "my-repo".to_string()],
    ///     ..Default::default()
    /// };
    /// ```
    pub include_repos: Vec<String>,

    /// Exclude repositories whose names match at least one of these glob
    /// patterns.  Takes precedence over [`include_repos`].
    ///
    /// [`include_repos`]: BackupOptions::include_repos
    pub exclude_repos: Vec<String>,

    // ── Incremental filter ────────────────────────────────────────────────
    /// Only fetch issues and pull requests updated *at or after* this ISO 8601
    /// timestamp (e.g. `"2024-01-01T00:00:00Z"`).
    ///
    /// Useful for incremental backups: run a full backup once, then pass the
    /// previous run's start time to limit subsequent API calls.
    pub since: Option<String>,

    // ── Clone URL override ────────────────────────────────────────────────
    /// Override the hostname used in git clone URLs.
    ///
    /// Useful for GitHub Enterprise Server deployments where the API host
    /// and the git clone host differ (e.g. behind separate load balancers).
    ///
    /// When set, the hostname component of every `clone_url` and `ssh_url`
    /// from the API is rewritten to this value before it is passed to git.
    ///
    /// Example: `--api-url https://github-api.example.com/api/v3
    ///            --clone-host github-git.example.com`
    pub clone_host: Option<String>,

    // ── Execution options ─────────────────────────────────────────────────
    /// When `true`, log what would be done without writing any files or
    /// running any git commands.
    pub dry_run: bool,
    /// Maximum number of repositories to back up concurrently.
    /// `1` means fully sequential (safe default).
    pub concurrency: usize,
}

impl BackupOptions {
    /// Returns a configuration that enables every available backup category.
    ///
    /// Equivalent to the `--all` flag in the Python reference implementation,
    /// but also enables `starred_gists`, `pull_reviews`, `topics`, and
    /// `branches`.  Repository name filters and `since` are left at their
    /// defaults (no filtering).
    ///
    /// Note that `clone_starred` and `action_runs` are **not** enabled because
    /// they can generate very large outputs and are considered opt-in.
    #[must_use]
    pub fn all() -> Self {
        Self {
            target: BackupTarget::User,
            repositories: true,
            forks: true,
            private: true,
            prefer_ssh: false,
            clone_type: CloneType::Mirror,
            lfs: false,
            no_prune: false,
            issues: true,
            issue_comments: true,
            issue_events: true,
            pulls: true,
            pull_comments: true,
            pull_commits: true,
            pull_reviews: true,
            labels: true,
            milestones: true,
            releases: true,
            release_assets: true,
            hooks: true,
            security_advisories: true,
            wikis: true,
            starred: true,
            clone_starred: false, // opt-in only; can be very large
            watched: true,
            followers: true,
            following: true,
            gists: true,
            starred_gists: true,
            topics: true,
            branches: true,
            deploy_keys: true,
            collaborators: true,
            org_members: true,
            org_teams: true,
            actions: true,
            action_runs: false, // opt-in only; can generate very large files
            environments: true,
            include_repos: vec![],
            exclude_repos: vec![],
            since: None,
            clone_host: None,
            dry_run: false,
            concurrency: 4,
        }
    }
}
