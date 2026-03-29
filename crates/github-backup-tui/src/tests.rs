// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Unit tests for the TUI state machine.
//!
//! These tests exercise every code path that does not require a live terminal:
//! keyboard dispatch, backup-event handling, form field editing, validation,
//! config conversion, and progress tracking.

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use crate::app::{handle_backup_event, handle_key_dispatch, App, InitialConfig};
use crate::event::BackupEvent;
use crate::state::{
    CloneTypeForm, MirrorTypeForm, RepoStatus, ResultsState, RunState, Screen,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_app() -> App {
    App::new(InitialConfig::default())
}

fn press(app: &mut App, code: KeyCode) {
    handle_key_dispatch(app, code, KeyModifiers::NONE);
}

fn ctrl(app: &mut App, ch: char) {
    handle_key_dispatch(app, KeyCode::Char(ch), KeyModifiers::CONTROL);
}

// ── InitialConfig pre-population ──────────────────────────────────────────────

#[test]
fn initial_config_populates_fields() {
    let app = App::new(InitialConfig {
        token:   Some("ghp_abc".into()),
        owner:   Some("octocat".into()),
        output:  Some("/var/backup".into()),
        api_url: Some("https://ghe.example.com/api/v3".into()),
    });
    assert_eq!(app.config.token,      "ghp_abc");
    assert_eq!(app.config.owner,      "octocat");
    assert_eq!(app.config.output_dir, "/var/backup");
    assert_eq!(app.config.api_url,    "https://ghe.example.com/api/v3");
}

#[test]
fn initial_config_defaults_are_sane() {
    let app = make_app();
    assert!(app.config.token.is_empty());
    assert!(app.config.owner.is_empty());
    assert_eq!(app.config.output_dir, "./github-backup");
    assert_eq!(app.config.concurrency, "4");
    assert!(app.config.repositories); // repos enabled by default
}

// ── Global navigation ─────────────────────────────────────────────────────────

#[test]
fn number_keys_switch_screens() {
    let mut app = make_app();
    press(&mut app, KeyCode::Char('2'));
    assert_eq!(app.screen, Screen::Configure);
    press(&mut app, KeyCode::Char('1'));
    assert_eq!(app.screen, Screen::Dashboard);
    press(&mut app, KeyCode::Char('4'));
    assert_eq!(app.screen, Screen::Verify);
    press(&mut app, KeyCode::Char('5'));
    assert_eq!(app.screen, Screen::Results);
    press(&mut app, KeyCode::Char('3'));
    assert_eq!(app.screen, Screen::Running);
}

#[test]
fn ctrl_c_on_dashboard_quits() {
    let mut app = make_app();
    assert!(!app.should_quit);
    ctrl(&mut app, 'c');
    assert!(app.should_quit);
}

#[test]
fn q_on_dashboard_quits() {
    let mut app = make_app();
    press(&mut app, KeyCode::Char('q'));
    assert!(app.should_quit);
}

// ── Modal error dismissal ─────────────────────────────────────────────────────

#[test]
fn any_key_dismisses_modal_error() {
    let mut app = make_app();
    app.modal_error = Some("something went wrong".into());
    press(&mut app, KeyCode::Char('x'));
    assert!(app.modal_error.is_none());
    assert!(!app.should_quit); // should NOT quit
}

#[test]
fn modal_blocks_screen_switch() {
    let mut app = make_app();
    app.modal_error = Some("error".into());
    press(&mut app, KeyCode::Char('2')); // would normally go to Configure
    // The modal should have been dismissed but screen unchanged
    assert!(app.modal_error.is_none());
    assert_eq!(app.screen, Screen::Dashboard); // still Dashboard
}

// ── Dashboard navigation ──────────────────────────────────────────────────────

#[test]
fn dashboard_j_k_navigate_actions() {
    let mut app = make_app();
    assert_eq!(app.dashboard.selected_action, 0);
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.dashboard.selected_action, 1);
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.dashboard.selected_action, 2);
    press(&mut app, KeyCode::Char('k'));
    assert_eq!(app.dashboard.selected_action, 1);
}

#[test]
fn dashboard_j_wraps_around() {
    let mut app = make_app();
    let n = crate::state::DashboardState::ACTIONS.len();
    for _ in 0..n {
        press(&mut app, KeyCode::Char('j'));
    }
    assert_eq!(app.dashboard.selected_action, 0);
}

