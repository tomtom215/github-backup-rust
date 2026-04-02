// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Running screen — live backup progress view.
//!
//! Layout:
//!  ┌ progress bar ────────────────────────────────────────────────────┐
//!  ├ repo list (left, 35%) ── live log (right, 65%) ─────────────────┤
//!  └ stats bar ──────────────────────────────────────────────────────┘

use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::state::{RepoEntry, RepoStatus, RunState};
use crate::theme;

pub fn render(frame: &mut Frame, run: &RunState, area: ratatui::layout::Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // progress bar
            Constraint::Min(0),    // main split
            Constraint::Length(3), // stats bar
        ])
        .split(area);

    render_progress(frame, run, rows[0]);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(rows[1]);

    render_repo_list(frame, run, cols[0]);
    render_log(frame, run, cols[1]);
    render_stats(frame, run, rows[2]);
}

fn render_progress(frame: &mut Frame, run: &RunState, area: ratatui::layout::Rect) {
    let done = run.repos_done + run.repos_errored + run.repos_skipped;
    let label = if run.total_repos > 0 {
        format!(
            "{} / {}  repos  ({} errors)",
            done, run.total_repos, run.repos_errored
        )
    } else {
        run.phase.clone()
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(Span::styled(format!(" {} ", run.phase), theme::TITLE))
                .borders(Borders::ALL)
                .border_style(theme::ACCENT_STYLE),
        )
        .gauge_style(theme::ACCENT_STYLE.add_modifier(Modifier::BOLD))
        .percent(run.progress_pct())
        .label(label);

    frame.render_widget(gauge, area);
}

fn render_repo_list(frame: &mut Frame, run: &RunState, area: ratatui::layout::Rect) {
    let block = Block::default()
        .title(Span::styled(
            format!(" Repositories ({}) ", run.repos.len()),
            theme::TITLE,
        ))
        .borders(Borders::ALL)
        .border_style(theme::DIM);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height as usize;

    // Determine scroll offset so the currently-running repo stays visible.
    let active_idx = run
        .repos
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
    };

    let items: Vec<ListItem> = run
        .repos
        .iter()
        .skip(offset)
        .take(visible_height)
        .map(|r| repo_list_item(r))
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

fn repo_list_item(entry: &RepoEntry) -> ListItem<'_> {
    let (icon, style) = match entry.status {
        RepoStatus::Pending => ("  . ", theme::DIM),
        RepoStatus::Running => (" >> ", theme::ACCENT_STYLE.add_modifier(Modifier::BOLD)),
        RepoStatus::Done => (" ok ", theme::OK_STYLE),
        RepoStatus::Error => (" !! ", theme::ERR_STYLE),
        RepoStatus::Skipped => (" -- ", theme::DIM),
    };

    // Extract repo name (strip owner prefix for brevity).
    let short_name = if let Some(pos) = entry.name.find('/') {
        &entry.name[pos + 1..]
    } else {
        &entry.name
    };

    // For errored repos, append a short reason when available so the operator
    // can see what went wrong without leaving the TUI.
    if entry.status == RepoStatus::Error {
        if let Some(ref msg) = entry.error {
            // Truncate long error messages to keep the list readable.
            let truncated: String = msg.chars().take(60).collect();
            let suffix = if msg.len() > 60 { "…" } else { "" };
            return ListItem::new(Line::from(vec![
                Span::styled(icon, style),
                Span::styled(short_name, style),
                Span::styled(format!(": {truncated}{suffix}"), theme::DIM),
            ]));
        }
    }

    ListItem::new(Line::from(vec![
        Span::styled(icon, style),
        Span::styled(short_name, style),
    ]))
}

fn render_log(frame: &mut Frame, run: &RunState, area: ratatui::layout::Rect) {
    let block = Block::default()
        .title(Span::styled(" Log ", theme::TITLE))
        .borders(Borders::ALL)
        .border_style(theme::DIM);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height as usize;
    let total = run.log_lines.len();
    let offset = if total > visible_height {
        // Use stored scroll offset, but clamp to valid range.
        run.log_offset.min(total.saturating_sub(visible_height))
    } else {
        0
    };

    let lines: Vec<Line> = run
        .log_lines
        .iter()
        .skip(offset)
        .take(visible_height)
        .map(|ll| {
            let level_style = theme::log_level_style(&ll.level);
            Line::from(vec![
                Span::styled(ll.timestamp.clone(), theme::DIM),
                Span::raw(" "),
                Span::styled(format!("{:<5}", ll.level), level_style),
                Span::raw(" "),
                Span::styled(ll.message.clone(), theme::NORMAL),
            ])
        })
        .collect();

    let para = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(para, inner);
}

fn render_stats(frame: &mut Frame, run: &RunState, area: ratatui::layout::Rect) {
    let line = Line::from(vec![
        Span::styled("Repos: ", theme::DIM),
        Span::styled(
            format!("{}/{}", run.repos_done, run.total_repos),
            theme::ACCENT_STYLE,
        ),
        Span::raw("  "),
        Span::styled("Errors: ", theme::DIM),
        Span::styled(
            run.repos_errored.to_string(),
            if run.repos_errored > 0 {
                theme::ERR_STYLE
            } else {
                theme::NORMAL
            },
        ),
        Span::raw("  "),
        Span::styled("Elapsed: ", theme::DIM),
        Span::styled(run.elapsed_str(), theme::NORMAL),
        Span::raw("     "),
        Span::styled("Ctrl+C", theme::KEY_HINT),
        Span::styled(" cancel  ", theme::KEY_DESC),
        Span::styled("j/k", theme::KEY_HINT),
        Span::styled(" scroll repos  ", theme::KEY_DESC),
        Span::styled("g/G", theme::KEY_HINT),
        Span::styled(" scroll log", theme::KEY_DESC),
    ]);

    let para = Paragraph::new(line)
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(theme::DIM),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(para, area);
}
