// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Render and flow tests: what the screens actually show, at several sizes,
//! and how the backup life cycle (start, cancel, finish, fail) moves the UI.

use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Terminal;

use github_backup_core::{BackupStats, Failure};

use crate::app::{handle_backup_event, handle_key_dispatch, handle_paste, App, InitialConfig};
use crate::event::BackupEvent;
use crate::state::{LogLine, Outcome, RepoStatus, RunStatus, Screen};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn ready_app() -> App {
    App::new(InitialConfig {
        token: Some("dummy-token-for-tests".into()),
        owner: Some("octocat".into()),
        output: Some("/tmp/gbk-tui-test-out".into()),
        ..Default::default()
    })
}

fn press(app: &mut App, code: KeyCode) {
    handle_key_dispatch(app, code, KeyModifiers::NONE);
}

fn ctrl(app: &mut App, ch: char) {
    handle_key_dispatch(app, KeyCode::Char(ch), KeyModifiers::CONTROL);
}

/// Renders `app` at `w`x`h` and returns the screen as text, one line per row.
fn dump(app: &App, w: u16, h: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("test terminal");
    terminal
        .draw(|frame| crate::render(frame, app))
        .expect("draw must not fail");
    let buf = terminal.backend().buffer().clone();
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

fn text(app: &App, w: u16, h: u16) -> String {
    dump(app, w, h).join("\n")
}

fn failure(scope: &str, step: &str, message: &str) -> Failure {
    Failure {
        scope: scope.into(),
        step: step.into(),
        message: message.into(),
    }
}

fn done_event(failures: Vec<Failure>, errored: u64, dry_run: bool) -> BackupEvent {
    BackupEvent::BackupDone {
        repos_backed_up: 7,
        repos_discovered: 10,
        repos_skipped: 1,
        repos_errored: errored,
        gists_backed_up: 2,
        issues_fetched: 30,
        prs_fetched: 4,
        workflows_fetched: 5,
        discussions_fetched: 6,
        elapsed_secs: 65.0,
        failures,
        dry_run,
    }
}

fn app_with_results(ev: BackupEvent) -> App {
    let mut app = ready_app();
    app.run.status = RunStatus::Active;
    handle_backup_event(&mut app, ev);
    app
}

const SIZES: &[(u16, u16)] = &[
    (1, 1),
    (2, 2),
    (10, 3),
    (20, 5),
    (29, 7),
    (30, 8),
    (40, 12),
    (60, 16),
    (80, 24),
    (83, 24),
    (120, 40),
    (200, 60),
    (80, 4),
    (30, 40),
];

fn every_screen(app: &mut App) -> Vec<Screen> {
    let _ = app;
    vec![
        Screen::Dashboard,
        Screen::Configure,
        Screen::Running,
        Screen::Verify,
        Screen::Results,
    ]
}

// ── Never panics, at any size ─────────────────────────────────────────────────

#[test]
fn every_screen_renders_at_every_size() {
    let mut app = app_with_results(done_event(
        vec![
            failure("octocat/a", "clone", "boom"),
            failure("gists", "list", "second"),
        ],
        1,
        false,
    ));
    app.verify.done = true;
    app.verify.tampered = (0..80).map(|i| format!("file-{i}.json")).collect();
    app.run.repos.push(crate::state::RepoEntry {
        name: "octocat/a".into(),
        status: RepoStatus::Error,
        error: Some("clone failed\nwith detail".into()),
    });
    app.run.total_repos = 10;
    for tab in 0..crate::state::ConfigState::TAB_COUNT {
        for field in 0..4 {
            for &(w, h) in SIZES {
                app.config.active_tab = tab;
                app.config.active_field = field;
                app.screen = Screen::Configure;
                let _ = dump(&app, w, h);
            }
        }
    }
    for screen in every_screen(&mut app) {
        for &(w, h) in SIZES {
            app.screen = screen.clone();
            let _ = dump(&app, w, h);
            // ... and with the error modal open and while editing
            app.modal_error = Some("A long error message that needs several rows to show".into());
            let _ = dump(&app, w, h);
            app.modal_error = None;
            app.config.editing = true;
            app.config.edit_buffer = "x".repeat(300);
            let _ = dump(&app, w, h);
            app.config.editing = false;
        }
    }
}

#[test]
fn error_modal_fits_inside_the_frame_at_every_size() {
    let msg = "Owner is required (Configure > Target tab) and then some more words";
    for w in 1..=120u16 {
        for h in [1u16, 3, 5, 7, 8, 9, 12, 24] {
            let area = ratatui::layout::Rect::new(0, 1, w, h);
            let r = crate::modal_rect(area, msg);
            assert!(r.x >= area.x && r.y >= area.y, "{w}x{h}: {r:?}");
            assert!(r.right() <= area.right(), "{w}x{h}: {r:?} exceeds {area:?}");
            assert!(
                r.bottom() <= area.bottom(),
                "{w}x{h}: {r:?} exceeds {area:?}"
            );
        }
    }
}

#[test]
fn too_small_terminal_says_so() {
    let app = ready_app();
    let t = text(&app, 20, 5);
    assert!(t.contains("too small"), "got:\n{t}");
    let t = text(&app, 80, 24);
    assert!(!t.contains("too small"));
}

// ── Results: the verdict follows the engine ──────────────────────────────────

#[test]
fn results_with_failures_are_incomplete_and_list_each_failure() {
    let app = app_with_results(done_event(
        vec![
            failure("octocat/alpha", "clone", "git clone failed: exit 128"),
            failure("gists", "list", "HTTP 502 from the API"),
            failure("post-processing", "manifest", "failed to write manifest"),
        ],
        1,
        false,
    ));
    assert_eq!(app.results.outcome, Outcome::Incomplete);
    for (w, h) in [(80u16, 24u16), (120, 40), (200, 60)] {
        let t = text(&app, w, h);
        assert!(t.contains("INCOMPLETE"), "{w}x{h}:\n{t}");
        assert!(t.contains("3 failures"), "{w}x{h}:\n{t}");
        assert!(!t.contains("BACKUP COMPLETE"), "{w}x{h}:\n{t}");
        assert!(t.contains("octocat/alpha"), "{w}x{h}:\n{t}");
        assert!(t.contains("clone"), "{w}x{h}:\n{t}");
        assert!(t.contains("git clone failed"), "{w}x{h}:\n{t}");
        assert!(t.contains("gists"), "{w}x{h}:\n{t}");
        assert!(t.contains("manifest"), "{w}x{h}:\n{t}");
    }
}

#[test]
fn results_never_say_complete_when_the_engine_recorded_a_failure_at_any_size() {
    let app = app_with_results(done_event(vec![failure("o/r", "wiki", "x")], 0, false));
    for &(w, h) in SIZES {
        if w < 30 || h < 8 {
            continue;
        }
        let t = text(&app, w, h);
        assert!(
            !t.contains("COMPLETE") || t.contains("INCOMPLETE"),
            "{w}x{h} shows a success verdict:\n{t}"
        );
    }
}

#[test]
fn results_with_errored_repos_but_no_recorded_failure_is_still_incomplete() {
    let app = app_with_results(done_event(vec![], 3, false));
    assert_eq!(app.results.outcome, Outcome::Incomplete);
    let t = text(&app, 80, 24);
    assert!(t.contains("INCOMPLETE"), "{t}");
    assert!(t.contains("3 repositories failed"), "{t}");
}

#[test]
fn results_clean_run_says_complete() {
    let app = app_with_results(done_event(vec![], 0, false));
    assert_eq!(app.results.outcome, Outcome::Complete);
    let t = text(&app, 80, 24);
    assert!(t.contains("BACKUP COMPLETE"), "{t}");
    assert!(!t.contains("INCOMPLETE"), "{t}");
    assert!(!t.contains("Failures"), "{t}");
}

#[test]
fn results_dry_run_is_labelled() {
    let app = app_with_results(done_event(vec![], 0, true));
    let t = text(&app, 80, 24);
    assert!(t.contains("DRY RUN COMPLETE"), "{t}");
    assert!(t.contains("nothing was written"), "{t}");
}

#[test]
fn results_cancelled_is_not_shown_as_failed_or_complete() {
    let mut app = ready_app();
    app.run.status = RunStatus::Cancelling;
    handle_backup_event(&mut app, BackupEvent::BackupCancelled);
    assert_eq!(app.results.outcome, Outcome::Cancelled);
    assert_eq!(app.screen, Screen::Results);
    assert_eq!(app.run.status, RunStatus::Idle);
    let t = text(&app, 80, 24);
    assert!(t.contains("BACKUP CANCELLED"), "{t}");
    assert!(!t.contains("FAILED"), "{t}");
    assert!(!t.contains("BACKUP COMPLETE"), "{t}");
}

#[test]
fn results_fatal_error_shows_the_error() {
    let mut app = ready_app();
    handle_backup_event(
        &mut app,
        BackupEvent::BackupFailed {
            error: "GitHub client init failed: bad url".into(),
        },
    );
    let t = text(&app, 80, 24);
    assert!(t.contains("BACKUP FAILED"), "{t}");
    assert!(t.contains("bad url"), "{t}");
}

#[test]
fn results_before_any_run_is_not_a_red_failure() {
    let mut app = ready_app();
    press(&mut app, KeyCode::Char('5'));
    assert_eq!(app.screen, Screen::Results);
    let t = text(&app, 80, 24);
    assert!(t.contains("No backup has finished"), "{t}");
    assert!(!t.contains("FAILED"), "{t}");
}

#[test]
fn results_failure_list_scrolls_and_selection_is_visible() {
    let failures: Vec<Failure> = (0..40)
        .map(|i| {
            failure(
                &format!("octocat/repo-{i:02}"),
                "clone",
                &format!("error {i:02}"),
            )
        })
        .collect();
    let mut app = app_with_results(done_event(failures, 40, false));
    let first = text(&app, 80, 24);
    assert!(first.contains("repo-00"));
    assert!(!first.contains("repo-39"));
    press(&mut app, KeyCode::Char('G'));
    assert_eq!(app.results.failure_selected, 39);
    let last = text(&app, 80, 24);
    assert!(last.contains("repo-39"), "{last}");
    assert!(last.contains("40/40"), "{last}");
    press(&mut app, KeyCode::Char('g'));
    assert_eq!(app.results.failure_selected, 0);
    for _ in 0..100 {
        press(&mut app, KeyCode::Char('j'));
    }
    assert_eq!(app.results.failure_selected, 39, "clamped");
    press(&mut app, KeyCode::Char('k'));
    assert_eq!(app.results.failure_selected, 38);
}

#[test]
fn results_failure_message_with_newlines_stays_on_one_row() {
    let app = app_with_results(done_event(
        vec![failure("o/r", "clone", "first line\nsecond line\nthird")],
        1,
        false,
    ));
    let rows = dump(&app, 100, 24);
    let row = rows
        .iter()
        .find(|r| r.contains("first line"))
        .expect("failure row");
    assert!(row.contains("second line"), "{row}");
}

// ── Running screen ────────────────────────────────────────────────────────────

#[test]
fn running_screen_flags_incomplete_as_soon_as_a_repo_fails() {
    let mut app = ready_app();
    press(&mut app, KeyCode::Char('r'));
    assert_eq!(app.screen, Screen::Running);
    handle_backup_event(&mut app, BackupEvent::ReposDiscovered { total: 4 });
    handle_backup_event(
        &mut app,
        BackupEvent::RepoStarted {
            name: "octocat/ok".into(),
        },
    );
    handle_backup_event(
        &mut app,
        BackupEvent::RepoStarted {
            name: "octocat/bad".into(),
        },
    );
    handle_backup_event(
        &mut app,
        BackupEvent::RepoCompleted {
            name: "octocat/bad".into(),
            success: false,
            error: Some("clone failed".into()),
        },
    );
    for (w, h) in [(40u16, 12u16), (60, 16), (80, 24), (120, 40)] {
        let t = text(&app, w, h);
        assert!(t.contains("INCOMPLETE"), "{w}x{h}:\n{t}");
        assert!(t.contains("Failed: 1"), "{w}x{h}:\n{t}");
    }
    // Failed repositories are pinned to the top of the list, so the marker is
    // visible even when only a couple of rows fit.
    for (w, h) in [(60u16, 16u16), (80, 24), (120, 40)] {
        let t = text(&app, w, h);
        assert!(
            t.contains("!!"),
            "{w}x{h}: failed repo must be marked:\n{t}"
        );
    }
    let t = text(&app, 80, 24);
    assert!(t.contains("INCOMPLETE: 1 failed"), "{t}");
    assert!(t.contains("Failed: 1"), "{t}");
}

#[test]
fn running_screen_with_no_failures_has_no_incomplete_marker() {
    let mut app = ready_app();
    press(&mut app, KeyCode::Char('r'));
    handle_backup_event(&mut app, BackupEvent::ReposDiscovered { total: 4 });
    let t = text(&app, 80, 24);
    assert!(!t.contains("INCOMPLETE"), "{t}");
    assert!(t.contains("Failed: 0"), "{t}");
}

#[test]
fn running_log_always_shows_the_newest_entries() {
    let mut app = ready_app();
    press(&mut app, KeyCode::Char('r'));
    for i in 0..100 {
        handle_backup_event(
            &mut app,
            BackupEvent::LogLine {
                timestamp: "12:00:00".into(),
                level: "INFO".into(),
                message: format!("entry number {i:03} {}", "padding ".repeat(30)),
            },
        );
    }
    for (w, h) in [(60u16, 16u16), (80, 24), (120, 40), (200, 60)] {
        let t = text(&app, w, h);
        assert!(
            t.contains("entry number 099"),
            "{w}x{h}: newest hidden:\n{t}"
        );
    }
}

#[test]
fn running_log_scroll_back_and_follow() {
    let mut app = ready_app();
    press(&mut app, KeyCode::Char('r'));
    for i in 0..100 {
        handle_backup_event(
            &mut app,
            BackupEvent::LogLine {
                timestamp: "12:00:00".into(),
                level: "INFO".into(),
                message: format!("entry {i:03}"),
            },
        );
    }
    press(&mut app, KeyCode::Char('g')); // top
    assert!(text(&app, 80, 24).contains("entry 000"));
    // New lines arriving while scrolled back must not move the view.
    let before = text(&app, 80, 24);
    handle_backup_event(
        &mut app,
        BackupEvent::LogLine {
            timestamp: "12:00:01".into(),
            level: "INFO".into(),
            message: "late arrival".into(),
        },
    );
    assert_eq!(before, text(&app, 80, 24));
    press(&mut app, KeyCode::Char('G')); // follow again
    assert!(text(&app, 80, 24).contains("late arrival"));
}

#[test]
fn running_idle_screen_explains_and_has_a_way_out() {
    let mut app = ready_app();
    press(&mut app, KeyCode::Char('3'));
    assert_eq!(app.screen, Screen::Running);
    let t = text(&app, 80, 24);
    assert!(t.contains("No backup is running"), "{t}");
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Dashboard);
    press(&mut app, KeyCode::Char('3'));
    press(&mut app, KeyCode::Char('2')); // number keys work again
    assert_eq!(app.screen, Screen::Configure);
}

