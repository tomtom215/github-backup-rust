// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! [`App`] struct plus keyboard / progress event handling.
//!
//! Rendering lives in `lib.rs` and the `screens/` modules; this module is
//! purely state + event dispatch.

use std::time::Instant;

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    event::BackupEvent,
    state::{
        CloneTypeForm, ConfigState, DashboardState, LogLine, MirrorTypeForm, RepoEntry, RepoStatus,
        ResultsState, RunState, Screen, VerifyState,
    },
};

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
}

/// Initial configuration pre-populated from CLI arguments.
#[derive(Default)]
pub struct InitialConfig {
    pub token: Option<String>,
    pub owner: Option<String>,
    pub output: Option<String>,
    pub api_url: Option<String>,
}

impl App {
    pub fn new(initial: InitialConfig) -> Self {
        let mut config = ConfigState::default();
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
        }
    }

    pub fn reload_dashboard(&mut self) {
        self.dashboard = load_dashboard_state(&self.config);
    }
}

fn load_dashboard_state(config: &ConfigState) -> DashboardState {
    let mut dash = DashboardState::default();
    if config.owner.is_empty() || config.output_dir.is_empty() {
        return dash;
    }
    let state_path = std::path::PathBuf::from(&config.output_dir)
        .join(&config.owner)
        .join("json")
        .join("backup_state.json");

    if let Ok(Some(s)) = github_backup_types::backup_state::BackupState::load(&state_path) {
        dash.last_backup_time = Some(s.last_successful_run.clone());
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
        if app.screen == Screen::Running {
            if let Some(tx) = app.cancel_tx.take() {
                let _ = tx.send(());
            }
        } else {
            app.should_quit = true;
        }
        return;
    }

    // Global number shortcuts when not editing text.
    if !app.config.editing && app.screen != Screen::Running {
        match code {
            KeyCode::Char('1') => {
                app.screen = Screen::Dashboard;
                return;
            }
            KeyCode::Char('2') => {
                app.screen = Screen::Configure;
                return;
            }
            KeyCode::Char('3') => {
                app.screen = Screen::Running;
                return;
            }
            KeyCode::Char('4') => {
                app.screen = Screen::Verify;
                return;
            }
            KeyCode::Char('5') => {
                app.screen = Screen::Results;
                return;
            }
            _ => {}
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
                });
            }
        }
        BackupEvent::RepoCompleted { name, success } => {
            let status = if success {
                RepoStatus::Done
            } else {
                RepoStatus::Error
            };
            if let Some(e) = app.run.repos.iter_mut().find(|r| r.name == name) {
                e.status = status;
            } else {
                app.run.repos.push(RepoEntry { name, status });
            }
            if success {
                app.run.repos_done += 1;
            } else {
                app.run.repos_errored += 1;
            }
        }
        BackupEvent::ReposDiscovered { total } => {
            app.run.total_repos = total;
            app.run.phase = format!("Backing up {total} repos");
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
        } => {
            app.results = ResultsState {
                success: true,
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
                owner: app.config.owner.clone(),
                output_dir: app.config.output_dir.clone(),
            };
            app.run.phase = "Complete".into();
            app.reload_dashboard();
            app.screen = Screen::Results;
        }
        BackupEvent::BackupFailed { error } => {
            app.results = ResultsState {
                success: false,
                error_message: Some(error),
                owner: app.config.owner.clone(),
                output_dir: app.config.output_dir.clone(),
                elapsed_secs: app
                    .run
                    .started_at
                    .map(|s| s.elapsed().as_secs_f64())
                    .unwrap_or(0.0),
                ..Default::default()
            };
            app.run.phase = "Failed".into();
            app.screen = Screen::Results;
        }
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
        KeyCode::Esc => app.screen = Screen::Dashboard,
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
        KeyCode::Char('A') if app.config.active_tab == 2 => {
            let all = !app.config.repositories;
            app.config.set_all_categories(all);
        }
        KeyCode::F(5) | KeyCode::Char('s') => request_backup(app),
        _ => {}
    }
}

fn handle_configure_editing(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Enter | KeyCode::Esc => commit_edit(app),
        KeyCode::Backspace => {
            app.config.edit_buffer.pop();
        }
        KeyCode::Char(c) => app.config.edit_buffer.push(c),
        _ => {}
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
        KeyCode::Char('g') => {
            app.run.log_offset = app.run.log_offset.saturating_sub(5);
        }
        KeyCode::Char('G') => {
            app.run.log_offset = app.run.log_lines.len().saturating_sub(1);
        }
        _ => {}
    }
}

fn handle_results(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('r') | KeyCode::Char('R') => request_backup(app),
        KeyCode::Char('d') | KeyCode::Esc => app.screen = Screen::Dashboard,
        KeyCode::Char('c') | KeyCode::Char('C') => app.screen = Screen::Configure,
        KeyCode::Char('q') | KeyCode::Char('Q') => app.should_quit = true,
        _ => {}
    }
}