#[test]
fn dashboard_c_goes_to_configure() {
    let mut app = make_app();
    press(&mut app, KeyCode::Char('c'));
    assert_eq!(app.screen, Screen::Configure);
}

#[test]
fn dashboard_v_goes_to_verify() {
    let mut app = make_app();
    press(&mut app, KeyCode::Char('v'));
    assert_eq!(app.screen, Screen::Verify);
    assert!(!app.verify.running); // not auto-started
}

#[test]
fn dashboard_r_without_config_shows_error() {
    let mut app = make_app(); // owner is empty
    press(&mut app, KeyCode::Char('r'));
    assert!(app.modal_error.is_some());
    assert_eq!(app.screen, Screen::Dashboard); // did not navigate away
}

#[test]
fn dashboard_r_with_valid_config_requests_backup() {
    let mut app = App::new(InitialConfig {
        token: Some("ghp_x".into()),
        owner: Some("octocat".into()),
        output: Some("/tmp/bk".into()),
        ..Default::default()
    });
    press(&mut app, KeyCode::Char('r'));
    assert!(app.modal_error.is_none());
    assert_eq!(app.screen, Screen::Running);
    assert!(app.start_backup_requested);
    assert!(app.run.started_at.is_some());
}

// ── Configure tab navigation ──────────────────────────────────────────────────

#[test]
fn configure_tab_cycles_forward() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    let n = crate::state::ConfigState::TAB_COUNT;
    for i in 1..=n {
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.config.active_tab, i % n);
    }
}

#[test]
fn configure_backtab_cycles_backward() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.config.active_tab, crate::state::ConfigState::TAB_COUNT - 1);
}

#[test]
fn configure_tab_resets_field_index() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    app.config.active_field = 3;
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.config.active_field, 0);
}

#[test]
fn configure_j_k_navigate_fields() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    // Tab 0 has 4 fields
    assert_eq!(app.config.active_tab, 0);
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.config.active_field, 1);
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.config.active_field, 2);
    press(&mut app, KeyCode::Char('k'));
    assert_eq!(app.config.active_field, 1);
}

#[test]
fn configure_j_wraps_within_tab() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    let n = app.config.tab_field_count();
    for _ in 0..n {
        press(&mut app, KeyCode::Char('j'));
    }
    assert_eq!(app.config.active_field, 0);
}

#[test]
fn configure_esc_returns_to_dashboard() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Dashboard);
}

// ── Configure field editing ───────────────────────────────────────────────────

#[test]
fn configure_enter_begins_edit_on_text_field() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    // Tab 0, field 0 = token (text field)
    assert_eq!(app.config.active_tab, 0);
    assert_eq!(app.config.active_field, 0);
    press(&mut app, KeyCode::Enter);
    assert!(app.config.editing);
    assert_eq!(app.config.edit_buffer, ""); // token was empty
}

#[test]
fn configure_typing_updates_buffer() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    press(&mut app, KeyCode::Enter); // begin editing token
    press(&mut app, KeyCode::Char('g'));
    press(&mut app, KeyCode::Char('h'));
    press(&mut app, KeyCode::Char('p'));
    assert_eq!(app.config.edit_buffer, "ghp");
}

#[test]
fn configure_backspace_removes_char() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('b'));
    press(&mut app, KeyCode::Backspace);
    assert_eq!(app.config.edit_buffer, "a");
}

#[test]
fn configure_enter_commits_edit() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    press(&mut app, KeyCode::Enter); // start editing token
    press(&mut app, KeyCode::Char('g'));
    press(&mut app, KeyCode::Char('h'));
    press(&mut app, KeyCode::Char('p'));
    press(&mut app, KeyCode::Enter); // commit
    assert!(!app.config.editing);
    assert_eq!(app.config.token, "ghp");
    assert!(app.config.edit_buffer.is_empty());
}

#[test]
fn configure_esc_commits_edit_too() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('x'));
    press(&mut app, KeyCode::Esc);
    assert!(!app.config.editing);
    assert_eq!(app.config.token, "x");
}

#[test]
fn configure_number_keys_do_not_switch_screens_while_editing() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    press(&mut app, KeyCode::Enter); // start editing
    press(&mut app, KeyCode::Char('2')); // would switch to Configure (already there) — but goes into buffer
    assert!(app.config.editing);
    assert_eq!(app.config.edit_buffer, "2");
    assert_eq!(app.screen, Screen::Configure);
}