#[test]
fn from_every_screen_without_a_backup_the_user_can_quit() {
    for screen in [
        Screen::Dashboard,
        Screen::Configure,
        Screen::Running,
        Screen::Results,
        Screen::Verify,
    ] {
        let mut app = ready_app();
        app.screen = screen.clone();
        ctrl(&mut app, 'c');
        assert!(app.should_quit, "Ctrl+C must quit from {screen:?}");
    }
}

// ── Backup life cycle ─────────────────────────────────────────────────────────

#[test]
fn ctrl_c_cancels_once_and_never_quits_mid_cancel() {
    let mut app = ready_app();
    press(&mut app, KeyCode::Char('r'));
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    app.cancel_tx = Some(tx);
    app.start_backup_requested = false;

    ctrl(&mut app, 'c');
    assert_eq!(app.run.status, RunStatus::Cancelling);
    assert!(rx.blocking_recv().is_ok());
    assert!(text(&app, 80, 24).contains("Cancelling"));

    ctrl(&mut app, 'c'); // impatient second press
    assert!(!app.should_quit, "must wait for the task to stop");
    // Number keys stay disabled until the task has finished.
    press(&mut app, KeyCode::Char('1'));
    assert_eq!(app.screen, Screen::Running);

    handle_backup_event(&mut app, BackupEvent::BackupCancelled);
    assert_eq!(app.screen, Screen::Results);
    assert_eq!(app.run.status, RunStatus::Idle);
    assert!(!app.run.is_active());
    assert!(app.cancel_tx.is_none());
}

