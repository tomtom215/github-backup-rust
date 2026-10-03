// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! `github-backup-tui` — Ratatui TUI front-end for the backup engine.
//!
//! # Entry point
//!
//! Call [`run_tui`] from `main.rs` when `--tui` is passed.  Terminal setup,
//! backup task spawning, and progress routing are all handled here.
//!
//! # Terminal safety
//!
//! Raw mode, the alternate screen and bracketed paste are owned by a
//! [`TerminalGuard`] whose `Drop` restores them, so every way out of
//! [`run_tui`] (normal quit, error, unwinding panic) leaves the shell usable.
//! A panic hook restores the terminal *before* the panic message is printed so
//! the message is readable, and SIGINT/SIGTERM/SIGHUP are turned into an
//! orderly cancel-then-quit instead of killing the process in raw mode.
//! (SIGKILL cannot be handled; nothing can restore the terminal after it.)

use std::io::{stdout, IsTerminal};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        cursor::Show,
        event::{DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    },
    layout::{Constraint, Direction, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame, Terminal,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};

use github_backup_client::GitHubClient;
use github_backup_core::{BackupEngine, EngineEvent, FsStorage, ProcessGitRunner};
use github_backup_types::config::{Credential, OutputConfig};

mod app;
mod event;
mod progress;
mod screens;
mod state;
mod theme;
mod tracing_layer;

use screens::util::fit;

pub use app::InitialConfig;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_ui;

/// Smallest terminal the screens are laid out for; below this a notice is
/// shown instead of a clipped, misleading screen.
const MIN_WIDTH: u16 = 30;
const MIN_HEIGHT: u16 = 8;

/// How long a signal-initiated shutdown waits for a running backup to stop.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);
/// After SIGHUP the terminal is gone and its reader may be spinning: wait less.
const HANGUP_GRACE: Duration = Duration::from_secs(3);

// ── Terminal ownership ────────────────────────────────────────────────────────

