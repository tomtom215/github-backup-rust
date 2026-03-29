// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! All application state types.

use std::path::PathBuf;

use github_backup_types::config::{BackupOptions, BackupTarget, CloneType};

// ── Screen ────────────────────────────────────────────────────────────────────

/// Which top-level screen is currently active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    Configure,
    Running,
    Results,
    Verify,
}

// ── Configure form ────────────────────────────────────────────────────────────

/// All configuration state, mirroring every CLI flag.
#[derive(Debug, Clone)]
pub struct ConfigState {
    // ── Auth ──────────────────────────────────────────────────────────────
    pub token: String,
    pub api_url: String,
    pub device_auth: bool,
    pub oauth_client_id: String,

    // ── Target ────────────────────────────────────────────────────────────
    pub owner: String,
    pub output_dir: String,
    pub org_mode: bool,
    pub since: String,

    // ── Categories ────────────────────────────────────────────────────────
    pub repositories: bool,
    pub issues: bool,
    pub issue_comments: bool,
    pub issue_events: bool,
    pub pulls: bool,
    pub pull_comments: bool,
    pub pull_commits: bool,
    pub pull_reviews: bool,
    pub labels: bool,
    pub milestones: bool,
    pub releases: bool,
    pub release_assets: bool,
    pub hooks: bool,
    pub security_advisories: bool,
    pub wikis: bool,
    pub starred: bool,
    pub clone_starred: bool,
    pub watched: bool,
    pub followers: bool,
    pub following: bool,
    pub gists: bool,
    pub starred_gists: bool,
    pub topics: bool,
    pub branches: bool,
    pub deploy_keys: bool,
    pub collaborators: bool,
    pub org_members: bool,
    pub org_teams: bool,
    pub actions: bool,
    pub action_runs: bool,
    pub environments: bool,
    pub discussions: bool,
    pub projects: bool,
    pub packages: bool,

    // ── Clone ─────────────────────────────────────────────────────────────
    pub clone_type: CloneTypeForm,
    pub forks: bool,
    pub private: bool,
    pub lfs: bool,
    pub prefer_ssh: bool,
    pub no_prune: bool,
    pub concurrency: String,

    // ── Filter ────────────────────────────────────────────────────────────
    pub include_repos: String, // comma-separated glob patterns
    pub exclude_repos: String,

    // ── Mirror ────────────────────────────────────────────────────────────
    pub mirror_to: String,
    pub mirror_type: MirrorTypeForm,
    pub mirror_token: String,
    pub mirror_owner: String,
    pub mirror_private: bool,

    // ── S3 ────────────────────────────────────────────────────────────────
    pub s3_bucket: String,
    pub s3_region: String,
    pub s3_prefix: String,
    pub s3_endpoint: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub s3_include_assets: bool,

    // ── Output / extras ───────────────────────────────────────────────────
    pub manifest: bool,
    pub dry_run: bool,
    pub report: String,
    pub prometheus_metrics: String,
    pub keep_last: String,
    pub max_age_days: String,

    // ── Navigation ────────────────────────────────────────────────────────
    pub active_tab: usize,
    pub active_field: usize,
    pub editing: bool,
    pub edit_buffer: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloneTypeForm {
    Mirror,
    Bare,
    Full,
    Shallow,
}

impl CloneTypeForm {
    pub const OPTIONS: &'static [&'static str] = &["mirror", "bare", "full", "shallow"];

    pub fn idx(&self) -> usize {
        match self {
            Self::Mirror => 0,
            Self::Bare => 1,
            Self::Full => 2,
            Self::Shallow => 3,
        }
    }

