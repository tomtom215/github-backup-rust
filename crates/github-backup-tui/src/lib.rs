// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! `github-backup-tui` — Ratatui TUI front-end for the backup engine.
//!
//! # Entry point
//!
//! Call [`run_tui`] from `main.rs` when `--tui` is passed.  Terminal setup,
//! backup task spawning, and progress routing are all handled here.

use std::io::stdout;
use std::process::ExitCode;
use std::time::Duration;

use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    },
    layout::{Constraint, Direction, Layout},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame, Terminal,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use github_backup_client::GitHubClient;
use github_backup_core::{BackupEngine, FsStorage, ProcessGitRunner};
use github_backup_types::config::{Credential, OutputConfig};

mod app;
mod event;
mod progress;
mod screens;
mod state;
mod theme;
mod tracing_layer;

pub use app::InitialConfig;

#[cfg(test)]
mod tests;

// ── Public entry point ────────────────────────────────────────────────────────

/// Launches the full-screen TUI.
///
/// Owns the terminal for its lifetime and always restores it before returning.
pub async fn run_tui(initial: InitialConfig) -> ExitCode {
    if let Err(e) = enable_raw_mode() {
        eprintln!("failed to enable raw mode: {e}");
        return ExitCode::FAILURE;
    }
    let mut out = stdout();
    if let Err(e) = execute!(out, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        eprintln!("failed to enter alternate screen: {e}");
        return ExitCode::FAILURE;
    }

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(e) => {
            let _ = disable_raw_mode();
            let _ = execute!(stdout(), LeaveAlternateScreen);
            eprintln!("failed to create terminal: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Progress channel shared by the backup task, verify task, and the
    // tracing subscriber layer.
    let (progress_tx, mut progress_rx) = progress::channel();

    // Install the TUI tracing layer so log output goes to the log panel.
    let tui_layer = tracing_layer::TuiTracingLayer::new(progress_tx.clone());
    let _ = tracing_subscriber::registry().with(tui_layer).try_init();

    let mut app = app::App::new(initial);

    let result = event_loop(&mut terminal, &mut app, &mut progress_rx, progress_tx).await;

    // Restore terminal regardless of outcome.
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("TUI error: {e}");
            ExitCode::FAILURE
        }
    }
}

