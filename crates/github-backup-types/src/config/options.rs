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
    /// Delete branches and tags from the local clone when they were deleted
    /// on the remote.  Off by default, so a backup keeps what GitHub no longer
    /// has.
    pub prune: bool,

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
    /// Backup the list of repository topics (tags), taken from the repository
    /// object.
    pub topics: bool,
    /// Backup the repository branches, the protection rules of protected
    /// branches and the repository rulesets.
    pub branches: bool,
    /// Backup the deploy keys configured on each repository.
    ///
    /// Requires admin access to the repository; see
    /// [`tolerate_forbidden`](BackupOptions::tolerate_forbidden) for what a
    /// refusal means.
    pub deploy_keys: bool,
    /// Backup the list of repository collaborators and their permissions.
    ///
    /// Requires push access to the repository; see
    /// [`tolerate_forbidden`](BackupOptions::tolerate_forbidden) for what a
    /// refusal means.
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

    // ── GitHub Discussions ────────────────────────────────────────────────
    /// Requested backup of GitHub Discussions.
    ///
    /// GitHub serves Discussions through GraphQL only and the backup does not
    /// speak GraphQL yet, so **nothing is backed up**: the run logs one
    /// warning saying so.  Not included in [`BackupOptions::all`].
    pub discussions: bool,

    // ── Projects ──────────────────────────────────────────────────────────
    /// Backup the owner's Projects (v2): each project, its fields and its
    /// items with their field values.
    ///
    /// Saves `projects/projects.json` and per-project `fields` / `items` files
    /// in the owner's JSON directory.  Classic projects (v1) were sunset by
    /// GitHub and are not backed up.
    pub projects: bool,

    // ── Commit comments ───────────────────────────────────────────────────
    /// Backup the comments made on commits of each repository
    /// (`commit_comments.json`).
    pub commit_comments: bool,

    // ── Attachments ───────────────────────────────────────────────────────
    /// Download the files and images attached to issues, pull requests,
    /// their comments and reviews, commit comments and release notes, found
    /// in the stored bodies (`attachments/` in each repository's metadata
    /// directory).
    pub attachments: bool,

    // ── GitHub Packages ───────────────────────────────────────────────────
    /// Backup GitHub Packages metadata for the target user or organisation.
    ///
    /// Requires the `read:packages` OAuth scope.  Iterates over all supported
    /// package ecosystems (container, npm, maven, etc.) and saves
    /// `packages_<type>.json` and version files to the owner's JSON directory.
    pub packages: bool,

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

    // ── Incremental behaviour ─────────────────────────────────────────────
    /// Treat everything last updated *before* this ISO 8601 timestamp (for
    /// example `"2024-01-01T00:00:00Z"`) as already backed up, for every
    /// repository.
    ///
    /// Issue and pull request **lists are always fetched in full** and merged
    /// into what is already stored, so this never loses data; it only skips
    /// re-fetching the comments, events, commits and reviews of items that have
    /// not changed since the timestamp *and* whose files already exist.  Without
    /// it the engine uses each repository's own watermark from the previous
    /// run (see [`BackupState`](crate::BackupState)).
    pub since: Option<String>,

    /// Ignore incremental state: fetch every item's comments, events, commits
    /// and reviews again, whatever the watermarks say.  Takes precedence over
    /// the stored watermarks (and conflicts with `since` at the CLI).
    #[serde(default)]
    pub full: bool,

    // ── Clone URL override ────────────────────────────────────────────────
    /// Override the hostname used in git clone URLs.
    ///
    /// Useful for GitHub Enterprise Server deployments where the API host
    /// and the git clone host differ (e.g. behind separate load balancers).
    ///
    /// When set, the host of every clone URL from the API (repositories,
    /// wikis, gists, starred repositories) is rewritten to this value before
    /// it is passed to git.  A `host:port` value also sets the port; an SSH
    /// URL in the `git@host:path` form is then written as
    /// `ssh://git@host:port/path`, the only SSH form that carries a port.
    ///
    /// Example: `--api-url https://github-api.example.com/api/v3
    ///            --clone-host github-git.example.com`
    pub clone_host: Option<String>,

    // ── Permission refusals ───────────────────────────────────────────────
    /// What a category the token is not permitted to read (HTTP 403, for
    /// example webhooks without admin access) means for the run.
    ///
    /// `false` (an explicitly requested category): the refusal is a recorded
    /// failure, naming the permission the token lacks, so the run exits with
    /// status 3.  `true` (set by [`BackupOptions::all`], i.e. `--all`, which
    /// asks for everything that is available rather than for that category):
    /// the category is skipped with one warning per category per run.
    ///
    /// A category that does not exist for a repository (HTTP 404 or 410,
    /// for example Issues switched off) is never a failure.
    #[serde(default)]
    pub tolerate_forbidden: bool,

    // ── Execution options ─────────────────────────────────────────────────
    /// When `true`, log what would be done without writing any files or
    /// running any git commands.
    pub dry_run: bool,
    /// Maximum number of repositories to back up concurrently.
    /// `1` means fully sequential (safe default).
    pub concurrency: usize,
}

impl BackupOptions {
    /// Names of the enabled categories that an incremental watermark can vouch
    /// for: the per-item files fetched for each issue and pull request.
    ///
    /// The names are what [`RepoWatermark::covers`](crate::RepoWatermark::covers)
    /// stores, in a fixed order.
    #[must_use]
    pub fn incremental_categories(&self) -> Vec<&'static str> {
        [
            (self.issue_comments, "issue_comments"),
            (self.issue_events, "issue_events"),
            (self.pull_comments, "pull_comments"),
            (self.pull_commits, "pull_commits"),
            (self.pull_reviews, "pull_reviews"),
        ]
        .into_iter()
        .filter_map(|(enabled, name)| enabled.then_some(name))
        .collect()
    }

    /// Returns a configuration that enables every available backup category.
    ///
    /// Repository name filters and `since` are left at their defaults (no
    /// filtering), and [`tolerate_forbidden`](Self::tolerate_forbidden) is set:
    /// a category the token may not read is a warning, not a failure.
    ///
    /// Not enabled: `clone_starred` and `action_runs`, which can generate very
    /// large outputs and are opt-in, and `discussions`, which are not backed
    /// up (GitHub serves them through GraphQL only).
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
            prune: false,
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
            // Not backed up yet (GraphQL only); `--all` must not claim it.
            discussions: false,
            projects: true,
            commit_comments: true,
            attachments: true,
            packages: true,
            include_repos: vec![],
            exclude_repos: vec![],
            since: None,
            full: false,
            clone_host: None,
            tolerate_forbidden: true,
            dry_run: false,
            concurrency: 4,
        }
    }
}