#[test]
fn a_backup_cannot_be_started_twice() {
    let mut app = ready_app();
    press(&mut app, KeyCode::Char('r'));
    app.start_backup_requested = false;
    app.screen = Screen::Results;
    press(&mut app, KeyCode::Char('r')); // would start a second run
    assert!(!app.start_backup_requested);
}

#[test]
fn backup_done_unlocks_navigation_and_freezes_the_clock() {
    let mut app = ready_app();
    press(&mut app, KeyCode::Char('r'));
    assert!(app.run.is_active());
    press(&mut app, KeyCode::Char('2')); // blocked while running
    assert_eq!(app.screen, Screen::Running);

    handle_backup_event(&mut app, done_event(vec![], 0, false));
    assert_eq!(app.screen, Screen::Results);
    assert!(app.run.elapsed_final.is_some());
    let frozen = app.run.elapsed_str();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert_eq!(
        frozen,
        app.run.elapsed_str(),
        "Elapsed must stop at the end"
    );
    press(&mut app, KeyCode::Char('2'));
    assert_eq!(app.screen, Screen::Configure);
}

#[test]
fn repos_that_never_reported_back_end_up_skipped() {
    let mut app = ready_app();
    press(&mut app, KeyCode::Char('r'));
    handle_backup_event(&mut app, BackupEvent::RepoStarted { name: "o/x".into() });
    handle_backup_event(&mut app, done_event(vec![], 0, true));
    assert_eq!(app.run.repos[0].status, RepoStatus::Skipped);
}