// ── Event loop ────────────────────────────────────────────────────────────────

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut app::App,
    progress_rx: &mut progress::ProgressRx,
    progress_tx: progress::ProgressTx,
) -> std::io::Result<()> {
    const TICK: Duration = Duration::from_millis(16);

    let mut backup_running = false;
    let mut verify_running = false;

    loop {
        // ── Drain progress channel ─────────────────────────────────────────
        while let Ok(ev) = progress_rx.try_recv() {
            app::handle_backup_event(app, ev);
        }

        // ── Spawn backup task if requested ────────────────────────────────
        if app.start_backup_requested && !backup_running {
            app.start_backup_requested = false;
            backup_running = true;

            let tx = progress_tx.clone();
            let cfg = app.config.clone();
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
            app.cancel_tx = Some(cancel_tx);

            tokio::spawn(async move {
                run_backup_task(cfg, tx, cancel_rx).await;
            });
        }

        // Reset flag when backup finishes (screen leaves Running).
        if backup_running && app.screen != state::Screen::Running {
            backup_running = false;
        }

        // ── Spawn verify task if requested ────────────────────────────────
        if app.start_verify_requested && !verify_running {
            app.start_verify_requested = false;
            verify_running = true;

            let vtx = progress_tx.clone();
            let owner = app.config.owner.clone();
            let output_dir = app.config.output_dir.clone();

            tokio::spawn(async move {
                run_verify_task(owner, output_dir, vtx).await;
            });
        }
        if !app.verify.running {
            verify_running = false;
        }

        // ── Render ─────────────────────────────────────────────────────────
        terminal.draw(|frame| render(frame, app))?;

        // ── Read terminal input ────────────────────────────────────────────
        if ratatui::crossterm::event::poll(TICK)? {
            use ratatui::crossterm::event::{Event, KeyEventKind};
            if let Event::Key(key) = ratatui::crossterm::event::read()? {
                if key.kind == KeyEventKind::Press {
                    app::handle_key_dispatch(app, key.code, key.modifiers);
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

// ── Frame rendering ───────────────────────────────────────────────────────────

fn render(frame: &mut Frame, app: &app::App) {
    let area = frame.area();

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    render_title_bar(frame, app, rows[0]);

    if app.modal_error.is_some() {
        render_screen_content(frame, app, rows[1]);
        render_error_modal(frame, app, rows[1]);
    } else {
        render_screen_content(frame, app, rows[1]);
    }
}

fn render_title_bar(frame: &mut Frame, app: &app::App, area: ratatui::layout::Rect) {
    let cur = &app.screen;
    let line = Line::from(vec![
        Span::styled(" github-backup ", theme::ACCENT_BOLD),
        Span::styled(concat!("v", env!("CARGO_PKG_VERSION"), "  "), theme::DIM),
        nav_tab("1", "Dashboard", *cur == state::Screen::Dashboard),
        nav_tab("2", "Configure", *cur == state::Screen::Configure),
        nav_tab("3", "Run", *cur == state::Screen::Running),
        nav_tab("4", "Verify", *cur == state::Screen::Verify),
        nav_tab("5", "Results", *cur == state::Screen::Results),
    ]);
    frame.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::NONE)),
        area,
    );
}

fn nav_tab(key: &'static str, label: &'static str, active: bool) -> Span<'static> {
    if active {
        Span::styled(
            format!("[{key}]{label}  "),
            theme::ACCENT_STYLE.add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(format!("[{key}]{label}  "), theme::DIM)
    }
}

fn render_screen_content(frame: &mut Frame, app: &app::App, area: ratatui::layout::Rect) {
    match app.screen {
        state::Screen::Dashboard => {
            screens::dashboard::render(frame, &app.dashboard, &app.config, area);
        }
        state::Screen::Configure => {
            screens::configure::render(frame, &app.config, area);
        }
        state::Screen::Running => {
            screens::running::render(frame, &app.run, area);
        }
        state::Screen::Results => {
            screens::results::render(frame, &app.results, area);
        }
        state::Screen::Verify => {
            screens::verify::render(frame, &app.verify, &app.config, area);
        }
    }
}

fn render_error_modal(frame: &mut Frame, app: &app::App, area: ratatui::layout::Rect) {
    let err = app.modal_error.as_deref().unwrap_or("");
    let h = 7u16;
    let w = (area.width * 2 / 3).clamp(40, 70);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = ratatui::layout::Rect::new(x, y, w, h);

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(Span::styled(
            " Error ",
            theme::ERR_STYLE.add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(theme::ERR_STYLE);

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let para = Paragraph::new(vec![
        Line::from(Span::styled(err, theme::NORMAL)),
        Line::from(""),
        Line::from(Span::styled("Press any key to dismiss", theme::DIM)),
    ])
    .wrap(Wrap { trim: true });
    frame.render_widget(para, inner);
}

// ── Backup task ───────────────────────────────────────────────────────────────

async fn run_backup_task(
    cfg: state::ConfigState,
    tx: progress::ProgressTx,
    cancel_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let (owner, output_path, opts, token_opt) = cfg.to_backup_config();

    let credential = match token_opt {
        Some(t) => Credential::Token(t),
        None => Credential::Anonymous,
    };

    let api_url = if cfg.api_url.trim().is_empty() {
        None
    } else {
        Some(cfg.api_url.trim().to_string())
    };

    let client_result = match api_url.as_deref() {
        Some(url) => GitHubClient::with_api_url(credential, url),
        None => GitHubClient::new(credential),
    };

    let client = match client_result {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(event::BackupEvent::BackupFailed {
                error: format!("GitHub client init failed: {e}"),
            });
            return;
        }
    };

    let output = OutputConfig::new(&output_path);
    let engine = BackupEngine::new(
        client,
        FsStorage::new(),
        ProcessGitRunner::new(),
        output,
        opts,
    );

    tokio::select! {
        result = engine.run(&owner) => {
            match result {
                Ok(stats) => {
                    // Persist backup state.
                    save_backup_state(&owner, &output_path, &stats);
                    let _ = tx.send(event::BackupEvent::BackupDone {
                        repos_backed_up:    stats.repos_backed_up(),
                        repos_discovered:   stats.repos_discovered(),
                        repos_skipped:      stats.repos_skipped(),
                        repos_errored:      stats.repos_errored(),
                        gists_backed_up:    stats.gists_backed_up(),
                        issues_fetched:     stats.issues_fetched(),
                        prs_fetched:        stats.prs_fetched(),
                        workflows_fetched:  stats.workflows_fetched(),
                        discussions_fetched: 0, // BackupStats does not expose discussions count
                        elapsed_secs:       stats.elapsed_secs(),
                    });
                }
                Err(e) => {
                    let _ = tx.send(event::BackupEvent::BackupFailed {
                        error: e.to_string(),
                    });
                }
            }
        }
        _ = cancel_rx => {
            let _ = tx.send(event::BackupEvent::BackupFailed {
                error: "Backup cancelled by user.".into(),
            });
        }
    }
}

fn save_backup_state(
    owner: &str,
    output_path: &std::path::Path,
    stats: &github_backup_core::BackupStats,
) {
    use github_backup_types::backup_state::BackupState;

    let output = OutputConfig::new(output_path);
    let state_path = output.backup_state_path(owner);
    let now = chrono::Utc::now();
    let s = BackupState {
        last_successful_run: now.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        repos_backed_up: stats.repos_backed_up(),
    };
    let _ = s.save(&state_path);
}

// ── Verify task ───────────────────────────────────────────────────────────────

async fn run_verify_task(owner: String, output_dir: String, tx: progress::ProgressTx) {
    let output = OutputConfig::new(&output_dir);
    let json_dir = output.owner_json_dir(&owner);

    let result =
        tokio::task::spawn_blocking(move || github_backup_core::verify_manifest(&json_dir)).await;

    match result {
        Ok(Ok(report)) => {
            let _ = tx.send(event::BackupEvent::VerifyDone {
                ok: report.ok,
                tampered: report.tampered,
                missing: report.missing,
                unexpected: report.unexpected,
            });
        }
        Ok(Err(e)) => {
            let _ = tx.send(event::BackupEvent::VerifyFailed {
                error: e.to_string(),
            });
        }
        Err(e) => {
            let _ = tx.send(event::BackupEvent::VerifyFailed {
                error: format!("verify task panicked: {e}"),
            });
        }
    }
}
