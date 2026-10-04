// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! [`App`] struct plus keyboard / progress event handling.
//!
//! Rendering lives in `lib.rs` and the `screens/` modules; this module is
//! purely state + event dispatch.

use std::path::Path;
use std::time::Instant;

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    event::BackupEvent,
    state::{
        CloneTypeForm, ConfigState, DashboardState, LogLine, Outcome, RepoEntry, RepoStatus,
        ResultsState, RunState, RunStatus, Screen, VerifyState,
    },
};

/// Longest value the edit buffer accepts (a fine-grained token is ~100).
const EDIT_MAX_CHARS: usize = 1024;

// ── App ───────────────────────────────────────────────────────────────────────

pub struct App {
    pub screen: Screen,
    pub config: ConfigState,
    pub dashboard: DashboardState,
    pub run: RunState,
    pub results: ResultsState,
    pub verify: VerifyState,
    pub should_quit: bool,
    /// Set when the user requests a backup start; cleared by lib.rs after spawn.
    pub start_backup_requested: bool,
    /// Set when the user requests a verify; cleared by lib.rs after spawn.
    pub start_verify_requested: bool,
    /// Sent `()` to cancel the running backup task.
    pub cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
    /// Error message shown in a modal overlay (cleared on any keypress).
    pub modal_error: Option<String>,
    /// A process signal asked us to exit; the loop leaves once the backup
    /// task has stopped (or a grace period has passed).
    pub shutdown_requested: bool,
    /// Exit status to return after a signal-initiated shutdown.
    pub shutdown_code: u8,
}

/// Initial configuration pre-populated from CLI arguments.
#[derive(Default)]
pub struct InitialConfig {
    pub token: Option<String>,
    pub owner: Option<String>,
    pub output: Option<String>,
    pub api_url: Option<String>,
    /// Backup options resolved from the command line (`--all`, `--private`,
    /// `--dry-run`, `--org`, ...); pre-fills the Configure screen.
    pub options: Option<github_backup_types::config::BackupOptions>,
    /// `--manifest` was given.
    pub manifest: bool,
}

impl App {
    pub fn new(initial: InitialConfig) -> Self {
        let mut config = match &initial.options {
            Some(opts) => ConfigState::from_backup_options(opts),
            None => ConfigState::default(),
        };
        config.manifest = initial.manifest;
        if let Some(t) = initial.token {
            config.token = t;
        }
        if let Some(o) = initial.owner {
            config.owner = o;
        }
        if let Some(p) = initial.output {
            config.output_dir = p;
        }
        if let Some(u) = initial.api_url {
            config.api_url = u;
        }

        let dashboard = load_dashboard_state(&config);

        Self {
            screen: Screen::Dashboard,
            config,
            dashboard,
            run: RunState::default(),
            results: ResultsState::default(),
            verify: VerifyState::default(),
            should_quit: false,
            start_backup_requested: false,
            start_verify_requested: false,
            cancel_tx: None,
            modal_error: None,
            shutdown_requested: false,
            shutdown_code: 0,
        }
    }

    /// Re-reads the last-run information for the configured owner/output.
    pub fn reload_dashboard(&mut self) {
        let selected = self.dashboard.selected_action;
        self.dashboard = load_dashboard_state(&self.config);
        self.dashboard.selected_action = selected;
    }

    /// Switches to the Dashboard, refreshing what it shows: the owner and
    /// output directory may have been edited since it was last drawn.
    pub fn go_dashboard(&mut self) {
        self.reload_dashboard();
        self.screen = Screen::Dashboard;
    }
}