// ── Configure toggle fields ───────────────────────────────────────────────────

#[test]
fn configure_space_toggles_bool_field() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    // Tab 0, field 2 = device_auth
    app.config.active_field = 2;
    assert!(!app.config.device_auth);
    press(&mut app, KeyCode::Char(' '));
    assert!(app.config.device_auth);
    press(&mut app, KeyCode::Char(' '));
    assert!(!app.config.device_auth);
}

#[test]
fn configure_enter_on_toggle_field_also_toggles() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    app.config.active_field = 2; // device_auth
    press(&mut app, KeyCode::Enter);
    assert!(app.config.device_auth);
}

#[test]
fn configure_categories_space_toggles() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    app.config.active_tab = 2; // Categories
    app.config.active_field = 1; // issues
    assert!(!app.config.issues);
    press(&mut app, KeyCode::Char(' '));
    assert!(app.config.issues);
}

#[test]
fn configure_A_selects_all_categories() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    app.config.active_tab = 2;
    // First 'A' enables all (repositories was true, issues was false)
    app.config.repositories = false; // ensure "all off" first toggle
    press(&mut app, KeyCode::Char('A'));
    assert!(app.config.repositories);
    assert!(app.config.issues);
    assert!(app.config.pulls);
    assert!(app.config.releases);
    // Second 'A' disables all
    press(&mut app, KeyCode::Char('A'));
    assert!(!app.config.repositories);
    assert!(!app.config.issues);
}

// ── Configure select fields ───────────────────────────────────────────────────

#[test]
fn configure_left_right_cycle_clone_type() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    app.config.active_tab = 3;
    app.config.active_field = 0;
    assert_eq!(app.config.clone_type, CloneTypeForm::Mirror);
    press(&mut app, KeyCode::Right);
    assert_eq!(app.config.clone_type, CloneTypeForm::Bare);
    press(&mut app, KeyCode::Right);
    assert_eq!(app.config.clone_type, CloneTypeForm::Full);
    press(&mut app, KeyCode::Right);
    assert_eq!(app.config.clone_type, CloneTypeForm::Shallow);
    press(&mut app, KeyCode::Right); // wraps
    assert_eq!(app.config.clone_type, CloneTypeForm::Mirror);
    press(&mut app, KeyCode::Left); // wraps backward
    assert_eq!(app.config.clone_type, CloneTypeForm::Shallow);
}

#[test]
fn configure_left_right_cycle_mirror_type() {
    let mut app = make_app();
    app.screen = Screen::Configure;
    app.config.active_tab = 5;
    app.config.active_field = 1;
    assert_eq!(app.config.mirror_type, MirrorTypeForm::Gitea);
    press(&mut app, KeyCode::Right);
    assert_eq!(app.config.mirror_type, MirrorTypeForm::Gitlab);
    press(&mut app, KeyCode::Right); // wraps
    assert_eq!(app.config.mirror_type, MirrorTypeForm::Gitea);
}

// ── ConfigState validation ────────────────────────────────────────────────────

#[test]
fn validate_fails_without_owner() {
    let app = App::new(InitialConfig {
        token: Some("ghp_x".into()),
        ..Default::default()
    });
    assert!(app.config.validate().is_some());
}

#[test]
fn validate_fails_without_token_or_device_auth() {
    let app = App::new(InitialConfig {
        owner: Some("octocat".into()),
        ..Default::default()
    });
    assert!(app.config.validate().is_some());
}

#[test]
fn validate_passes_with_owner_and_token() {
    let app = App::new(InitialConfig {
        token: Some("ghp_x".into()),
        owner: Some("octocat".into()),
        ..Default::default()
    });
    assert!(app.config.validate().is_none());
}

#[test]
fn validate_passes_with_owner_and_device_auth() {
    let mut app = App::new(InitialConfig {
        owner: Some("octocat".into()),
        ..Default::default()
    });
    app.config.device_auth = true;
    assert!(app.config.validate().is_none());
}

// ── ConfigState::to_backup_config ────────────────────────────────────────────

#[test]
fn to_backup_config_empty_token_gives_none() {
    let app = make_app();
    let (_, _, _, token) = app.config.to_backup_config();
    assert!(token.is_none());
}

#[test]
fn to_backup_config_token_trimmed() {
    let mut app = make_app();
    app.config.token = "  ghp_abc  ".into();
    let (_, _, _, token) = app.config.to_backup_config();
    assert_eq!(token, Some("ghp_abc".into()));
}