/// Puts the terminal into TUI mode and restores it on drop.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> std::io::Result<Self> {
        enable_raw_mode()?;
        // From here on Drop restores whatever part of the setup succeeded.
        let guard = Self;
        execute!(stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Undoes [`TerminalGuard::enter`].  Idempotent and infallible: every step is
/// attempted whatever the others did.
fn restore_terminal() {
    let _ = execute!(stdout(), DisableBracketedPaste, LeaveAlternateScreen, Show);
    let _ = disable_raw_mode();
}

/// Installs a panic hook that restores the terminal before the message prints.
///
/// Only a panic on the TUI thread tears the screen down.  A panic in a
/// background task is swallowed here (it is reported inside the UI by the task
/// supervisor); printing it would scribble over the alternate screen.
fn install_panic_hook() {
    let tui_thread = std::thread::current().id();
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().id() == tui_thread {
            restore_terminal();
            default_hook(info);
        } else {
            tracing::error!("background task panicked: {info}");
        }
    }));
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Launches the full-screen TUI.
///
/// Owns the terminal for its lifetime and always restores it before returning.
pub async fn run_tui(initial: InitialConfig) -> ExitCode {
    if !stdout().is_terminal() {
        eprintln!(
            "--tui needs an interactive terminal on stdout (use `docker run -it` or \
             `docker compose run`), or drop --tui for a non-interactive backup."
        );
        return ExitCode::from(2);
    }

    let guard = match TerminalGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!(
                "failed to start the terminal UI: {e}\n\
                 --tui needs an interactive terminal (use `docker run -it` or \
                 `docker compose run`)."
            );
            return ExitCode::FAILURE;
        }
    };
    install_panic_hook();

    let mut terminal = match Terminal::new(CrosstermBackend::new(stdout())) {
        Ok(t) => t,
        Err(e) => {
            drop(guard);
            eprintln!("failed to create terminal: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Progress channel shared by the backup task, verify task, signal task and
    // the tracing subscriber layer.
    let (progress_tx, mut progress_rx) = progress::channel();

    // Install the TUI tracing layer so log output goes to the log panel.  Only
    // this crate's own events (and warnings from anything) are shown by default;
    // RUST_LOG overrides that.
    let tui_layer =
        tracing_layer::TuiTracingLayer::new(progress_tx.clone()).with_filter(log_filter());
    let _ = tracing_subscriber::registry().with(tui_layer).try_init();

    spawn_signal_task(progress_tx.clone());

    let (term_tx, mut term_rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
    let stop_input = Arc::new(AtomicBool::new(false));
    let input_thread = spawn_input_thread(term_tx, Arc::clone(&stop_input));

    let mut app = app::App::new(initial);

    let result = event_loop(
        &mut terminal,
        &mut app,
        &mut progress_rx,
        progress_tx,
        &mut term_rx,
    )
    .await;

    stop_input.store(true, Ordering::Relaxed);
    join_with_timeout(input_thread, Duration::from_millis(400));

    // Leave the alternate screen before anything is printed.
    drop(terminal);
    drop(guard);

    if app.run.is_active() {
        // The backup task did not stop in time.  The runtime would wait for it
        // on exit, so make sure the process does leave.
        let code = i32::from(app.shutdown_code.max(1));
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(3));
            std::process::exit(code);
        });
    }

    match result {
        Ok(()) if app.shutdown_requested => ExitCode::from(app.shutdown_code),
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("TUI error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Joins `handle` if it ends within `patience`, otherwise leaves it behind.
///
/// The input reader normally notices its stop flag within 100 ms.  If the
/// terminal has hung up, crossterm's blocking `read` can spin on EOF and never
/// return, so an unconditional `join` hangs the process (it did when the
/// terminal window was closed).  A left-behind thread dies with the process.
fn join_with_timeout(handle: std::thread::JoinHandle<()>, patience: Duration) -> bool {
    let deadline = Instant::now() + patience;
    while !handle.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if handle.is_finished() {
        let _ = handle.join();
        true
    } else {
        false
    }
}

fn log_filter() -> tracing_subscriber::EnvFilter {
    const DEFAULT: &str = "warn,github_backup=info,github_backup_core=info,\
        github_backup_client=info,github_backup_types=info,github_backup_tui=info";
    std::env::var("RUST_LOG")
        .ok()
        .and_then(|v| tracing_subscriber::EnvFilter::try_new(v).ok())
        .unwrap_or_else(|| tracing_subscriber::EnvFilter::new(DEFAULT))
}

/// Turns SIGINT/SIGTERM/SIGHUP/SIGQUIT into [`event::BackupEvent::Shutdown`].
///
/// In raw mode Ctrl+C is an ordinary key; these signals only arrive from
/// outside (`kill`, `docker stop`, a closing terminal).
fn spawn_signal_task(tx: progress::ProgressTx) {
    #[cfg(unix)]
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let (Ok(mut term), Ok(mut hup), Ok(mut int), Ok(mut quit)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::hangup()),
            signal(SignalKind::interrupt()),
            signal(SignalKind::quit()),
        ) else {
            return;
        };
        loop {
            let code = tokio::select! {
                _ = term.recv() => 143u8,
                _ = hup.recv() => 129u8,
                _ = int.recv() => 130u8,
                _ = quit.recv() => 131u8,
            };
            if tx.send(event::BackupEvent::Shutdown { code }).is_err() {
                return;
            }
        }
    });
    #[cfg(not(unix))]
    tokio::spawn(async move {
        while tokio::signal::ctrl_c().await.is_ok() {
            if tx.send(event::BackupEvent::Shutdown { code: 130 }).is_err() {
                return;
            }
        }
    });
}

/// Reads terminal events on a dedicated thread so the async loop never blocks.
fn spawn_input_thread(
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match ratatui::crossterm::event::poll(Duration::from_millis(100)) {
                Ok(true) => match ratatui::crossterm::event::read() {
                    Ok(ev) => {
                        if tx.send(ev).is_err() {
                            return;
                        }
                    }
                    Err(_) => return,
                },
                Ok(false) => {}
                Err(_) => return,
            }
        }
    })
}