#[test]
fn progress_reaches_100_percent_when_repos_are_skipped() {
    let mut app = ready_app();
    press(&mut app, KeyCode::Char('r'));
    handle_backup_event(&mut app, BackupEvent::ReposDiscovered { total: 3 });
    for i in 1..=3 {
        handle_backup_event(
            &mut app,
            BackupEvent::Progress {
                current: i,
                total: 3,
            },
        );
    }
    assert_eq!(app.run.progress_pct(), 100);
}

#[test]
fn shutdown_signal_cancels_the_backup_then_quits() {
    let mut app = ready_app();
    press(&mut app, KeyCode::Char('r'));
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    app.cancel_tx = Some(tx);
    app.start_backup_requested = false;

    handle_backup_event(&mut app, BackupEvent::Shutdown { code: 143 });
    assert!(app.shutdown_requested);
    assert_eq!(app.shutdown_code, 143);
    assert!(rx.blocking_recv().is_ok(), "the backup was told to cancel");
    assert!(!app.should_quit, "wait for git to stop first");

    handle_backup_event(&mut app, BackupEvent::Shutdown { code: 143 });
    assert!(app.should_quit, "a second signal forces the exit");
}

#[test]
fn shutdown_signal_when_idle_quits_at_once() {
    let mut app = ready_app();
    handle_backup_event(&mut app, BackupEvent::Shutdown { code: 130 });
    assert!(app.should_quit);
}

// ── Input handling ────────────────────────────────────────────────────────────

#[test]
fn paste_never_triggers_shortcuts() {
    let mut app = ready_app();
    app.screen = Screen::Configure;
    handle_paste(&mut app, "s\n2\nr");
    assert!(!app.start_backup_requested);
    assert_eq!(app.screen, Screen::Configure);
    assert!(!app.run.is_active());
}

#[test]
fn paste_while_editing_inserts_one_clean_line() {
    let mut app = App::new(InitialConfig::default());
    app.screen = Screen::Configure;
    press(&mut app, KeyCode::Enter); // edit the token
    handle_paste(&mut app, "tok_123\nsome more\ttext\r\n");
    assert_eq!(app.config.edit_buffer, "tok_123some moretext");
    assert!(!app.start_backup_requested);
}