#[test]
fn to_backup_config_concurrency_defaults_to_4() {
    let mut app = make_app();
    app.config.concurrency = "not-a-number".into();
    let (_, _, opts, _) = app.config.to_backup_config();
    assert_eq!(opts.concurrency, 4);
}

#[test]
fn to_backup_config_concurrency_parsed_correctly() {
    let mut app = make_app();
    app.config.concurrency = "8".into();
    let (_, _, opts, _) = app.config.to_backup_config();
    assert_eq!(opts.concurrency, 8);
}

#[test]
fn to_backup_config_org_mode_sets_target() {
    use github_backup_types::config::BackupTarget;
    let mut app = make_app();
    app.config.org_mode = true;
    let (_, _, opts, _) = app.config.to_backup_config();
    assert_eq!(opts.target, BackupTarget::Org);
}

#[test]
fn to_backup_config_include_repos_parsed() {
    let mut app = make_app();
    app.config.include_repos = "rust-*, *-backup, my-repo".into();
    let (_, _, opts, _) = app.config.to_backup_config();
    assert_eq!(opts.include_repos, vec!["rust-*", "*-backup", "my-repo"]);
}

#[test]
fn to_backup_config_empty_include_repos_is_empty_vec() {
    let app = make_app();
    let (_, _, opts, _) = app.config.to_backup_config();
    assert!(opts.include_repos.is_empty());
}

#[test]
fn to_backup_config_clone_type_mirror() {
    use github_backup_types::config::CloneType;
    let app = make_app();
    let (_, _, opts, _) = app.config.to_backup_config();
    assert_eq!(opts.clone_type, CloneType::Mirror);
}

#[test]
fn to_backup_config_clone_type_bare() {
    use github_backup_types::config::CloneType;
    let mut app = make_app();
    app.config.clone_type = CloneTypeForm::Bare;
    let (_, _, opts, _) = app.config.to_backup_config();
    assert_eq!(opts.clone_type, CloneType::Bare);
}

#[test]
fn to_backup_config_since_empty_gives_none() {
    let app = make_app();
    let (_, _, opts, _) = app.config.to_backup_config();
    assert!(opts.since.is_none());
}

#[test]
fn to_backup_config_since_trimmed() {
    let mut app = make_app();
    app.config.since = "  2026-01-01T00:00:00Z  ".into();
    let (_, _, opts, _) = app.config.to_backup_config();
    assert_eq!(opts.since, Some("2026-01-01T00:00:00Z".into()));
}

// ── BackupEvent handling ──────────────────────────────────────────────────────

#[test]
fn backup_event_log_line_appended() {
    let mut app = make_app();
    handle_backup_event(&mut app, BackupEvent::LogLine {
        timestamp: "12:00:00".into(),
        level: "INFO".into(),
        message: "starting backup".into(),
    });
    assert_eq!(app.run.log_lines.len(), 1);
    assert_eq!(app.run.log_lines[0].message, "starting backup");
}

#[test]
fn backup_event_repos_discovered() {
    let mut app = make_app();
    handle_backup_event(&mut app, BackupEvent::ReposDiscovered { total: 42 });
    assert_eq!(app.run.total_repos, 42);
    assert!(app.run.phase.contains("42"));
}

#[test]
fn backup_event_repo_started_adds_entry() {
    let mut app = make_app();
    handle_backup_event(&mut app, BackupEvent::RepoStarted {
        name: "octocat/hello-world".into(),
    });
    assert_eq!(app.run.repos.len(), 1);
    assert_eq!(app.run.repos[0].status, RepoStatus::Running);
}

#[test]
fn backup_event_repo_started_updates_existing() {
    let mut app = make_app();
    handle_backup_event(&mut app, BackupEvent::RepoStarted {
        name: "octocat/hello-world".into(),
    });
    handle_backup_event(&mut app, BackupEvent::RepoStarted {
        name: "octocat/hello-world".into(),
    });
    assert_eq!(app.run.repos.len(), 1); // no duplicate
}

#[test]
fn backup_event_repo_completed_success() {
    let mut app = make_app();
    handle_backup_event(&mut app, BackupEvent::RepoStarted {
        name: "octocat/hello-world".into(),
    });
    handle_backup_event(&mut app, BackupEvent::RepoCompleted {
        name: "octocat/hello-world".into(),
        success: true,
    });
    assert_eq!(app.run.repos[0].status, RepoStatus::Done);
    assert_eq!(app.run.repos_done, 1);
    assert_eq!(app.run.repos_errored, 0);
}

