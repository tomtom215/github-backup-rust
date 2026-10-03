// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Running screen — live backup progress view.
//!
//! Layout (80x24 and larger):
//!  ┌ progress bar ────────────────────────────────────────────────────┐
//!  ├ repo list (left, 35%) ── live log (right, 65%) ─────────────────┤
//!  └ counters + key hints ───────────────────────────────────────────┘
//!
//! Below 70 columns the repo list sits above the log instead of beside it;
//! below 14 rows the borders around the progress bar and counters go away.
//! A repository that finished with a failure is shown as `!!` in red and the
//! run is labelled INCOMPLETE as soon as the first failure is known.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use super::util::{fit, hint_lines, one_line};
use crate::state::{RepoEntry, RepoStatus, RunState, RunStatus};
use crate::theme;

pub fn render(frame: &mut Frame, run: &RunState, area: Rect) {
    if run.status == RunStatus::Idle && run.started_at.is_none() {
        render_idle(frame, area);
        return;
    }

    let compact = area.height < 14;
    let (gauge_h, stats_h) = if compact { (1, 2) } else { (3, 3) };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(gauge_h),
            Constraint::Min(0), // main split
            Constraint::Length(stats_h),
        ])
        .split(area);

    render_progress(frame, run, rows[0], compact);

    let main = rows[1];
    if main.width >= 70 {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(main);
        render_repo_list(frame, run, cols[0]);
        render_log(frame, run, cols[1]);
    } else if main.height >= 8 {
        // At least 4 rows (2 visible entries) for the list, the rest for the log.
        let list_h = (main.height * 2 / 5)
            .max(4)
            .min(main.height.saturating_sub(4));
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(list_h), Constraint::Min(0)])
            .split(main);
        render_repo_list(frame, run, parts[0]);
        render_log(frame, run, parts[1]);
    } else {
        // Too short for both: the log is the more useful of the two.
        render_log(frame, run, main);
    }
    render_stats(frame, run, rows[2], compact);
}