fn load_dashboard_state(config: &ConfigState) -> DashboardState {
    use github_backup_types::backup_state::{BackupRunHistory, BackupState};
    use github_backup_types::config::OutputConfig;

    let mut dash = DashboardState::default();
    let owner = config.owner.trim();
    let output = config.output_dir.trim();
    if owner.is_empty() || output.is_empty() {
        return dash;
    }
    let out = OutputConfig::new(Path::new(output));

    // The newest history entry says how the last run ended (including how many
    // things failed); the state file only holds the incremental watermark.
    let history = BackupRunHistory::load(&out.backup_history_path(owner)).unwrap_or_default();
    if let Some(last) = history.entries.first() {
        dash.last_backup_time = Some(last.timestamp.clone());
        dash.last_backup_repos = Some(last.repos_backed_up);
        dash.last_tool_version = Some(last.tool_version.clone());
        dash.last_run_ok = Some(last.success);
        dash.last_run_failures = last.failures;
        return dash;
    }

    if let Ok(Some(s)) = BackupState::load(&out.backup_state_path(owner)) {
        dash.last_backup_time = s.last_successful_run.clone();
        dash.last_backup_repos = Some(s.repos_backed_up);
        dash.last_tool_version = Some(s.tool_version.clone());
    }
    dash
}

// ── Public event dispatch ─────────────────────────────────────────────────────

/// Called by `lib.rs` on every key press.
pub fn handle_key_dispatch(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    // Dismiss any modal error first.
    if app.modal_error.is_some() {
        app.modal_error = None;
        return;
    }

    // Ctrl+C.
    if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        handle_ctrl_c(app);
        return;
    }

    // Other Ctrl/Alt chords never insert text and never trigger shortcuts.
    if modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
        return;
    }

    // Global number shortcuts when not editing text and no backup is active.
    if !app.config.editing && !app.run.is_active() {
        let target = match code {
            KeyCode::Char('1') => Some(Screen::Dashboard),
            KeyCode::Char('2') => Some(Screen::Configure),
            KeyCode::Char('3') => Some(Screen::Running),
            KeyCode::Char('4') => Some(Screen::Verify),
            KeyCode::Char('5') => Some(Screen::Results),
            _ => None,
        };
        if let Some(screen) = target {
            if screen == Screen::Dashboard {
                app.go_dashboard();
            } else {
                app.screen = screen;
            }
            return;
        }
    }

    match app.screen {
        Screen::Dashboard => handle_dashboard(app, code),
        Screen::Configure => handle_configure(app, code),
        Screen::Running => handle_running(app, code),
        Screen::Results => handle_results(app, code),
        Screen::Verify => handle_verify(app, code),
    }
}

/// Called by `lib.rs` for a bracketed paste.
///
/// Pasted text is only ever inserted into a field that is being edited; it is
/// never interpreted as key presses (a pasted `s` must not start a backup).
pub fn handle_paste(app: &mut App, text: &str) {
    if app.modal_error.is_some() || !app.config.editing {
        return;
    }
    for c in text.chars().filter(|c| !c.is_control()) {
        push_edit_char(app, c);
    }
}

/// Starts cancelling the active backup, if any.  Returns `true` if a cancel
/// was sent or is already under way.
fn begin_cancel(app: &mut App) -> bool {
    if app.run.status == RunStatus::Cancelling {
        return true;
    }
    if let Some(tx) = app.cancel_tx.take() {
        let _ = tx.send(());
        app.run.status = RunStatus::Cancelling;
        app.run.phase = "Cancelling: stopping git and releasing the lock".into();
        return true;
    }
    false
}

fn handle_ctrl_c(app: &mut App) {
    if app.screen == Screen::Running {
        if begin_cancel(app) {
            return;
        }
        if app.start_backup_requested {
            // Requested but not spawned yet: just withdraw the request.
            app.start_backup_requested = false;
            app.run.status = RunStatus::Idle;
            app.go_dashboard();
            return;
        }
    }
    app.should_quit = true;
}

/// Asks the TUI to exit: cancels a running backup first so git is killed and
/// the lock released before the process leaves.
fn request_shutdown(app: &mut App, code: u8) {
    if app.shutdown_requested {
        // A second signal: stop waiting for the backup to wind down.
        app.should_quit = true;
        return;
    }
    app.shutdown_requested = true;
    app.shutdown_code = code;
    if !begin_cancel(app) && !app.run.is_active() {
        app.should_quit = true;
    }
}