#[test]
fn backup_event_repo_completed_failure() {
    let mut app = make_app();
    handle_backup_event(&mut app, BackupEvent::RepoStarted { name: "r".into() });
    handle_backup_event(&mut app, BackupEvent::RepoCompleted {
        name: "r".into(), success: false,
    });
    assert_eq!(app.run.repos[0].status, RepoStatus::Error);
    assert_eq!(app.run.repos_errored, 1);
    assert_eq!(app.run.repos_done, 0);
}

#[test]
fn backup_event_done_transitions_to_results() {
    let mut app = App::new(InitialConfig {
        owner: Some("octocat".into()),
        output: Some("/tmp/bk".into()),
        token: Some("ghp_x".into()),
        ..Default::default()
    });
    app.screen = Screen::Running;

    handle_backup_event(&mut app, BackupEvent::BackupDone {
        repos_backed_up: 10,
        repos_discovered: 12,
        repos_skipped: 2,
        repos_errored: 0,
        gists_backed_up: 3,
        issues_fetched: 100,
        prs_fetched: 50,
        workflows_fetched: 20,
        discussions_fetched: 0,
        elapsed_secs: 42.5,
    });

    assert_eq!(app.screen, Screen::Results);
    assert!(app.results.success);
    assert_eq!(app.results.repos_backed_up, 10);
    assert_eq!(app.results.repos_discovered, 12);
    assert_eq!(app.results.repos_skipped, 2);
    assert_eq!(app.results.elapsed_secs, 42.5);
    assert_eq!(app.results.owner, "octocat");
}

#[test]
fn backup_event_failed_transitions_to_results() {
    let mut app = make_app();
    app.screen = Screen::Running;
    handle_backup_event(&mut app, BackupEvent::BackupFailed {
        error: "rate limit exceeded".into(),
    });
    assert_eq!(app.screen, Screen::Results);
    assert!(!app.results.success);
    assert_eq!(
        app.results.error_message.as_deref(),
        Some("rate limit exceeded")
    );
}

#[test]
fn backup_event_verify_done() {
    let mut app = make_app();
    app.verify.running = true;
    handle_backup_event(&mut app, BackupEvent::VerifyDone {
        ok: 123,
        tampered: vec!["file.json".into()],
        missing: vec![],
        unexpected: vec![],
    });
    assert!(!app.verify.running);
    assert!(app.verify.done);
    assert_eq!(app.verify.ok, 123);
    assert_eq!(app.verify.tampered, vec!["file.json"]);
    assert!(app.verify.is_clean() == false);
}

#[test]
fn backup_event_verify_failed() {
    let mut app = make_app();
    app.verify.running = true;
    handle_backup_event(&mut app, BackupEvent::VerifyFailed {
        error: "manifest not found".into(),
    });
    assert!(!app.verify.running);
    assert_eq!(
        app.verify.error.as_deref(),
        Some("manifest not found")
    );
}

// ── Running screen controls ───────────────────────────────────────────────────

#[test]
fn running_ctrl_c_cancels_if_running() {
    let mut app = make_app();
    app.screen = Screen::Running;
    let (tx, _rx) = tokio::sync::oneshot::channel::<()>();
    app.cancel_tx = Some(tx);
    ctrl(&mut app, 'c');
    assert!(app.cancel_tx.is_none()); // consumed
    assert!(!app.should_quit);
}

#[test]
fn running_j_k_scroll_repo_list() {
    let mut app = make_app();
    app.screen = Screen::Running;
    // Add some repos so there's something to scroll.
    for i in 0..10 {
        app.run.repos.push(crate::state::RepoEntry {
            name: format!("repo-{i}"),
            status: RepoStatus::Done,
        });
    }
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.run.repo_list_offset, 1);
    press(&mut app, KeyCode::Char('k'));
    assert_eq!(app.run.repo_list_offset, 0);
    press(&mut app, KeyCode::Char('k')); // clamps at 0
    assert_eq!(app.run.repo_list_offset, 0);
}

// ── Results screen controls ───────────────────────────────────────────────────

