// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! All application state types.

use std::collections::VecDeque;
use std::fmt;
use std::path::PathBuf;

use github_backup_core::Failure;
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

/// Configuration the TUI can really apply.
///
/// Every field is read by [`ConfigState::to_backup_config`] or by the backup
/// task in `lib.rs`.  Options the TUI cannot honour (mirror push, S3 sync,
/// report, metrics, webhook, device-flow sign-in) are deliberately not
/// representable, so the form can never promise something the run will not do.
/// `Debug` is written by hand so a token never reaches a log or panic message.
#[derive(Clone)]
pub struct ConfigState {
    // ── Auth ──────────────────────────────────────────────────────────────
    pub token: String,
    pub api_url: String,

    // ── Target ────────────────────────────────────────────────────────────
    pub owner: String,
    pub output_dir: String,
    pub org_mode: bool,
    pub since: String,
    /// Ignore the saved incremental state and fetch everything again.
    pub full: bool,

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
    pub prune: bool,
    pub concurrency: String,

    // ── Filter ────────────────────────────────────────────────────────────
    pub include_repos: String, // comma-separated glob patterns
    pub exclude_repos: String,

    // ── Output ────────────────────────────────────────────────────────────
    /// Write the SHA-256 manifest after a (non-dry) run.
    pub manifest: bool,
    pub dry_run: bool,

    // ── Navigation ────────────────────────────────────────────────────────
    pub active_tab: usize,
    pub active_field: usize,
    pub editing: bool,
    pub edit_buffer: String,
}

impl fmt::Debug for ConfigState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let token = if self.token.is_empty() {
            "<empty>"
        } else {
            "<redacted>"
        };
        f.debug_struct("ConfigState")
            .field("token", &token)
            .field("owner", &self.owner)
            .field("output_dir", &self.output_dir)
            .field("org_mode", &self.org_mode)
            .field("dry_run", &self.dry_run)
            .finish_non_exhaustive()
    }
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

impl Default for ConfigState {
    fn default() -> Self {
        Self {
            token: String::new(),
            api_url: String::new(),
            owner: String::new(),
            output_dir: String::from("./github-backup"),
            org_mode: false,
            since: String::new(),
            full: false,
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
            prune: false,
            concurrency: String::from("4"),
            include_repos: String::new(),
            exclude_repos: String::new(),
            manifest: false,
            dry_run: false,
            active_tab: 0,
            active_field: 0,
            editing: false,
            edit_buffer: String::new(),
        }
    }
}

/// Highest concurrency the form accepts.
pub const MAX_CONCURRENCY: usize = 64;

/// Normalises a `since` value to `YYYY-MM-DDTHH:MM:SSZ` (UTC).
///
/// Accepts a bare date (`2024-01-01`, midnight UTC) or an RFC 3339 timestamp.
fn normalise_since(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.len() >= 20 && matches!(value.as_bytes()[10], b'T' | b't') {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
            return Ok(dt
                .with_timezone(&chrono::Utc)
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string());
        }
    }
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .ok_or_else(|| {
            "Since must be a date like 2024-01-01 or a timestamp like 2024-01-01T00:00:00Z".into()
        })
}

impl ConfigState {
    /// Returns the number of tabs in the configure screen.
    pub const TAB_COUNT: usize = 6;