    pub fn from_idx(i: usize) -> Self {
        match i {
            1 => Self::Bare,
            2 => Self::Full,
            3 => Self::Shallow,
            _ => Self::Mirror,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirrorTypeForm {
    Gitea,
    Gitlab,
}

impl MirrorTypeForm {
    pub const OPTIONS: &'static [&'static str] = &["gitea", "gitlab"];

    pub fn idx(&self) -> usize {
        match self {
            Self::Gitea => 0,
            Self::Gitlab => 1,
        }
    }

    pub fn from_idx(i: usize) -> Self {
        match i {
            1 => Self::Gitlab,
            _ => Self::Gitea,
        }
    }
}

impl Default for ConfigState {
    fn default() -> Self {
        Self {
            token: String::new(),
            api_url: String::new(),
            device_auth: false,
            oauth_client_id: String::new(),
            owner: String::new(),
            output_dir: String::from("./github-backup"),
            org_mode: false,
            since: String::new(),
            repositories: true,
            issues: false,
            issue_comments: false,
            issue_events: false,
            pulls: false,
            pull_comments: false,
            pull_commits: false,
            pull_reviews: false,
            labels: false,
            milestones: false,
            releases: false,
            release_assets: false,
            hooks: false,
            security_advisories: false,
            wikis: false,
            starred: false,
            clone_starred: false,
            watched: false,
            followers: false,
            following: false,
            gists: false,
            starred_gists: false,
            topics: false,
            branches: false,
            deploy_keys: false,
            collaborators: false,
            org_members: false,
            org_teams: false,
            actions: false,
            action_runs: false,
            environments: false,
            discussions: false,
            projects: false,
            packages: false,
            clone_type: CloneTypeForm::Mirror,
            forks: false,
            private: false,
            lfs: false,
            prefer_ssh: false,
            no_prune: false,
            concurrency: String::from("4"),
            include_repos: String::new(),
            exclude_repos: String::new(),
            mirror_to: String::new(),
            mirror_type: MirrorTypeForm::Gitea,
            mirror_token: String::new(),
            mirror_owner: String::new(),
            mirror_private: false,
            s3_bucket: String::new(),
            s3_region: String::from("us-east-1"),
            s3_prefix: String::new(),
            s3_endpoint: String::new(),
            s3_access_key: String::new(),
            s3_secret_key: String::new(),
            s3_include_assets: false,
            manifest: false,
            dry_run: false,
            report: String::new(),
            prometheus_metrics: String::new(),
            keep_last: String::new(),
            max_age_days: String::new(),
            active_tab: 0,
            active_field: 0,
            editing: false,
            edit_buffer: String::new(),
        }
    }
}

impl ConfigState {
    /// Returns the number of tabs in the configure screen.
    pub const TAB_COUNT: usize = 8;

    pub const TAB_NAMES: &'static [&'static str] = &[
        "Auth",
        "Target",
        "Categories",
        "Clone",
        "Filter",
        "Mirror",
        "S3",
        "Output",
    ];

    /// Count of fields per tab (for navigation wrapping).
    pub fn tab_field_count(&self) -> usize {
        match self.active_tab {
            0 => 4,  // Auth: token, api_url, device_auth, oauth_client_id
            1 => 4,  // Target: owner, output_dir, org_mode, since
            2 => 34, // Categories: 34 bool flags
            3 => 7,  // Clone: clone_type, forks, private, lfs, prefer_ssh, no_prune, concurrency
            4 => 2,  // Filter: include, exclude
            5 => 5,  // Mirror: mirror_to, mirror_type, mirror_token, mirror_owner, mirror_private
            6 => 7,  // S3: bucket, region, prefix, endpoint, access_key, secret_key, include_assets
            7 => 6, // Output: manifest, dry_run, report, prometheus_metrics, keep_last, max_age_days
            _ => 1,
        }
    }

    /// Converts form state into the types needed by the backup engine.
    /// Returns `(owner, output_path, BackupOptions, token_opt)`.
    pub fn to_backup_config(&self) -> (String, PathBuf, BackupOptions, Option<String>) {
        let owner = self.owner.trim().to_string();
        let output = PathBuf::from(self.output_dir.trim());
        let token = if self.token.trim().is_empty() {
            None
        } else {
            Some(self.token.trim().to_string())
        };

        let target = if self.org_mode {
            BackupTarget::Org
        } else {
            BackupTarget::User
        };

        let clone_type = match self.clone_type {
            CloneTypeForm::Mirror => CloneType::Mirror,
            CloneTypeForm::Bare => CloneType::Bare,
            CloneTypeForm::Full => CloneType::Full,
            CloneTypeForm::Shallow => CloneType::Shallow(10),
        };

        let concurrency = self.concurrency.trim().parse::<usize>().unwrap_or(4);

        let since = if self.since.trim().is_empty() {
            None
        } else {
            Some(self.since.trim().to_string())
        };

        let include_repos: Vec<String> = self
            .include_repos
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let exclude_repos: Vec<String> = self
            .exclude_repos
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let api_url = if self.api_url.trim().is_empty() {
            None
        } else {
            Some(self.api_url.trim().to_string())
        };
        let _ = api_url; // used by caller

        let opts = BackupOptions {
            target,
            repositories: self.repositories,
            forks: self.forks,
            private: self.private,
            prefer_ssh: self.prefer_ssh,
            clone_type,
            lfs: self.lfs,
            no_prune: self.no_prune,
            issues: self.issues,
            issue_comments: self.issue_comments,
            issue_events: self.issue_events,
            pulls: self.pulls,
            pull_comments: self.pull_comments,
            pull_commits: self.pull_commits,
            pull_reviews: self.pull_reviews,
            labels: self.labels,
            milestones: self.milestones,
            releases: self.releases,
            release_assets: self.release_assets,
            hooks: self.hooks,
            security_advisories: self.security_advisories,
            wikis: self.wikis,
            starred: self.starred,
            clone_starred: self.clone_starred,
            watched: self.watched,
            followers: self.followers,
            following: self.following,
            gists: self.gists,
            starred_gists: self.starred_gists,
            topics: self.topics,
            branches: self.branches,
            deploy_keys: self.deploy_keys,
            collaborators: self.collaborators,
            org_members: self.org_members,
            org_teams: self.org_teams,
            actions: self.actions,
            action_runs: self.action_runs,
            environments: self.environments,
            discussions: self.discussions,
            projects: self.projects,
            packages: self.packages,
            include_repos,
            exclude_repos,
            since,
            clone_host: None,
            dry_run: self.dry_run,
            concurrency,
        };

        (owner, output, opts, token)
    }

    /// Validates required fields; returns an error string if invalid.
    pub fn validate(&self) -> Option<String> {
        if self.owner.trim().is_empty() {
            return Some("Owner is required (Configure > Target tab)".into());
        }
        if self.token.trim().is_empty() && !self.device_auth {
            return Some(
                "A token is required unless using device auth (Configure > Auth tab)".into(),
            );
        }
        None
    }

    /// Sets all category flags to `val`.
    pub fn set_all_categories(&mut self, val: bool) {
        self.repositories = val;
        self.issues = val;
        self.issue_comments = val;
        self.issue_events = val;
        self.pulls = val;
        self.pull_comments = val;
        self.pull_commits = val;
        self.pull_reviews = val;
        self.labels = val;
        self.milestones = val;
        self.releases = val;
        self.release_assets = val;
        self.hooks = val;
        self.security_advisories = val;
        self.wikis = val;
        self.starred = val;
        self.watched = val;
        self.followers = val;
        self.following = val;
        self.gists = val;
        self.starred_gists = val;
        self.topics = val;
        self.branches = val;
        self.deploy_keys = val;
        self.collaborators = val;
        self.org_members = val;
        self.org_teams = val;
        self.actions = val;
        self.environments = val;
        self.discussions = val;
        self.projects = val;
        self.packages = val;
    }
}

// ── Run state ─────────────────────────────────────────────────────────────────

/// Status of a single repository in the Running screen.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum RepoStatus {
    Pending,
    Running,
    Done,
    Error,
    Skipped,
}