#[test]
fn results_d_returns_to_dashboard() {
    let mut app = make_app();
    app.screen = Screen::Results;
    press(&mut app, KeyCode::Char('d'));
    assert_eq!(app.screen, Screen::Dashboard);
}

#[test]
fn results_esc_returns_to_dashboard() {
    let mut app = make_app();
    app.screen = Screen::Results;
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Dashboard);
}

#[test]
fn results_q_quits() {
    let mut app = make_app();
    app.screen = Screen::Results;
    press(&mut app, KeyCode::Char('q'));
    assert!(app.should_quit);
}

// ── Verify screen controls ────────────────────────────────────────────────────

#[test]
fn verify_v_without_config_shows_error() {
    let mut app = make_app(); // owner is empty
    app.screen = Screen::Verify;
    press(&mut app, KeyCode::Char('v'));
    assert!(app.modal_error.is_some());
    assert!(!app.verify.running);
}

#[test]
fn verify_v_with_config_starts_verify() {
    let mut app = App::new(InitialConfig {
        owner: Some("octocat".into()),
        output: Some("/tmp/bk".into()),
        ..Default::default()
    });
    app.screen = Screen::Verify;
    press(&mut app, KeyCode::Char('v'));
    assert!(app.verify.running);
    assert!(app.start_verify_requested);
}

#[test]
fn verify_j_k_scroll() {
    let mut app = make_app();
    app.screen = Screen::Verify;
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.verify.scroll, 1);
    press(&mut app, KeyCode::Char('k'));
    assert_eq!(app.verify.scroll, 0);
    press(&mut app, KeyCode::Char('k')); // clamps
    assert_eq!(app.verify.scroll, 0);
}

#[test]
fn verify_esc_returns_to_dashboard() {
    let mut app = make_app();
    app.screen = Screen::Verify;
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Dashboard);
}

// ── RunState helpers ──────────────────────────────────────────────────────────

#[test]
fn run_state_progress_pct_zero_when_no_total() {
    let run = RunState::default();
    assert_eq!(run.progress_pct(), 0);
}

#[test]
fn run_state_progress_pct_correct() {
    let mut run = RunState::default();
    run.total_repos = 10;
    run.repos_done = 5;
    assert_eq!(run.progress_pct(), 50);
}

#[test]
fn run_state_progress_pct_clamps_at_100() {
    let mut run = RunState::default();
    run.total_repos = 10;
    run.repos_done = 15; // over
    assert_eq!(run.progress_pct(), 100);
}

#[test]
fn run_state_elapsed_str_format() {
    let run = RunState::default();
    // No timer started — returns 00:00:00
    assert_eq!(run.elapsed_str(), "00:00:00");
}

#[test]
fn run_state_push_log_caps_at_2000() {
    let mut run = RunState::default();
    for i in 0..2100u32 {
        run.push_log(crate::state::LogLine {
            timestamp: "00:00:00".into(),
            level: "INFO".into(),
            message: format!("line {i}"),
        });
    }
    assert_eq!(run.log_lines.len(), 2000);
    // Newest line should be last.
    assert!(run.log_lines.last().unwrap().message.contains("2099"));
}

// ── ResultsState helpers ──────────────────────────────────────────────────────

#[test]
fn results_elapsed_str() {
    let mut res = ResultsState::default();
    res.elapsed_secs = 3723.0; // 1h 2m 3s
    assert_eq!(res.elapsed_str(), "01:02:03");
}

// ── VerifyState helpers ───────────────────────────────────────────────────────

#[test]
fn verify_state_is_clean_when_no_issues() {
    let mut v = crate::state::VerifyState::default();
    v.done = true;
    v.ok = 10;
    assert!(v.is_clean());
}

#[test]
fn verify_state_not_clean_with_tampered() {
    let mut v = crate::state::VerifyState::default();
    v.done = true;
    v.tampered = vec!["file.json".into()];
    assert!(!v.is_clean());
}

#[test]
fn verify_state_reset_clears_all() {
    let mut v = crate::state::VerifyState {
        running: true,
        done: true,
        ok: 99,
        tampered: vec!["x".into()],
        missing: vec!["y".into()],
        unexpected: vec![],
        error: Some("err".into()),
        scroll: 5,
    };
    v.reset();
    assert!(!v.running);
    assert!(!v.done);
    assert_eq!(v.ok, 0);
    assert!(v.tampered.is_empty());
    assert!(v.error.is_none());
    assert_eq!(v.scroll, 0);
}