/// Called by `lib.rs` whenever a [`BackupEvent`] arrives on the channel.
pub fn handle_backup_event(app: &mut App, ev: BackupEvent) {
    match ev {
        BackupEvent::LogLine {
            timestamp,
            level,
            message,
        } => {
            app.run.push_log(LogLine {
                timestamp,
                level,
                message,
            });
        }
        BackupEvent::RepoStarted { name } => {
            if let Some(e) = app.run.repos.iter_mut().find(|r| r.name == name) {
                e.status = RepoStatus::Running;
            } else {
                app.run.repos.push(RepoEntry {
                    name,
                    status: RepoStatus::Running,
                    error: None,
                });
            }
        }
        BackupEvent::RepoCompleted {
            name,
            success,
            error,
        } => {
            let status = if success {
                RepoStatus::Done
            } else {
                RepoStatus::Error
            };
            if let Some(e) = app.run.repos.iter_mut().find(|r| r.name == name) {
                e.status = status;
                e.error = error;
            } else {
                app.run.repos.push(RepoEntry {
                    name,
                    status,
                    error,
                });
            }
            if success {
                app.run.repos_done += 1;
            } else {
                app.run.repos_errored += 1;
            }
        }
        BackupEvent::ReposDiscovered { total } => {
            app.run.total_repos = total;
            if app.run.status != RunStatus::Cancelling {
                app.run.phase = format!("Backing up {total} repos");
            }
        }
        BackupEvent::Progress { current, total } => {
            app.run.processed = app.run.processed.max(current);
            if app.run.total_repos == 0 {
                app.run.total_repos = total;
            }
        }
        BackupEvent::BackupDone {
            repos_backed_up,
            repos_discovered,
            repos_skipped,
            repos_errored,
            gists_backed_up,
            issues_fetched,
            prs_fetched,
            workflows_fetched,
            discussions_fetched,
            elapsed_secs,
            failures,
            dry_run,
        } => {
            // A run that recorded any failure is incomplete, however many
            // repositories made it.
            let outcome = if failures.is_empty() && repos_errored == 0 {
                Outcome::Complete
            } else {
                Outcome::Incomplete
            };
            app.results = ResultsState {
                outcome,
                dry_run,
                repos_backed_up,
                repos_discovered,
                repos_skipped,
                repos_errored,
                gists_backed_up,
                issues_fetched,
                prs_fetched,
                workflows_fetched,
                discussions_fetched,
                elapsed_secs,
                error_message: None,
                failures,
                failure_selected: 0,
                owner: app.config.owner.trim().to_string(),
                output_dir: app.config.output_dir.trim().to_string(),
            };
            // Repositories that never reported back were skipped (filtered
            // out, or a dry run); do not leave them looking "in progress".
            for r in &mut app.run.repos {
                if r.status == RepoStatus::Running {
                    r.status = RepoStatus::Skipped;
                }
            }
            app.run.phase = if outcome == Outcome::Complete {
                "Complete".into()
            } else {
                "Incomplete".into()
            };
            finish_backup(app, Screen::Results);
        }
        BackupEvent::BackupFailed { error } => {
            app.results = finished_without_stats(app, Outcome::Failed, Some(error));
            app.run.phase = "Failed".into();
            finish_backup(app, Screen::Results);
        }
        BackupEvent::BackupCancelled => {
            app.results = finished_without_stats(app, Outcome::Cancelled, None);
            app.run.phase = "Cancelled".into();
            finish_backup(app, Screen::Results);
        }
        BackupEvent::Shutdown { code } => request_shutdown(app, code),
        BackupEvent::VerifyDone {
            ok,
            tampered,
            missing,
            unexpected,
        } => {
            app.verify.running = false;
            app.verify.done = true;
            app.verify.ok = ok;
            app.verify.tampered = tampered;
            app.verify.missing = missing;
            app.verify.unexpected = unexpected;
            app.start_verify_requested = false;
        }
        BackupEvent::VerifyFailed { error } => {
            app.verify.running = false;
            app.verify.error = Some(error);
            app.start_verify_requested = false;
        }
    }
}