#[test]
fn control_chords_are_not_inserted_into_fields() {
    let mut app = App::new(InitialConfig::default());
    app.screen = Screen::Configure;
    press(&mut app, KeyCode::Enter);
    handle_key_dispatch(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    handle_key_dispatch(&mut app, KeyCode::Char('w'), KeyModifiers::ALT);
    assert_eq!(app.config.edit_buffer, "");
}

#[test]
fn concurrency_field_only_takes_digits() {
    let mut app = ready_app();
    app.screen = Screen::Configure;
    app.config.active_tab = 3;
    app.config.active_field = 6;
    press(&mut app, KeyCode::Enter);
    app.config.edit_buffer.clear();
    for c in "1a2b".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    assert_eq!(app.config.edit_buffer, "12");
}

#[test]
fn edit_buffer_is_length_capped() {
    let mut app = App::new(InitialConfig::default());
    app.screen = Screen::Configure;
    press(&mut app, KeyCode::Enter);
    handle_paste(&mut app, &"x".repeat(5000));
    assert_eq!(app.config.edit_buffer.chars().count(), 1024);
}

#[test]
fn secret_is_never_echoed_while_typing() {
    let mut app = App::new(InitialConfig::default());
    app.screen = Screen::Configure;
    press(&mut app, KeyCode::Enter);
    for c in "ghp_SECRETVALUE".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    let t = text(&app, 80, 24);
    assert!(!t.contains("SECRETVALUE"), "{t}");
    assert!(t.contains("***************_"), "{t}");
}

#[test]
fn long_value_keeps_its_end_visible_at_narrow_width() {
    let mut app = ready_app();
    app.screen = Screen::Configure;
    app.config.active_tab = 1;
    app.config.active_field = 0;
    press(&mut app, KeyCode::Enter);
    app.config.edit_buffer = format!("{}-THE-END", "a".repeat(200));
    for (w, h) in [(40u16, 12u16), (60, 16), (80, 24)] {
        let t = text(&app, w, h);
        assert!(t.contains("THE-END_"), "{w}x{h}: cursor end hidden:\n{t}");
    }
}

// ── Configure: no inert fields ────────────────────────────────────────────────

#[test]
fn full_toggle_reaches_backup_options() {
    let mut app = ready_app();
    app.screen = Screen::Configure;
    app.config.active_tab = 1;
    app.config.active_field = 4;
    assert!(!app.config.to_backup_config().2.full);
    press(&mut app, KeyCode::Char(' '));
    assert!(app.config.full);
    assert!(app.config.to_backup_config().2.full);
    let t = text(&app, 80, 24);
    assert!(t.contains("Full backup"), "{t}");
}

#[test]
fn every_configure_field_can_be_reached_and_changed() {
    // Every field of every tab must do something when activated: toggles flip,
    // text fields enter edit mode, the select cycles.  A field that reacts to
    // nothing is an inert control.
    for tab in 0..crate::state::ConfigState::TAB_COUNT {
        let probe = {
            let mut a = ready_app();
            a.config.active_tab = tab;
            a
        };
        let n = probe.config.tab_field_count();
        for field in 0..n {
            let mut app = ready_app();
            app.screen = Screen::Configure;
            app.config.active_tab = tab;
            app.config.active_field = field;
            let before = (
                format!("{:?}", app.config.to_backup_config().2),
                app.config.clone_type.clone(),
                app.config.manifest,
            );
            press(&mut app, KeyCode::Enter);
            if app.config.editing {
                continue; // text field: editing started
            }
            press(&mut app, KeyCode::Right); // select fields
            let after = (
                format!("{:?}", app.config.to_backup_config().2),
                app.config.clone_type.clone(),
                app.config.manifest,
            );
            assert_ne!(before, after, "tab {tab} field {field} did nothing");
        }
    }
}

#[test]
fn removed_inert_options_are_not_shown() {
    let mut app = ready_app();
    app.screen = Screen::Configure;
    for tab in 0..crate::state::ConfigState::TAB_COUNT {
        app.config.active_tab = tab;
        let t = text(&app, 120, 40);
        for gone in [
            "Mirror To",
            "Bucket",
            "Keep Last",
            "Max Age",
            "Device Auth",
            "OAuth",
        ] {
            assert!(!t.contains(gone), "tab {tab} still shows {gone}:\n{t}");
        }
    }
    app.config.active_tab = 5;
    let t = text(&app, 120, 40);
    assert!(t.contains("Command line only"), "{t}");
}

#[test]
fn debug_output_never_contains_the_token() {
    let app = ready_app();
    let s = format!("{:?}", app.config);
    assert!(!s.contains("dummy-token-for-tests"), "{s}");
}

#[test]
fn validate_rejects_bad_values_instead_of_silently_fixing_them() {
    let mut app = ready_app();
    assert!(app.config.validate().is_none());

    app.config.concurrency = "abc".into();
    assert!(app.config.validate().unwrap().contains("Concurrency"));
    app.config.concurrency = "0".into();
    assert!(app.config.validate().is_some());
    app.config.concurrency = "9999999999999999999".into();
    assert!(app.config.validate().is_some(), "must not reach the engine");
    app.config.concurrency = "64".into();
    assert!(app.config.validate().is_none());

    app.config.since = "yesterday".into();
    assert!(app.config.validate().unwrap().contains("Since"));
    app.config.since = "2024-01-01".into();
    assert!(app.config.validate().is_none());
    assert_eq!(
        app.config.to_backup_config().2.since.as_deref(),
        Some("2024-01-01T00:00:00Z")
    );
    app.config.since = "2024-01-01T10:00:00+02:00".into();
    assert_eq!(
        app.config.to_backup_config().2.since.as_deref(),
        Some("2024-01-01T08:00:00Z")
    );
    app.config.full = true;
    assert!(app.config.validate().unwrap().contains("Full backup"));
    app.config.full = false;
    app.config.since.clear();

    app.config.api_url = "http://insecure.example".into();
    assert!(app.config.validate().unwrap().contains("https://"));
    app.config.api_url = "https://ghe.example/api/v3".into();
    assert!(app.config.validate().is_none());

    for bad in ["../x", "a/b", "with space", "a\\b"] {
        app.config.owner = bad.into();
        assert!(app.config.validate().is_some(), "owner {bad:?}");
    }
}

// ── Dashboard: last run ───────────────────────────────────────────────────────

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("gbk-tui-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[test]
fn dashboard_shows_last_run_from_history_including_failures() {
    use github_backup_types::backup_state::{BackupRunEntry, BackupRunHistory};
    use github_backup_types::config::OutputConfig;

    let dir = temp_dir("dash");
    let out = OutputConfig::new(&dir);
    let mut history = BackupRunHistory::default();
    history.push(
        BackupRunEntry {
            timestamp: "2026-10-03T10:00:00Z".into(),
            repos_backed_up: 12,
            elapsed_secs: 5.0,
            success: false,
            failures: 2,
            tool_version: "9.9.9".into(),
        },
        20,
    );
    history.save(&out.backup_history_path("octocat")).unwrap();

    // The owner and output directory are typed in AFTER the app started.
    let mut app = App::new(InitialConfig::default());
    assert!(app.dashboard.last_backup_time.is_none());
    app.config.owner = "octocat".into();
    app.config.output_dir = dir.display().to_string();
    app.screen = Screen::Configure;
    press(&mut app, KeyCode::Esc); // back to the dashboard
    assert_eq!(app.screen, Screen::Dashboard);
    assert_eq!(app.dashboard.last_backup_repos, Some(12));
    assert_eq!(app.dashboard.last_run_ok, Some(false));
    assert_eq!(app.dashboard.last_run_failures, 2);

    for (w, h) in [(80u16, 24u16), (120, 40), (60, 16), (40, 12)] {
        let t = text(&app, w, h);
        assert!(t.contains("2026-10-03"), "{w}x{h}:\n{t}");
        assert!(t.contains("INCOMPLETE"), "{w}x{h}:\n{t}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dashboard_without_history_falls_back_to_the_state_file() {
    use github_backup_types::backup_state::BackupState;
    use github_backup_types::config::OutputConfig;

    let dir = temp_dir("dash-state");
    let out = OutputConfig::new(&dir);
    let state = BackupState {
        last_successful_run: Some("2026-01-02T03:04:05Z".into()),
        tool_version: "1.2.3".into(),
        repos_backed_up: 4,
        ..Default::default()
    };
    state.save(&out.backup_state_path("octocat")).unwrap();

    let app = App::new(InitialConfig {
        owner: Some("octocat".into()),
        output: Some(dir.display().to_string()),
        ..Default::default()
    });
    assert_eq!(app.dashboard.last_backup_repos, Some(4));
    assert_eq!(app.dashboard.last_run_ok, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dashboard_actions_and_hints_stay_visible_at_small_sizes() {
    let app = ready_app();
    for (w, h) in [(40u16, 12u16), (60, 16), (30, 8), (80, 24)] {
        let t = text(&app, w, h);
        assert!(t.contains("Run Backup"), "{w}x{h}:\n{t}");
        assert!(t.contains("Quit"), "{w}x{h}:\n{t}");
        assert!(t.contains("quit"), "{w}x{h}: hint line missing:\n{t}");
    }
}

#[test]
fn title_bar_fits_and_marks_the_active_screen() {
    let mut app = ready_app();
    app.screen = Screen::Configure;
    for (w, expect) in [
        (80u16, "[2]Configure"),
        (60, "[2]Configure"),
        (40, "[2 Configure]"),
    ] {
        let rows = dump(&app, w, 24);
        assert!(rows[0].contains(expect), "{w}: {:?}", rows[0]);
        assert!(rows[0].trim_end().chars().count() <= w as usize);
    }
}

#[test]
fn tab_bar_shows_every_tab_from_56_columns_up() {
    let mut app = ready_app();
    app.screen = Screen::Configure;
    for (w, h) in [(56u16, 16u16), (60, 16), (80, 24), (120, 40)] {
        let t = text(&app, w, h);
        for name in crate::state::ConfigState::TAB_NAMES {
            assert!(t.contains(name), "{w}x{h}: tab {name} missing:\n{t}");
        }
    }
    // Narrower than that the bar collapses to the active tab and its position.
    let t = text(&app, 40, 12);
    assert!(t.contains("Auth  1/6"), "{t}");
}

#[test]
fn dashboard_columns_do_not_run_into_each_other() {
    let mut app = ready_app();
    app.config.output_dir = "/a/very/long/output/directory/that/does/not/fit/anywhere".into();
    for (w, h) in [(60u16, 16u16), (80, 24), (120, 40)] {
        let rows = dump(&app, w, h);
        let row = rows
            .iter()
            .find(|r| r.contains("Output:"))
            .expect("output row");
        assert!(
            row.contains(" Repos:"),
            "{w}x{h}: no gap before right column: {row:?}"
        );
    }
}

#[test]
fn narrow_failure_list_folds_the_step_into_the_message() {
    let app = app_with_results(done_event(
        vec![failure("octocat/bad1", "clone", "git clone failed")],
        1,
        false,
    ));
    let t = text(&app, 40, 14);
    assert!(t.contains("octocat/bad1"), "{t}");
    assert!(t.contains("clone: git"), "{t}");
}

#[test]
fn configure_key_hints_match_the_keys() {
    let mut app = ready_app();
    app.screen = Screen::Configure;
    let t = text(&app, 120, 40);
    for hint in [
        "s/F5", "start", "Esc", "back", "Tab", "Space", "toggle", "Enter", "edit",
    ] {
        assert!(t.contains(hint), "missing {hint}:\n{t}");
    }
    // The categories hint appears only where `A` works.
    assert!(!t.contains("all/none"));
    app.config.active_tab = 2;
    assert!(text(&app, 120, 40).contains("all/none"));
    // F5 and s really start a backup.
    app.config.active_tab = 0;
    press(&mut app, KeyCode::F(5));
    assert!(app.start_backup_requested);
}

// ── Task-level behaviour ──────────────────────────────────────────────────────

#[test]
fn append_history_records_the_failure_count() {
    use github_backup_types::backup_state::BackupRunHistory;
    use github_backup_types::config::OutputConfig;

    let dir = temp_dir("hist");
    let out = OutputConfig::new(&dir);
    let stats = BackupStats::new();
    stats.record_failure("o/r", "clone", "boom");
    stats.record_failure("gists", "list", "bad");
    crate::append_history(&out, "octocat", &stats, "2026-10-03T10:00:00Z");

    let history = BackupRunHistory::load(&out.backup_history_path("octocat")).unwrap();
    assert_eq!(history.entries.len(), 1);
    assert!(!history.entries[0].success);
    assert_eq!(history.entries[0].failures, 2);
    assert_eq!(history.entries[0].timestamp, "2026-10-03T10:00:00Z");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn finish_run_records_a_manifest_failure_as_a_run_failure() {
    use github_backup_types::config::OutputConfig;

    let dir = temp_dir("manifest");
    // The owner's json dir is a FILE, so the manifest cannot be written.
    let out = OutputConfig::new(&dir);
    std::fs::create_dir_all(dir.join("octocat")).unwrap();
    std::fs::write(dir.join("octocat").join("json"), b"not a dir").unwrap();

    let stats = BackupStats::new();
    crate::finish_run(&stats, &out, "octocat", chrono::Utc::now(), true).await;
    assert!(
        stats.has_failures(),
        "a manifest that was asked for but not written is a failure"
    );
    let f = stats.failures();
    assert_eq!(f[0].step, "manifest");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn backup_task_unreachable_api_ends_with_a_terminal_event() {
    let dir = temp_dir("task-fail");
    let cfg = crate::state::ConfigState {
        owner: "octocat".into(),
        token: "dummy-token".into(),
        output_dir: dir.display().to_string(),
        api_url: "https://127.0.0.1:1".into(),
        ..Default::default()
    };
    let (tx, mut rx) = crate::progress::channel();
    let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        crate::run_backup_task(cfg, tx, cancel_rx),
    )
    .await
    .expect("the task must finish");

    let mut terminal = None;
    while let Ok(ev) = rx.try_recv() {
        match ev {
            BackupEvent::BackupFailed { .. } | BackupEvent::BackupDone { .. } => {
                terminal = Some(ev);
            }
            _ => {}
        }
    }
    assert!(terminal.is_some(), "task ended without Done/Failed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn backup_task_cancel_ends_with_cancelled_and_does_not_hang() {
    let dir = temp_dir("task-cancel");
    // A listener that accepts and never answers: the run would hang forever
    // without a working cancel.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let cfg = crate::state::ConfigState {
        owner: "octocat".into(),
        token: "dummy-token".into(),
        output_dir: dir.display().to_string(),
        api_url: format!(
            "https://127.0.0.1:{}",
            listener.local_addr().unwrap().port()
        ),
        ..Default::default()
    };
    let (tx, mut rx) = crate::progress::channel();
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

    let task = tokio::spawn(crate::run_backup_task(cfg, tx, cancel_rx));
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    cancel_tx.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(20), task)
        .await
        .expect("cancel must stop the task promptly")
        .unwrap();

    let mut got_cancelled = false;
    while let Ok(ev) = rx.try_recv() {
        got_cancelled |= matches!(ev, BackupEvent::BackupCancelled);
    }
    assert!(got_cancelled);
    drop(listener);
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Tracing layer ─────────────────────────────────────────────────────────────

#[test]
fn layer_reports_progress_but_never_guesses_success_from_log_text() {
    use tracing_subscriber::layer::SubscriberExt;

    let (tx, mut rx) = crate::progress::channel();
    let subscriber =
        tracing_subscriber::registry().with(crate::tracing_layer::TuiTracingLayer::new(tx));
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(
            repo = "octocat/a",
            progress = "3/10",
            "repository processed"
        );
        tracing::error!(
            repo = "octocat/a",
            error = "boom",
            "repository backup incomplete, continuing"
        );
    });

    let mut progress = vec![];
    let mut repo_events = 0;
    let mut logs = 0;
    while let Ok(ev) = rx.try_recv() {
        match ev {
            BackupEvent::Progress { current, total } => progress.push((current, total)),
            BackupEvent::RepoCompleted { .. } | BackupEvent::RepoStarted { .. } => {
                repo_events += 1;
            }
            BackupEvent::LogLine { .. } => logs += 1,
            _ => {}
        }
    }
    assert_eq!(progress, vec![(3, 10)]);
    assert_eq!(
        repo_events, 0,
        "repo outcome comes from the engine, not from log text"
    );
    assert_eq!(logs, 2);
}

#[test]
fn log_window_never_hides_the_newest_when_following() {
    use crate::screens::running::log_window;
    assert_eq!(log_window(100, 10, 0), 90..100);
    assert_eq!(log_window(5, 10, 0), 0..5);
    assert_eq!(log_window(100, 10, 30), 60..70);
    assert_eq!(log_window(100, 10, 10_000), 0..10);
    assert_eq!(log_window(0, 10, 0), 0..0);
}

#[test]
fn push_log_keeps_view_still_when_scrolled_back() {
    let mut run = crate::state::RunState::default();
    for i in 0..10 {
        run.push_log(LogLine {
            timestamp: String::new(),
            level: "INFO".into(),
            message: format!("{i}"),
        });
    }
    run.log_back = 3;
    run.push_log(LogLine {
        timestamp: String::new(),
        level: "INFO".into(),
        message: "new".into(),
    });
    assert_eq!(run.log_back, 4);
}

// ── Terminal hang-up ──────────────────────────────────────────────────────────

#[test]
fn a_reader_stuck_in_read_cannot_hang_shutdown() {
    // Stands in for crossterm's `read` spinning forever on a hung-up tty.
    let stuck = std::thread::spawn(|| std::thread::sleep(std::time::Duration::from_secs(30)));
    let t0 = std::time::Instant::now();
    let joined = crate::join_with_timeout(stuck, std::time::Duration::from_millis(200));
    assert!(!joined);
    assert!(
        t0.elapsed() < std::time::Duration::from_secs(2),
        "must give up promptly"
    );

    let quick = std::thread::spawn(|| {});
    assert!(crate::join_with_timeout(
        quick,
        std::time::Duration::from_secs(2)
    ));
}

// ── Pre-fill from the command line ────────────────────────────────────────────

#[test]
fn form_round_trips_through_backup_options() {
    use github_backup_types::config::{BackupOptions, BackupTarget, CloneType};

    let opts = BackupOptions {
        target: BackupTarget::Org,
        full: true,
        forks: true,
        private: true,
        lfs: true,
        prefer_ssh: true,
        no_prune: true,
        clone_type: CloneType::Bare,
        include_repos: vec!["a-*".into(), "b".into()],
        exclude_repos: vec!["c".into()],
        since: Some("2024-01-01T00:00:00Z".into()),
        dry_run: true,
        concurrency: 9,
        ..BackupOptions::all()
    };
    let cfg = crate::state::ConfigState::from_backup_options(&opts);
    let (_, _, back, _) = cfg.to_backup_config();
    assert_eq!(format!("{opts:?}"), format!("{back:?}"));
    assert!(cfg.validate().is_some(), "owner/token still missing");
}

#[test]
fn default_options_round_trip_too() {
    use github_backup_types::config::BackupOptions;
    let opts = BackupOptions {
        repositories: true,
        concurrency: 4,
        ..BackupOptions::default()
    };
    let (_, _, back, _) = crate::state::ConfigState::from_backup_options(&opts).to_backup_config();
    assert_eq!(format!("{opts:?}"), format!("{back:?}"));
}

#[test]
fn initial_config_options_prefill_the_configure_screen() {
    use github_backup_types::config::BackupOptions;
    let app = App::new(InitialConfig {
        owner: Some("octocat".into()),
        token: Some("dummy".into()),
        options: Some(BackupOptions {
            private: true,
            dry_run: true,
            issues: true,
            concurrency: 2,
            ..BackupOptions::default()
        }),
        manifest: true,
        ..Default::default()
    });
    assert!(app.config.private && app.config.dry_run && app.config.issues);
    assert!(app.config.manifest);
    assert_eq!(app.config.concurrency, "2");
    assert_eq!(app.config.owner, "octocat");
    assert!(app.config.validate().is_none());
}

#[test]
fn results_show_discussions() {
    let app = app_with_results(done_event(vec![], 0, false));
    let t = text(&app, 120, 40);
    assert!(t.contains("Discussions fetched"), "{t}");
}