    pub const TAB_NAMES: &'static [&'static str] =
        &["Auth", "Target", "Categories", "Clone", "Filter", "Output"];

    /// Index of the Categories tab (it has the select-all shortcut).
    pub const TAB_CATEGORIES: usize = 2;

    /// Count of fields per tab (for navigation wrapping).
    pub fn tab_field_count(&self) -> usize {
        match self.active_tab {
            0 => 2,  // Auth: token, api_url
            1 => 5,  // Target: owner, output_dir, org_mode, since, full
            2 => 34, // Categories: 34 bool flags
            3 => 7,  // Clone: clone_type, forks, private, lfs, prefer_ssh, prune, concurrency
            4 => 2,  // Filter: include, exclude
            5 => 2,  // Output: manifest, dry_run
            _ => 1,
        }
    }

    /// Converts form state into the types needed by the backup engine.
    /// Returns `(owner, output_path, BackupOptions, token_opt)`.
    ///
    /// The destructuring below names every field and uses no `..`: adding a
    /// field to [`ConfigState`] is a compile error until it is decided whether
    /// the backup uses it, so the form can never silently grow an inert field.
    pub fn to_backup_config(&self) -> (String, PathBuf, BackupOptions, Option<String>) {
        let ConfigState {
            token,
            api_url: _, // read by the backup task, which builds the client
            owner,
            output_dir,
            org_mode,
            since,
            full,
            repositories,
            issues,
            issue_comments,
            issue_events,
            pulls,
            pull_comments,
            pull_commits,
            pull_reviews,
            labels,
            milestones,
            releases,
            release_assets,
            hooks,
            security_advisories,
            wikis,
            starred,
            clone_starred,
            watched,
            followers,
            following,
            gists,
            starred_gists,
            topics,
            branches,
            deploy_keys,
            collaborators,
            org_members,
            org_teams,
            actions,
            action_runs,
            environments,
            discussions,
            projects,
            packages,
            clone_type,
            forks,
            private,
            lfs,
            prefer_ssh,
            prune,
            concurrency,
            include_repos,
            exclude_repos,
            manifest: _, // applied by the backup task after the run
            dry_run,
            active_tab: _,
            active_field: _,
            editing: _,
            edit_buffer: _,
        } = self;

        let token = if token.trim().is_empty() {
            None
        } else {
            Some(token.trim().to_string())
        };

        let target = if *org_mode {
            BackupTarget::Org
        } else {
            BackupTarget::User
        };

        let clone_type = match clone_type {
            CloneTypeForm::Mirror => CloneType::Mirror,
            CloneTypeForm::Bare => CloneType::Bare,
            CloneTypeForm::Full => CloneType::Full,
            CloneTypeForm::Shallow => CloneType::Shallow(10),
        };

        // `validate` rejects anything else before a run starts; the clamp keeps
        // this conversion total (a bad value can never reach the engine).
        let concurrency = concurrency
            .trim()
            .parse::<usize>()
            .unwrap_or(4)
            .clamp(1, MAX_CONCURRENCY);

        let since = if since.trim().is_empty() {
            None
        } else {
            Some(normalise_since(since).unwrap_or_else(|_| since.trim().to_string()))
        };

        let split = |s: &str| -> Vec<String> {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        };

        let opts = BackupOptions {
            target,
            full: *full,
            repositories: *repositories,
            forks: *forks,
            private: *private,
            prefer_ssh: *prefer_ssh,
            clone_type,
            lfs: *lfs,
            prune: *prune,
            issues: *issues,
            issue_comments: *issue_comments,
            issue_events: *issue_events,
            pulls: *pulls,
            pull_comments: *pull_comments,
            pull_commits: *pull_commits,
            pull_reviews: *pull_reviews,
            labels: *labels,
            milestones: *milestones,
            releases: *releases,
            release_assets: *release_assets,
            hooks: *hooks,
            security_advisories: *security_advisories,
            wikis: *wikis,
            starred: *starred,
            clone_starred: *clone_starred,
            watched: *watched,
            followers: *followers,
            following: *following,
            gists: *gists,
            starred_gists: *starred_gists,
            topics: *topics,
            branches: *branches,
            deploy_keys: *deploy_keys,
            collaborators: *collaborators,
            org_members: *org_members,
            org_teams: *org_teams,
            actions: *actions,
            action_runs: *action_runs,
            environments: *environments,
            discussions: *discussions,
            projects: *projects,
            packages: *packages,
            include_repos: split(include_repos),
            exclude_repos: split(exclude_repos),
            since,
            clone_host: None,
            dry_run: *dry_run,
            concurrency,
        };

        (
            owner.trim().to_string(),
            PathBuf::from(output_dir.trim()),
            opts,
            token,
        )
    }

    /// The reverse of [`ConfigState::to_backup_config`]: a form pre-filled from
    /// resolved command-line options (`--tui --all --private --dry-run ...`).
    ///
    /// Like `to_backup_config` this names every `BackupOptions` field and uses
    /// no `..`, so a new option is a compile error until it is decided how the
    /// form represents it.
    pub fn from_backup_options(opts: &BackupOptions) -> Self {
        let BackupOptions {
            target,
            full,
            repositories,
            issues,
            issue_comments,
            issue_events,
            pulls,
            pull_comments,
            pull_commits,
            pull_reviews,
            labels,
            milestones,
            releases,
            release_assets,
            hooks,
            security_advisories,
            wikis,
            starred,
            clone_starred,
            watched,
            followers,
            following,
            gists,
            starred_gists,
            topics,
            branches,
            deploy_keys,
            collaborators,
            org_members,
            org_teams,
            actions,
            action_runs,
            environments,
            discussions,
            projects,
            packages,
            forks,
            private,
            prefer_ssh,
            clone_type,
            lfs,
            prune,
            include_repos,
            exclude_repos,
            since,
            // The form has no field for the clone host override.
            clone_host: _,
            dry_run,
            concurrency,
        } = opts;

        let clone_type = match clone_type {
            CloneType::Mirror => CloneTypeForm::Mirror,
            CloneType::Bare => CloneTypeForm::Bare,
            CloneType::Full => CloneTypeForm::Full,
            CloneType::Shallow(_) => CloneTypeForm::Shallow,
        };

        Self {
            org_mode: *target == BackupTarget::Org,
            full: *full,
            repositories: *repositories,
            issues: *issues,
            issue_comments: *issue_comments,
            issue_events: *issue_events,
            pulls: *pulls,
            pull_comments: *pull_comments,
            pull_commits: *pull_commits,
            pull_reviews: *pull_reviews,
            labels: *labels,
            milestones: *milestones,
            releases: *releases,
            release_assets: *release_assets,
            hooks: *hooks,
            security_advisories: *security_advisories,
            wikis: *wikis,
            starred: *starred,
            clone_starred: *clone_starred,
            watched: *watched,
            followers: *followers,
            following: *following,
            gists: *gists,
            starred_gists: *starred_gists,
            topics: *topics,
            branches: *branches,
            deploy_keys: *deploy_keys,
            collaborators: *collaborators,
            org_members: *org_members,
            org_teams: *org_teams,
            actions: *actions,
            action_runs: *action_runs,
            environments: *environments,
            discussions: *discussions,
            projects: *projects,
            packages: *packages,
            forks: *forks,
            private: *private,
            prefer_ssh: *prefer_ssh,
            clone_type,
            lfs: *lfs,
            prune: *prune,
            include_repos: include_repos.join(", "),
            exclude_repos: exclude_repos.join(", "),
            since: since.clone().unwrap_or_default(),
            dry_run: *dry_run,
            concurrency: concurrency.to_string(),
            ..Self::default()
        }
    }

    /// Validates the form; returns what to fix, or `None` if a run may start.
    pub fn validate(&self) -> Option<String> {
        let owner = self.owner.trim();
        if owner.is_empty() {
            return Some("Owner is required (Configure > Target tab)".into());
        }
        if owner.contains(['/', '\\'])
            || owner.contains("..")
            || owner.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Some(
                "Owner must be a plain user or organisation name (Configure > Target tab)".into(),
            );
        }
        if self.output_dir.trim().is_empty() {
            return Some("Output directory is required (Configure > Target tab)".into());
        }
        if self.token.trim().is_empty() {
            return Some("A GitHub token is required (Configure > Auth tab)".into());
        }
        let api_url = self.api_url.trim();
        if !api_url.is_empty() && !api_url.starts_with("https://") {
            return Some("API URL must start with https:// (Configure > Auth tab)".into());
        }
        if !self.since.trim().is_empty() {
            if let Err(e) = normalise_since(&self.since) {
                return Some(format!("{e} (Configure > Target tab)"));
            }
        }
        if self.full && !self.since.trim().is_empty() {
            return Some(
                "Full backup and Since cannot be combined (as on the command line); clear one \
                 (Configure > Target tab)"
                    .into(),
            );
        }
        match self.concurrency.trim().parse::<usize>() {
            Ok(n) if (1..=MAX_CONCURRENCY).contains(&n) => {}
            _ => {
                return Some(format!(
                    "Concurrency must be a number from 1 to {MAX_CONCURRENCY} (Configure > Clone tab)"
                ));
            }
        }
        None
    }

    /// Returns `true` if every category flag (the ones `A` toggles) is on.
    pub fn all_categories_on(&self) -> bool {
        self.repositories
            && self.issues
            && self.issue_comments
            && self.issue_events
            && self.pulls
            && self.pull_comments
            && self.pull_commits
            && self.pull_reviews
            && self.labels
            && self.milestones
            && self.releases
            && self.release_assets
            && self.hooks
            && self.security_advisories
            && self.wikis
            && self.starred
            && self.watched
            && self.followers
            && self.following
            && self.gists
            && self.starred_gists
            && self.topics
            && self.branches
            && self.deploy_keys
            && self.collaborators
            && self.org_members
            && self.org_teams
            && self.actions
            && self.environments
            && self.discussions
            && self.projects
            && self.packages
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
pub enum RepoStatus {
    Running,
    Done,
    /// The repository finished with at least one failed step.
    Error,
    /// Nothing was attempted (filtered out or dry run).
    Skipped,
}

/// A row in the repo list during a running backup.
#[derive(Debug, Clone)]
pub struct RepoEntry {
    pub name: String,
    pub status: RepoStatus,
    /// Error description, populated when `status == RepoStatus::Error`.
    pub error: Option<String>,
}

/// Where the backup task is in its life cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunStatus {
    /// No backup is requested or running.
    #[default]
    Idle,
    /// A backup was requested or is running.
    Active,
    /// Cancel was requested; the task is stopping git and releasing its lock.
    Cancelling,
}