fn finished_without_stats(app: &App, outcome: Outcome, error: Option<String>) -> ResultsState {
    ResultsState {
        outcome,
        dry_run: app.config.dry_run,
        error_message: error,
        owner: app.config.owner.trim().to_string(),
        output_dir: app.config.output_dir.trim().to_string(),
        elapsed_secs: app
            .run
            .started_at
            .map(|s| s.elapsed().as_secs_f64())
            .unwrap_or(0.0),
        repos_errored: app.run.repos_errored,
        ..Default::default()
    }
}

/// Common tail of every way a backup can end.
fn finish_backup(app: &mut App, screen: Screen) {
    app.run.status = RunStatus::Idle;
    app.run.elapsed_final = app.run.started_at.map(|s| s.elapsed());
    app.cancel_tx = None;
    app.reload_dashboard();
    app.screen = screen;
}

// ── Per-screen key handlers ───────────────────────────────────────────────────

fn handle_dashboard(app: &mut App, code: KeyCode) {
    use crate::state::DashboardState;
    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') => app.should_quit = true,
        KeyCode::Char('r') | KeyCode::Char('R') => request_backup(app),
        KeyCode::Char('c') | KeyCode::Char('C') => app.screen = Screen::Configure,
        KeyCode::Char('v') | KeyCode::Char('V') => {
            app.screen = Screen::Verify;
            app.verify.reset();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.dashboard.selected_action =
                (app.dashboard.selected_action + 1) % DashboardState::ACTIONS.len();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let n = DashboardState::ACTIONS.len();
            app.dashboard.selected_action = (app.dashboard.selected_action + n - 1) % n;
        }
        KeyCode::Enter => match app.dashboard.selected_action {
            0 => request_backup(app),
            1 => app.screen = Screen::Configure,
            2 => {
                app.screen = Screen::Verify;
                app.verify.reset();
            }
            3 => app.should_quit = true,
            _ => {}
        },
        _ => {}
    }
}

fn handle_configure(app: &mut App, code: KeyCode) {
    if app.config.editing {
        handle_configure_editing(app, code);
        return;
    }

    match code {
        KeyCode::Esc => app.go_dashboard(),
        KeyCode::Tab => {
            app.config.active_tab = (app.config.active_tab + 1) % ConfigState::TAB_COUNT;
            app.config.active_field = 0;
        }
        KeyCode::BackTab => {
            app.config.active_tab =
                (app.config.active_tab + ConfigState::TAB_COUNT - 1) % ConfigState::TAB_COUNT;
            app.config.active_field = 0;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let n = app.config.tab_field_count();
            app.config.active_field = (app.config.active_field + 1) % n;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let n = app.config.tab_field_count();
            app.config.active_field = (app.config.active_field + n - 1) % n;
        }
        KeyCode::Char(' ') => toggle_field(app),
        KeyCode::Enter => enter_field(app),
        KeyCode::Left => cycle_select(app, -1),
        KeyCode::Right => cycle_select(app, 1),
        KeyCode::Char('A') if app.config.active_tab == ConfigState::TAB_CATEGORIES => {
            // Any category off: turn them all on; all on: turn them all off.
            let all_on = app.config.all_categories_on();
            app.config.set_all_categories(!all_on);
        }
        KeyCode::F(5) | KeyCode::Char('s') => request_backup(app),
        _ => {}
    }
}

fn handle_configure_editing(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Enter => commit_edit(app),
        KeyCode::Esc => cancel_edit(app),
        KeyCode::Backspace => {
            app.config.edit_buffer.pop();
        }
        KeyCode::Char(c) => push_edit_char(app, c),
        _ => {}
    }
}

fn is_numeric_field(app: &App) -> bool {
    // Clone > Concurrency
    (app.config.active_tab, app.config.active_field) == (3, 6)
}

fn push_edit_char(app: &mut App, c: char) {
    if c.is_control() {
        return;
    }
    if is_numeric_field(app) && !c.is_ascii_digit() {
        return;
    }
    if app.config.edit_buffer.chars().count() < EDIT_MAX_CHARS {
        app.config.edit_buffer.push(c);
    }
}