// ── Event loop ────────────────────────────────────────────────────────────────

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut app::App,
    progress_rx: &mut progress::ProgressRx,
    progress_tx: progress::ProgressTx,
    term_rx: &mut tokio::sync::mpsc::UnboundedReceiver<Event>,
) -> std::io::Result<()> {
    // Wakes the loop so the elapsed clock and the shutdown deadline are served;
    // the screen is only redrawn when something changed.
    let mut ticker = tokio::time::interval(Duration::from_millis(250));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut dirty = true;
    let mut last_elapsed = String::new();
    let mut shutdown_deadline: Option<Instant> = None;

    loop {
        // ── Spawn backup task if requested ────────────────────────────────
        if app.start_backup_requested {
            app.start_backup_requested = false;

            let tx = progress_tx.clone();
            let cfg = app.config.clone();
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
            app.cancel_tx = Some(cancel_tx);

            // Supervised: a panic inside the backup becomes a visible failure
            // instead of a screen that waits forever.
            tokio::spawn(async move {
                let inner = tokio::spawn(run_backup_task(cfg, tx.clone(), cancel_rx));
                if let Err(e) = inner.await {
                    let _ = tx.send(event::BackupEvent::BackupFailed {
                        error: format!("backup task panicked: {e}"),
                    });
                }
            });
            dirty = true;
        }

        // ── Spawn verify task if requested ────────────────────────────────
        if app.start_verify_requested {
            app.start_verify_requested = false;

            let vtx = progress_tx.clone();
            let owner = app.config.owner.trim().to_string();
            let output_dir = app.config.output_dir.trim().to_string();

            tokio::spawn(async move {
                run_verify_task(owner, output_dir, vtx).await;
            });
            dirty = true;
        }

        // ── Leave? ─────────────────────────────────────────────────────────
        if app.shutdown_requested && shutdown_deadline.is_none() {
            let grace = if app.shutdown_code == 129 {
                HANGUP_GRACE
            } else {
                SHUTDOWN_GRACE
            };
            shutdown_deadline = Some(Instant::now() + grace);
        }
        let grace_over = shutdown_deadline.is_some_and(|d| Instant::now() >= d);
        if app.should_quit || (app.shutdown_requested && (!app.run.is_active() || grace_over)) {
            break;
        }

        // ── Render only when something changed ────────────────────────────
        if app.run.is_active() {
            let el = app.run.elapsed_str();
            if el != last_elapsed {
                last_elapsed = el;
                dirty = true;
            }
        }
        if dirty {
            terminal.draw(|frame| render(frame, app))?;
            dirty = false;
        }

        // ── Wait for input, progress, or the next tick ────────────────────
        tokio::select! {
            ev = term_rx.recv() => match ev {
                Some(Event::Key(key)) => {
                    if key.kind == KeyEventKind::Press {
                        app::handle_key_dispatch(app, key.code, key.modifiers);
                    }
                    dirty = true;
                }
                Some(Event::Paste(text)) => {
                    app::handle_paste(app, &text);
                    dirty = true;
                }
                Some(Event::Resize(..)) => dirty = true,
                Some(_) => {}
                // The input thread died: nothing can be typed any more.
                None => app.should_quit = true,
            },
            ev = progress_rx.recv() => {
                if let Some(ev) = ev {
                    app::handle_backup_event(app, ev);
                    while let Ok(ev) = progress_rx.try_recv() {
                        app::handle_backup_event(app, ev);
                    }
                    dirty = true;
                }
            }
            _ = ticker.tick() => {}
        }
    }

    Ok(())
}

// ── Frame rendering ───────────────────────────────────────────────────────────

fn render(frame: &mut Frame, app: &app::App) {
    let area = frame.area();

    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_too_small(frame, area);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    render_title_bar(frame, app, rows[0]);
    render_screen_content(frame, app, rows[1]);
    if app.modal_error.is_some() {
        render_error_modal(frame, app, rows[1]);
    }
}