/// Maximum number of log lines kept in memory.
pub const LOG_CAPACITY: usize = 2000;

/// All state for the Running screen.
#[derive(Debug, Default)]
pub struct RunState {
    pub status: RunStatus,
    pub repos: Vec<RepoEntry>,
    pub log_lines: VecDeque<LogLine>,
    pub total_repos: u64,
    pub repos_done: u64,
    /// Repositories that finished with a failure so far.
    pub repos_errored: u64,
    pub repos_skipped: u64,
    /// Repositories the engine has finished with (any outcome).
    pub processed: u64,
    pub started_at: Option<std::time::Instant>,
    /// Frozen elapsed time once the run has ended (the clock stops).
    pub elapsed_final: Option<std::time::Duration>,
    pub repo_list_offset: usize,
    /// How many entries above the newest the log view is scrolled (0 = follow).
    pub log_back: usize,
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

    /// `true` from the moment a backup is requested until it has finished.
    pub fn is_active(&self) -> bool {
        self.status != RunStatus::Idle
    }

    /// Number of repositories known to have failed so far.
    pub fn incomplete_repos(&self) -> u64 {
        self.repos_errored
    }

    pub fn elapsed_str(&self) -> String {
        let elapsed = self
            .elapsed_final
            .or_else(|| self.started_at.map(|s| s.elapsed()));
        if let Some(elapsed) = elapsed {
            let secs = elapsed.as_secs();
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
        let finished = self
            .processed
            .max(self.repos_done + self.repos_errored + self.repos_skipped);
        ((finished * 100) / self.total_repos).min(100) as u16
    }

    /// Append a log line, capping the buffer to avoid memory growth.
    ///
    /// While the view is following the newest line it stays there; if the
    /// user scrolled back, the view keeps showing the same lines.
    pub fn push_log(&mut self, line: LogLine) {
        if self.log_lines.len() >= LOG_CAPACITY {
            self.log_lines.pop_front();
        }
        self.log_lines.push_back(line);
        if self.log_back > 0 {
            self.log_back = (self.log_back + 1).min(self.log_lines.len().saturating_sub(1));
        }
    }
}

// ── Results state ─────────────────────────────────────────────────────────────

/// How the last run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Outcome {
    /// No backup has finished in this session.
    #[default]
    NotRun,
    /// Ran to the end with no failures.
    Complete,
    /// Ran to the end but some items failed: the backup is not complete.
    Incomplete,
    /// Stopped by a fatal error before it could finish.
    Failed,
    /// Stopped by the user.
    Cancelled,
}