/// A row in the repo list during a running backup.
#[derive(Debug, Clone)]
pub struct RepoEntry {
    pub name: String,
    pub status: RepoStatus,
}

/// All state for the Running screen.
#[derive(Debug, Default)]
pub struct RunState {
    pub repos: Vec<RepoEntry>,
    pub log_lines: Vec<LogLine>,
    pub total_repos: u64,
    pub repos_done: u64,
    pub repos_errored: u64,
    pub repos_skipped: u64,
    pub started_at: Option<std::time::Instant>,
    pub repo_list_offset: usize,
    pub log_offset: usize,
    pub phase: String,
}

/// A captured log line from the tracing subscriber.
#[derive(Debug, Clone)]
pub struct LogLine {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

impl RunState {
    pub fn reset(&mut self) {
        *self = Self {
            phase: "Initialising".into(),
            ..Default::default()
        };
    }

    pub fn elapsed_str(&self) -> String {
        if let Some(start) = self.started_at {
            let secs = start.elapsed().as_secs();
            format!(
                "{:02}:{:02}:{:02}",
                secs / 3600,
                (secs % 3600) / 60,
                secs % 60
            )
        } else {
            "00:00:00".into()
        }
    }

    pub fn progress_pct(&self) -> u16 {
        if self.total_repos == 0 {
            return 0;
        }
        let done = self.repos_done + self.repos_errored + self.repos_skipped;
        ((done * 100) / self.total_repos).min(100) as u16
    }

    /// Append a log line, capping at 2000 to avoid memory growth.
    pub fn push_log(&mut self, line: LogLine) {
        if self.log_lines.len() >= 2000 {
            self.log_lines.remove(0);
        }
        self.log_lines.push(line);
        // Auto-scroll: always show newest if user hasn't scrolled up.
        if !self.log_lines.is_empty() {
            self.log_offset = self.log_lines.len().saturating_sub(1);
        }
    }
}

// ── Results state ─────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
pub struct ResultsState {
    pub success: bool,
    pub repos_backed_up: u64,
    pub repos_discovered: u64,
    pub repos_skipped: u64,
    pub repos_errored: u64,
    pub gists_backed_up: u64,
    pub issues_fetched: u64,
    pub prs_fetched: u64,
    pub workflows_fetched: u64,
    pub discussions_fetched: u64,
    pub elapsed_secs: f64,
    pub error_message: Option<String>,
    pub owner: String,
    pub output_dir: String,
}

impl ResultsState {
    pub fn elapsed_str(&self) -> String {
        let secs = self.elapsed_secs as u64;
        format!(
            "{:02}:{:02}:{:02}",
            secs / 3600,
            (secs % 3600) / 60,
            secs % 60
        )
    }
}

// ── Verify state ─────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct VerifyState {
    pub running: bool,
    pub done: bool,
    pub ok: u64,
    pub tampered: Vec<String>,
    pub missing: Vec<String>,
    pub unexpected: Vec<String>,
    pub error: Option<String>,
    pub scroll: usize,
}

impl VerifyState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn is_clean(&self) -> bool {
        self.tampered.is_empty() && self.missing.is_empty()
    }
}

// ── Dashboard state ───────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct DashboardState {
    pub last_backup_time: Option<String>,
    pub last_backup_repos: Option<u64>,
    pub last_tool_version: Option<String>,
    pub selected_action: usize,
    pub status_message: Option<String>,
    pub error_message: Option<String>,
}

impl DashboardState {
    pub const ACTIONS: &'static [&'static str] =
        &["Run Backup", "Configure", "Verify Integrity", "Quit"];
}