fn render_idle(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(Span::styled("No backup is running.", theme::NORMAL)),
        Line::from(""),
        Line::from(vec![
            Span::styled("Press ", theme::DIM),
            Span::styled("1", theme::KEY_HINT),
            Span::styled(" for the dashboard, then ", theme::DIM),
            Span::styled("r", theme::KEY_HINT),
            Span::styled(" to start one.", theme::DIM),
        ]),
        Line::from(vec![
            Span::styled("Esc", theme::KEY_HINT),
            Span::styled(" or ", theme::DIM),
            Span::styled("q", theme::KEY_HINT),
            Span::styled(" go back.", theme::DIM),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

/// Style for the run as a whole: red-free but unmistakable once anything failed.
fn run_style(run: &RunState) -> Style {
    if run.status == RunStatus::Cancelling {
        theme::WARN_STYLE
    } else if run.incomplete_repos() > 0 {
        theme::ERR_STYLE
    } else {
        theme::ACCENT_STYLE
    }
}

fn render_progress(frame: &mut Frame, run: &RunState, area: Rect, compact: bool) {
    let done = run
        .processed
        .max(run.repos_done + run.repos_errored + run.repos_skipped);
    let failed = run.incomplete_repos();
    let label = if run.total_repos > 0 {
        if failed > 0 {
            format!("{done} / {} repos, {failed} failed", run.total_repos)
        } else {
            format!("{done} / {} repos", run.total_repos)
        }
    } else {
        run.phase.clone()
    };

    let title = if failed > 0 && run.status != RunStatus::Cancelling {
        format!(" {} - INCOMPLETE: {failed} failed ", run.phase)
    } else {
        format!(" {} ", run.phase)
    };

    let style = run_style(run);
    let mut gauge = Gauge::default()
        .gauge_style(style.add_modifier(Modifier::BOLD))
        .percent(run.progress_pct())
        .label(fit(&label, (area.width as usize).saturating_sub(2)));

    if compact {
        // One row: no room for a border, so the status goes into the label.
        gauge = gauge.label(fit(
            &format!("{} | {label}", title.trim()),
            (area.width as usize).saturating_sub(1),
        ));
        frame.render_widget(gauge, area);
    } else {
        gauge = gauge.block(
            Block::default()
                .title(Span::styled(
                    fit(&title, (area.width as usize).saturating_sub(2)),
                    style.add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(style),
        );
        frame.render_widget(gauge, area);
    }
}

/// Rows in display order: failed repositories first (so a failure can never
/// scroll out of sight), then everything else in the order the engine took it.
fn display_order(run: &RunState) -> Vec<&RepoEntry> {
    let (failed, rest): (Vec<&RepoEntry>, Vec<&RepoEntry>) = run
        .repos
        .iter()
        .partition(|r| r.status == RepoStatus::Error);
    failed.into_iter().chain(rest).collect()
}

fn render_repo_list(frame: &mut Frame, run: &RunState, area: Rect) {
    let failed = run.incomplete_repos();
    let title = if failed > 0 {
        format!(" Repositories ({}, {failed} failed) ", run.repos.len())
    } else {
        format!(" Repositories ({}) ", run.repos.len())
    };
    let block = Block::default()
        .title(Span::styled(
            fit(&title, (area.width as usize).saturating_sub(2)),
            if failed > 0 {
                theme::ERR_STYLE.add_modifier(Modifier::BOLD)
            } else {
                theme::TITLE
            },
        ))
        .borders(Borders::ALL)
        .border_style(theme::DIM);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height as usize;
    if visible_height == 0 {
        return;
    }
    let ordered = display_order(run);
    let max_offset = ordered.len().saturating_sub(visible_height);

    // Determine scroll offset so the currently-running repo stays visible.
    let active_idx = ordered
        .iter()
        .rposition(|r| r.status == RepoStatus::Running);

    let offset = if let Some(idx) = active_idx {
        if idx >= run.repo_list_offset + visible_height {
            idx.saturating_sub(visible_height.saturating_sub(1))
        } else if idx < run.repo_list_offset {
            idx
        } else {
            run.repo_list_offset
        }
    } else {
        run.repo_list_offset
    }
    .min(max_offset);

    let width = inner.width as usize;
    let items: Vec<ListItem> = ordered
        .iter()
        .skip(offset)
        .take(visible_height)
        .map(|r| repo_list_item(r, width))
        .collect();

    // Compute selected row relative to visible window.
    let sel = active_idx.and_then(|idx| {
        if idx >= offset && idx < offset + visible_height {
            Some(idx - offset)
        } else {
            None
        }
    });

    let mut state = ListState::default();
    state.select(sel);

    let list = List::new(items).highlight_style(theme::SELECTED);
    frame.render_stateful_widget(list, inner, &mut state);
}

fn repo_list_item(entry: &RepoEntry, width: usize) -> ListItem<'_> {
    let (icon, style) = match entry.status {
        RepoStatus::Running => (" >> ", theme::ACCENT_STYLE.add_modifier(Modifier::BOLD)),
        RepoStatus::Done => (" ok ", theme::OK_STYLE),
        RepoStatus::Error => (" !! ", theme::ERR_STYLE.add_modifier(Modifier::BOLD)),
        RepoStatus::Skipped => (" -- ", theme::DIM),
    };

    // Strip the owner prefix for brevity.
    let short_name = entry.name.split_once('/').map_or(&*entry.name, |(_, n)| n);
    let room = width.saturating_sub(icon.chars().count());

    // For failed repos, append the reason so the operator can see what went
    // wrong without leaving the TUI.
    if entry.status == RepoStatus::Error {
        if let Some(ref msg) = entry.error {
            let name = fit(short_name, room);
            let left = room.saturating_sub(name.chars().count() + 2);
            let mut spans = vec![Span::styled(icon, style), Span::styled(name, style)];
            if left >= 4 {
                spans.push(Span::styled(
                    format!(": {}", fit(&one_line(msg), left)),
                    theme::DIM,
                ));
            }
            return ListItem::new(Line::from(spans));
        }
    }

    ListItem::new(Line::from(vec![
        Span::styled(icon, style),
        Span::styled(fit(short_name, room), style),
    ]))
}

/// Entries of the log that fit `height` rows given how far back the view is
/// scrolled.  One entry is one row (long lines are cut, not wrapped), so the
/// newest entries can never be pushed out of view.
pub fn log_window(total: usize, height: usize, back: usize) -> std::ops::Range<usize> {
    // Scrolled all the way back the window still fills the panel.
    let end = total
        .saturating_sub(back.min(total.saturating_sub(1)))
        .max(height.min(total));
    let start = end.saturating_sub(height);
    start..end
}

fn render_log(frame: &mut Frame, run: &RunState, area: Rect) {
    let title = if run.log_back > 0 {
        " Log (scrolled back, G = newest) "
    } else {
        " Log "
    };
    let block = Block::default()
        .title(Span::styled(
            fit(title, (area.width as usize).saturating_sub(2)),
            theme::TITLE,
        ))
        .borders(Borders::ALL)
        .border_style(theme::DIM);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let width = inner.width as usize;
    let window = log_window(run.log_lines.len(), inner.height as usize, run.log_back);

    let lines: Vec<Line> = run
        .log_lines
        .iter()
        .skip(window.start)
        .take(window.len())
        .map(|ll| {
            let level_style = theme::log_level_style(&ll.level);
            // "HH:MM:SS LEVEL " is 15 columns; the message gets the rest.
            let msg = fit(&one_line(&ll.message), width.saturating_sub(15));
            Line::from(vec![
                Span::styled(ll.timestamp.clone(), theme::DIM),
                Span::raw(" "),
                Span::styled(format!("{:<5}", ll.level), level_style),
                Span::raw(" "),
                Span::styled(msg, theme::NORMAL),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_stats(frame: &mut Frame, run: &RunState, area: Rect, compact: bool) {
    let failed = run.incomplete_repos();
    let counters = Line::from(vec![
        Span::styled("Repos: ", theme::DIM),
        Span::styled(
            format!(
                "{}/{}",
                run.processed
                    .max(run.repos_done + run.repos_errored + run.repos_skipped),
                run.total_repos
            ),
            theme::ACCENT_STYLE,
        ),
        Span::raw("  "),
        Span::styled("Failed: ", theme::DIM),
        Span::styled(
            failed.to_string(),
            if failed > 0 {
                theme::ERR_STYLE.add_modifier(Modifier::BOLD)
            } else {
                theme::NORMAL
            },
        ),
        Span::raw("  "),
        Span::styled("Elapsed: ", theme::DIM),
        Span::styled(run.elapsed_str(), theme::NORMAL),
    ]);

    let hints = match run.status {
        RunStatus::Active => hint_lines(
            &[
                ("Ctrl+C", "cancel"),
                ("j/k", "repos"),
                ("g/G", "log top/end"),
                ("PgUp/PgDn", "log"),
            ],
            area.width as usize,
            1,
        ),
        RunStatus::Cancelling => vec![Line::from(Span::styled(
            "Cancelling: waiting for git to stop...",
            theme::WARN_STYLE,
        ))],
        RunStatus::Idle => hint_lines(
            &[
                ("Esc", "back"),
                ("1-5", "screens"),
                ("j/k", "repos"),
                ("g/G", "log"),
            ],
            area.width as usize,
            1,
        ),
    };

    let mut lines = vec![counters];
    lines.extend(hints);

    let mut para = Paragraph::new(lines);
    if !compact {
        para = para.block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(theme::DIM),
        );
    }
    frame.render_widget(para, area);
}