#[derive(Debug, Default, Clone)]
pub struct ResultsState {
    pub outcome: Outcome,
    pub dry_run: bool,
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
    /// Every failure the engine recorded, in the order recorded.
    pub failures: Vec<Failure>,
    /// Selected row of the failure list.
    pub failure_selected: usize,
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

    /// Moves the failure selection by `delta`, clamped to the list.
    pub fn move_failure_selection(&mut self, delta: isize) {
        let last = self.failures.len().saturating_sub(1);
        let next = self.failure_selected as isize + delta;
        self.failure_selected = next.clamp(0, last as isize) as usize;
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

/// Longest list the verify screen prints per category before "+N more".
pub const VERIFY_LIST_CAP: usize = 50;

impl VerifyState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn is_clean(&self) -> bool {
        self.tampered.is_empty() && self.missing.is_empty()
    }

    /// Number of result rows the screen can scroll through.
    pub fn row_count(&self) -> usize {
        if !self.done {
            return 0;
        }
        let section = |n: usize| {
            if n == 0 {
                0
            } else {
                1 + n.min(VERIFY_LIST_CAP) + usize::from(n > VERIFY_LIST_CAP)
            }
        };
        1 + section(self.tampered.len())
            + section(self.missing.len())
            + section(self.unexpected.len())
    }

    /// Scrolls by `delta`, never past the last row.
    pub fn scroll_by(&mut self, delta: isize) {
        let last = self.row_count().saturating_sub(1);
        let next = self.scroll as isize + delta;
        self.scroll = next.clamp(0, last as isize) as usize;
    }
}

// ── Dashboard state ───────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct DashboardState {
    pub last_backup_time: Option<String>,
    pub last_backup_repos: Option<u64>,
    pub last_tool_version: Option<String>,
    /// `Some(false)` if the last recorded run had failures.
    pub last_run_ok: Option<bool>,
    pub last_run_failures: u64,
    pub selected_action: usize,
}

impl DashboardState {
    pub const ACTIONS: &'static [&'static str] =
        &["Run Backup", "Configure", "Verify Integrity", "Quit"];
}