fn handle_running(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('j') | KeyCode::Down => {
            let max = app.run.repos.len().saturating_sub(1);
            app.run.repo_list_offset = (app.run.repo_list_offset + 1).min(max);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.run.repo_list_offset = app.run.repo_list_offset.saturating_sub(1);
        }
        KeyCode::PageUp => scroll_log_back(app, 10),
        KeyCode::PageDown => {
            app.run.log_back = app.run.log_back.saturating_sub(10);
        }
        KeyCode::Char('g') => scroll_log_back(app, usize::MAX),
        KeyCode::Char('G') => app.run.log_back = 0,
        // With no backup running there is nothing to watch: leave.
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') if !app.run.is_active() => {
            app.go_dashboard();
        }
        _ => {}
    }
}

fn scroll_log_back(app: &mut App, by: usize) {
    let max = app.run.log_lines.len().saturating_sub(1);
    app.run.log_back = app.run.log_back.saturating_add(by).min(max);
}

fn handle_results(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('r') | KeyCode::Char('R') => request_backup(app),
        KeyCode::Char('d') | KeyCode::Esc => app.go_dashboard(),
        KeyCode::Char('c') | KeyCode::Char('C') => app.screen = Screen::Configure,
        KeyCode::Char('q') | KeyCode::Char('Q') => app.should_quit = true,
        KeyCode::Down | KeyCode::Char('j') => app.results.move_failure_selection(1),
        KeyCode::Up | KeyCode::Char('k') => app.results.move_failure_selection(-1),
        KeyCode::PageDown => app.results.move_failure_selection(10),
        KeyCode::PageUp => app.results.move_failure_selection(-10),
        KeyCode::Char('g') => app.results.failure_selected = 0,
        KeyCode::Char('G') => {
            app.results.failure_selected = app.results.failures.len().saturating_sub(1);
        }
        _ => {}
    }
}