fn handle_verify(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('v') | KeyCode::Char('V') if !app.verify.running => {
            if app.config.owner.is_empty() || app.config.output_dir.is_empty() {
                app.modal_error = Some("Configure owner and output directory first.".into());
            } else {
                app.verify.reset();
                app.verify.running = true;
                app.start_verify_requested = true;
            }
        }
        KeyCode::Char('d') | KeyCode::Esc => app.screen = Screen::Dashboard,
        KeyCode::Char('j') | KeyCode::Down => app.verify.scroll += 1,
        KeyCode::Char('k') | KeyCode::Up => {
            app.verify.scroll = app.verify.scroll.saturating_sub(1);
        }
        KeyCode::Char('q') | KeyCode::Char('Q') => app.should_quit = true,
        _ => {}
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn request_backup(app: &mut App) {
    if let Some(err) = app.config.validate() {
        app.modal_error = Some(err);
        return;
    }
    app.run.reset();
    app.run.started_at = Some(Instant::now());
    app.run.phase = "Connecting to GitHub".into();
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

fn get_text_value(app: &App) -> Option<String> {
    let (tab, f) = (app.config.active_tab, app.config.active_field);
    match (tab, f) {
        (0, 0) => Some(app.config.token.clone()),
        (0, 1) => Some(app.config.api_url.clone()),
        (0, 3) => Some(app.config.oauth_client_id.clone()),
        (1, 0) => Some(app.config.owner.clone()),
        (1, 1) => Some(app.config.output_dir.clone()),
        (1, 3) => Some(app.config.since.clone()),
        (3, 6) => Some(app.config.concurrency.clone()),
        (4, 0) => Some(app.config.include_repos.clone()),
        (4, 1) => Some(app.config.exclude_repos.clone()),
        (5, 0) => Some(app.config.mirror_to.clone()),
        (5, 2) => Some(app.config.mirror_token.clone()),
        (5, 3) => Some(app.config.mirror_owner.clone()),
        (6, 0) => Some(app.config.s3_bucket.clone()),
        (6, 1) => Some(app.config.s3_region.clone()),
        (6, 2) => Some(app.config.s3_prefix.clone()),
        (6, 3) => Some(app.config.s3_endpoint.clone()),
        (6, 4) => Some(app.config.s3_access_key.clone()),
        (6, 5) => Some(app.config.s3_secret_key.clone()),
        (7, 2) => Some(app.config.report.clone()),
        (7, 3) => Some(app.config.prometheus_metrics.clone()),
        (7, 4) => Some(app.config.keep_last.clone()),
        (7, 5) => Some(app.config.max_age_days.clone()),
        _ => None,
    }
}

fn set_text_value(app: &mut App, value: &str) {
    let (tab, f) = (app.config.active_tab, app.config.active_field);
    match (tab, f) {
        (0, 0) => app.config.token = value.to_string(),
        (0, 1) => app.config.api_url = value.to_string(),
        (0, 3) => app.config.oauth_client_id = value.to_string(),
        (1, 0) => app.config.owner = value.to_string(),
        (1, 1) => app.config.output_dir = value.to_string(),
        (1, 3) => app.config.since = value.to_string(),
        (3, 6) => app.config.concurrency = value.to_string(),
        (4, 0) => app.config.include_repos = value.to_string(),
        (4, 1) => app.config.exclude_repos = value.to_string(),
        (5, 0) => app.config.mirror_to = value.to_string(),
        (5, 2) => app.config.mirror_token = value.to_string(),
        (5, 3) => app.config.mirror_owner = value.to_string(),
        (6, 0) => app.config.s3_bucket = value.to_string(),
        (6, 1) => app.config.s3_region = value.to_string(),
        (6, 2) => app.config.s3_prefix = value.to_string(),
        (6, 3) => app.config.s3_endpoint = value.to_string(),
        (6, 4) => app.config.s3_access_key = value.to_string(),
        (6, 5) => app.config.s3_secret_key = value.to_string(),
        (7, 2) => app.config.report = value.to_string(),
        (7, 3) => app.config.prometheus_metrics = value.to_string(),
        (7, 4) => app.config.keep_last = value.to_string(),
        (7, 5) => app.config.max_age_days = value.to_string(),
        _ => {}
    }
}

fn toggle_field(app: &mut App) {
    let (tab, f) = (app.config.active_tab, app.config.active_field);
    match (tab, f) {
        (0, 2) => app.config.device_auth = !app.config.device_auth,
        (1, 2) => app.config.org_mode = !app.config.org_mode,
        (2, _) => toggle_category(app, f),
        (3, 1) => app.config.forks = !app.config.forks,
        (3, 2) => app.config.private = !app.config.private,
        (3, 3) => app.config.lfs = !app.config.lfs,
        (3, 4) => app.config.prefer_ssh = !app.config.prefer_ssh,
        (3, 5) => app.config.no_prune = !app.config.no_prune,
        (5, 4) => app.config.mirror_private = !app.config.mirror_private,
        (6, 6) => app.config.s3_include_assets = !app.config.s3_include_assets,
        (7, 0) => app.config.manifest = !app.config.manifest,
        (7, 1) => app.config.dry_run = !app.config.dry_run,
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
    match (tab, f) {
        (3, 0) => {
            let n = CloneTypeForm::OPTIONS.len() as i32;
            let i = ((app.config.clone_type.idx() as i32 + delta).rem_euclid(n)) as usize;
            app.config.clone_type = CloneTypeForm::from_idx(i);
        }
        (5, 1) => {
            let n = MirrorTypeForm::OPTIONS.len() as i32;
            let i = ((app.config.mirror_type.idx() as i32 + delta).rem_euclid(n)) as usize;
            app.config.mirror_type = MirrorTypeForm::from_idx(i);
        }
        _ => {}
    }
}