fn render_too_small(frame: &mut Frame, area: Rect) {
    let w = area.width as usize;
    let lines = vec![
        Line::from(Span::styled(
            fit("Terminal too small", w),
            theme::WARN_STYLE.add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            fit(
                &format!(
                    "{}x{} now, need {MIN_WIDTH}x{MIN_HEIGHT}",
                    area.width, area.height
                ),
                w,
            ),
            theme::DIM,
        )),
        Line::from(Span::styled(
            fit("Resize, or Ctrl+C to quit", w),
            theme::DIM,
        )),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_title_bar(frame: &mut Frame, app: &app::App, area: Rect) {
    use state::Screen;
    let cur = &app.screen;
    let tabs: [(&'static str, &'static str, Screen); 5] = [
        ("1", "Dashboard", Screen::Dashboard),
        ("2", "Configure", Screen::Configure),
        ("3", "Run", Screen::Running),
        ("4", "Verify", Screen::Verify),
        ("5", "Results", Screen::Results),
    ];
    let name = Span::styled(" github-backup ", theme::ACCENT_BOLD);
    let version = Span::styled(concat!("v", env!("CARGO_PKG_VERSION"), "  "), theme::DIM);

    // Widest layout that fits the row: name + version + all labels, then
    // without the version, then without the name, and finally digits only
    // with just the active screen spelled out.
    let full = |lead: Vec<Span<'static>>| {
        let mut spans = lead;
        spans.extend(tabs.iter().map(|(k, l, s)| nav_tab(k, l, s == cur)));
        Line::from(spans)
    };
    let digits = || {
        let mut spans = vec![Span::raw(" ")];
        spans.extend(tabs.iter().map(|(k, l, s)| {
            if s == cur {
                Span::styled(
                    format!("[{k} {l}] "),
                    theme::ACCENT_STYLE.add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(format!("{k} "), theme::DIM)
            }
        }));
        Line::from(spans)
    };
    let candidates = [
        full(vec![name.clone(), version]),
        full(vec![name]),
        full(vec![Span::raw(" ")]),
        digits(),
    ];
    let width = area.width as usize;
    let line = candidates
        .iter()
        .find(|l| l.width() <= width)
        .cloned()
        .unwrap_or_else(|| candidates[3].clone());
    frame.render_widget(Paragraph::new(line), area);
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

fn render_screen_content(frame: &mut Frame, app: &app::App, area: Rect) {
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

/// Rectangle for the error modal: as large as its text needs, never larger
/// than `area`.
fn modal_rect(area: Rect, message: &str) -> Rect {
    let w = (area.width * 2 / 3).clamp(40, 70).min(area.width);
    let inner_w = usize::from(w.saturating_sub(2)).max(1);
    let text_rows = message.chars().count().div_ceil(inner_w).max(1);
    // borders (2) + message + blank + dismiss hint
    let h = u16::try_from(text_rows + 4)
        .unwrap_or(u16::MAX)
        .max(5)
        .min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

fn render_error_modal(frame: &mut Frame, app: &app::App, area: Rect) {
    let err = app.modal_error.as_deref().unwrap_or("");
    let popup = modal_rect(area, err);

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

/// How a backup run ended, before it is turned into an event.
enum RunEnd {
    Finished(Result<github_backup_core::BackupStats, github_backup_core::CoreError>),
    Cancelled,
}

async fn run_backup_task(
    cfg: state::ConfigState,
    tx: progress::ProgressTx,
    cancel_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let (owner, output_path, opts, token_opt) = cfg.to_backup_config();
    let dry_run = opts.dry_run;
    let write_manifest = cfg.manifest;
    let started_at = chrono::Utc::now();

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

    // The engine reports per-repository progress and outcome on its own
    // channel; forward it to the UI.
    let (engine_tx, mut engine_rx) = tokio::sync::mpsc::unbounded_channel();
    let forward = {
        let tx = tx.clone();
        tokio::spawn(async move {
            while let Some(ev) = engine_rx.recv().await {
                let ev = match ev {
                    EngineEvent::ReposDiscovered { total } => {
                        event::BackupEvent::ReposDiscovered { total }
                    }
                    EngineEvent::RepoStarted { name } => event::BackupEvent::RepoStarted { name },
                    EngineEvent::RepoCompleted {
                        name,
                        success,
                        error,
                    } => event::BackupEvent::RepoCompleted {
                        name,
                        success,
                        error,
                    },
                };
                if tx.send(ev).is_err() {
                    break;
                }
            }
        })
    };

    let output = OutputConfig::new(&output_path);
    let engine = BackupEngine::new(
        client,
        FsStorage::new(),
        ProcessGitRunner::new(),
        output.clone(),
        opts,
    )
    .with_event_channel(engine_tx);

    // Cancelling must really stop the run: git processes are killed and no
    // further work starts, then the engine is awaited so its lock is released.
    let cancel = engine.cancel_handle();
    let end = {
        let mut run = Box::pin(engine.run(&owner));
        // A dropped sender (the UI went away) is not a cancel request.
        let cancel_requested = async {
            if cancel_rx.await.is_err() {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            result = &mut run => RunEnd::Finished(result),
            () = cancel_requested => {
                cancel.cancel();
                let _ = run.await;
                RunEnd::Cancelled
            }
        }
    };
    // Let the engine's event sender drop so the forwarder drains and ends; the
    // last repository events must reach the UI before the final one does.
    drop(engine);
    let _ = tokio::time::timeout(Duration::from_secs(2), forward).await;

    match end {
        RunEnd::Cancelled => {
            let _ = tx.send(event::BackupEvent::BackupCancelled);
        }
        RunEnd::Finished(Err(e)) => {
            let _ = tx.send(event::BackupEvent::BackupFailed {
                error: e.to_string(),
            });
        }
        RunEnd::Finished(Ok(stats)) => {
            if !dry_run {
                finish_run(&stats, &output, &owner, started_at, write_manifest).await;
            }
            let _ = tx.send(event::BackupEvent::BackupDone {
                repos_backed_up: stats.repos_backed_up(),
                repos_discovered: stats.repos_discovered(),
                repos_skipped: stats.repos_skipped(),
                repos_errored: stats.repos_errored(),
                gists_backed_up: stats.gists_backed_up(),
                issues_fetched: stats.issues_fetched(),
                prs_fetched: stats.prs_fetched(),
                workflows_fetched: stats.workflows_fetched(),
                elapsed_secs: stats.elapsed_secs(),
                failures: stats.failures(),
                dry_run,
            });
        }
    }
}

/// Post-run work the engine does not do: the optional SHA-256 manifest and the
/// run-history entry the Dashboard reads.  Anything that fails here is recorded
/// as a failure of the run, so the Results screen cannot call it complete.
///
/// (The engine itself writes the incremental state file.)
async fn finish_run(
    stats: &github_backup_core::BackupStats,
    output: &OutputConfig,
    owner: &str,
    started_at: chrono::DateTime<chrono::Utc>,
    write_manifest: bool,
) {
    const SCOPE: &str = "post-processing";
    let stats = stats.handle();
    let output = output.clone();
    let owner = owner.to_string();
    let started = started_at.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let work = {
        let stats = stats.handle();
        tokio::task::spawn_blocking(move || {
            if write_manifest {
                match github_backup_core::write_manifest(&output.owner_json_dir(&owner), &started) {
                    Ok(n) => tracing::info!(entries = n, "SHA-256 manifest written"),
                    Err(e) => {
                        tracing::error!(error = %e, "failed to write manifest");
                        stats.record_failure(
                            SCOPE,
                            "manifest",
                            format!("failed to write manifest: {e}"),
                        );
                    }
                }
            }
            append_history(&output, &owner, &stats, &started);
        })
    };
    if let Err(e) = work.await {
        stats.record_failure(SCOPE, "task", format!("post-run task panicked: {e}"));
    }
}

fn append_history(
    output: &OutputConfig,
    owner: &str,
    stats: &github_backup_core::BackupStats,
    started: &str,
) {
    use github_backup_types::backup_state::{BackupRunEntry, BackupRunHistory};

    let path = output.backup_history_path(owner);
    let mut history = BackupRunHistory::load(&path).unwrap_or_default();
    history.push(
        BackupRunEntry {
            timestamp: started.to_string(),
            repos_backed_up: stats.repos_backed_up(),
            elapsed_secs: stats.elapsed_secs().round(),
            success: !stats.has_failures(),
            failures: stats.failure_count() as u64,
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
        },
        BackupRunHistory::MAX_ENTRIES,
    );
    if let Err(e) = history.save(&path) {
        tracing::warn!(error = %e, "failed to write backup history file");
    }
}

// ── Verify task ───────────────────────────────────────────────────────────────

async fn run_verify_task(owner: String, output_dir: String, tx: progress::ProgressTx) {
    let output = OutputConfig::new(&output_dir);
    let json_dir = output.owner_json_dir(&owner);
    let manifest_exists = json_dir
        .join(github_backup_core::manifest::MANIFEST_FILENAME)
        .exists();

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
            let error = if manifest_exists {
                e.to_string()
            } else {
                "No manifest found. Turn on 'Write SHA-256 Manifest' (Configure > Output), \
                 run a backup, then verify."
                    .to_string()
            };
            let _ = tx.send(event::BackupEvent::VerifyFailed { error });
        }
        Err(e) => {
            let _ = tx.send(event::BackupEvent::VerifyFailed {
                error: format!("verify task panicked: {e}"),
            });
        }
    }
}