fn handle_verify(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('v') | KeyCode::Char('V') if !app.verify.running => {
            if app.config.owner.trim().is_empty() || app.config.output_dir.trim().is_empty() {
                app.modal_error = Some("Configure owner and output directory first.".into());
            } else {
                app.verify.reset();
                app.verify.running = true;
                app.start_verify_requested = true;
            }
        }
        KeyCode::Char('d') | KeyCode::Esc => app.go_dashboard(),
        KeyCode::Char('j') | KeyCode::Down => app.verify.scroll_by(1),
        KeyCode::Char('k') | KeyCode::Up => app.verify.scroll_by(-1),
        KeyCode::PageDown => app.verify.scroll_by(10),
        KeyCode::PageUp => app.verify.scroll_by(-10),
        KeyCode::Char('q') | KeyCode::Char('Q') => app.should_quit = true,
        _ => {}
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn request_backup(app: &mut App) {
    if app.run.is_active() {
        return;
    }
    if let Some(err) = app.config.validate() {
        app.modal_error = Some(err);
        return;
    }
    app.run.reset();
    app.run.started_at = Some(Instant::now());
    app.run.phase = "Connecting to GitHub".into();
    app.run.status = RunStatus::Active;
    app.start_backup_requested = true;
    app.cancel_tx = None;
    app.screen = Screen::Running;
}

fn enter_field(app: &mut App) {
    if let Some(v) = get_text_value(app) {
        app.config.edit_buffer = v;
        app.config.editing = true;
    } else {
        toggle_field(app);
    }
}

fn commit_edit(app: &mut App) {
    let buf = app.config.edit_buffer.clone();
    set_text_value(app, &buf);
    app.config.editing = false;
    app.config.edit_buffer.clear();
}

/// Leaves edit mode without touching the stored value.
fn cancel_edit(app: &mut App) {
    app.config.editing = false;
    app.config.edit_buffer.clear();
}

fn get_text_value(app: &App) -> Option<String> {
    let (tab, f) = (app.config.active_tab, app.config.active_field);
    match (tab, f) {
        (0, 0) => Some(app.config.token.clone()),
        (0, 1) => Some(app.config.api_url.clone()),
        (1, 0) => Some(app.config.owner.clone()),
        (1, 1) => Some(app.config.output_dir.clone()),
        (1, 3) => Some(app.config.since.clone()),
        (3, 6) => Some(app.config.concurrency.clone()),
        (4, 0) => Some(app.config.include_repos.clone()),
        (4, 1) => Some(app.config.exclude_repos.clone()),
        _ => None,
    }
}

fn set_text_value(app: &mut App, value: &str) {
    let (tab, f) = (app.config.active_tab, app.config.active_field);
    match (tab, f) {
        (0, 0) => app.config.token = value.to_string(),
        (0, 1) => app.config.api_url = value.to_string(),
        (1, 0) => app.config.owner = value.to_string(),
        (1, 1) => app.config.output_dir = value.to_string(),
        (1, 3) => app.config.since = value.to_string(),
        (3, 6) => app.config.concurrency = value.to_string(),
        (4, 0) => app.config.include_repos = value.to_string(),
        (4, 1) => app.config.exclude_repos = value.to_string(),
        _ => {}
    }
}

fn toggle_field(app: &mut App) {
    let (tab, f) = (app.config.active_tab, app.config.active_field);
    match (tab, f) {
        (1, 2) => app.config.org_mode = !app.config.org_mode,
        (1, 4) => app.config.full = !app.config.full,
        (2, _) => toggle_category(app, f),
        (3, 1) => app.config.forks = !app.config.forks,
        (3, 2) => app.config.private = !app.config.private,
        (3, 3) => app.config.lfs = !app.config.lfs,
        (3, 4) => app.config.prefer_ssh = !app.config.prefer_ssh,
        (3, 5) => app.config.prune = !app.config.prune,
        (5, 0) => app.config.manifest = !app.config.manifest,
        (5, 1) => app.config.dry_run = !app.config.dry_run,
        _ => {}
    }
}

fn toggle_category(app: &mut App, idx: usize) {
    match idx {
        0 => app.config.repositories = !app.config.repositories,
        1 => app.config.issues = !app.config.issues,
        2 => app.config.issue_comments = !app.config.issue_comments,
        3 => app.config.issue_events = !app.config.issue_events,
        4 => app.config.pulls = !app.config.pulls,
        5 => app.config.pull_comments = !app.config.pull_comments,
        6 => app.config.pull_commits = !app.config.pull_commits,
        7 => app.config.pull_reviews = !app.config.pull_reviews,
        8 => app.config.labels = !app.config.labels,
        9 => app.config.milestones = !app.config.milestones,
        10 => app.config.releases = !app.config.releases,
        11 => app.config.release_assets = !app.config.release_assets,
        12 => app.config.hooks = !app.config.hooks,
        13 => app.config.security_advisories = !app.config.security_advisories,
        14 => app.config.wikis = !app.config.wikis,
        15 => app.config.starred = !app.config.starred,
        16 => app.config.clone_starred = !app.config.clone_starred,
        17 => app.config.watched = !app.config.watched,
        18 => app.config.followers = !app.config.followers,
        19 => app.config.following = !app.config.following,
        20 => app.config.gists = !app.config.gists,
        21 => app.config.starred_gists = !app.config.starred_gists,
        22 => app.config.topics = !app.config.topics,
        23 => app.config.branches = !app.config.branches,
        24 => app.config.deploy_keys = !app.config.deploy_keys,
        25 => app.config.collaborators = !app.config.collaborators,
        26 => app.config.org_members = !app.config.org_members,
        27 => app.config.org_teams = !app.config.org_teams,
        28 => app.config.actions = !app.config.actions,
        29 => app.config.action_runs = !app.config.action_runs,
        30 => app.config.environments = !app.config.environments,
        31 => app.config.discussions = !app.config.discussions,
        32 => app.config.projects = !app.config.projects,
        33 => app.config.packages = !app.config.packages,
        _ => {}
    }
}

fn cycle_select(app: &mut App, delta: i32) {
    let (tab, f) = (app.config.active_tab, app.config.active_field);
    if (tab, f) == (3, 0) {
        let n = CloneTypeForm::OPTIONS.len() as i32;
        let i = ((app.config.clone_type.idx() as i32 + delta).rem_euclid(n)) as usize;
        app.config.clone_type = CloneTypeForm::from_idx(i);
    }
}
