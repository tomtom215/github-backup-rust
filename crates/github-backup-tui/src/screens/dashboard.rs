// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Dashboard screen — the first screen a user sees.
//!
//! Shows the last backup summary and quick-action menu.  Below about 15 rows
//! or 56 columns it switches to a compact layout without borders so the
//! actions and the key hints always stay visible.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use super::util::{fit, fit_tail, hint_lines};
use crate::state::{ConfigState, DashboardState};
use crate::theme;

pub fn render(frame: &mut Frame, dash: &DashboardState, cfg: &ConfigState, area: Rect) {
    let compact = area.height < 15 || area.width < 56;
    let actions = DashboardState::ACTIONS.len() as u16;

    if compact {
        let info_h = 4.min(area.height.saturating_sub(actions + 1));
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(info_h),
                Constraint::Length(actions.min(area.height.saturating_sub(info_h))),
                Constraint::Min(0),
            ])
            .split(area);
        render_info_compact(frame, dash, cfg, rows[0]);
        render_actions(frame, dash, rows[1], false);
        render_hint(frame, rows[2]);
    } else {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5), // info panel
                Constraint::Min(actions + 2),
                Constraint::Length(2), // hint
            ])
            .split(area);
        render_info(frame, dash, cfg, rows[0]);
        render_actions(frame, dash, rows[1], true);
        render_hint(frame, rows[2]);
    }
}

/// `"2026-10-03T18:49:42Z  3 repos  OK"` style summary of the last run.
fn last_run_result(dash: &DashboardState) -> Span<'static> {
    match (dash.last_run_ok, dash.last_run_failures) {
        (Some(true), _) => Span::styled("complete", theme::OK_STYLE),
        (Some(false), 0) => Span::styled("incomplete", theme::WARN_STYLE),
        (Some(false), n) => Span::styled(
            format!("INCOMPLETE ({n} failure{})", if n == 1 { "" } else { "s" }),
            theme::WARN_STYLE,
        ),
        (None, _) => Span::styled("-", theme::DIM),
    }
}

fn owner_span(cfg: &ConfigState, width: usize) -> Span<'static> {
    if cfg.owner.trim().is_empty() {
        Span::styled("(not configured)", theme::WARN_STYLE)
    } else {
        Span::styled(fit(cfg.owner.trim(), width), theme::ACCENT_BOLD)
    }
}

fn token_span(cfg: &ConfigState) -> Span<'static> {
    if cfg.token.trim().is_empty() {
        Span::styled("not set", theme::WARN_STYLE)
    } else {
        Span::styled("configured", theme::OK_STYLE)
    }
}

fn render_info(frame: &mut Frame, dash: &DashboardState, cfg: &ConfigState, area: Rect) {
    let block = Block::default()
        .title(Span::styled(" Status ", theme::TITLE))
        .borders(Borders::ALL)
        .border_style(theme::ACCENT_STYLE);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    // Left column: 8 columns of label.
    // One column of gap before the right-hand column.
    let lw = (cols[0].width as usize).saturating_sub(8 + 1);
    let output = if cfg.output_dir.trim().is_empty() {
        "(not set)".to_string()
    } else {
        fit_tail(cfg.output_dir.trim(), lw)
    };
    let left = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Owner:  ", theme::DIM),
            owner_span(cfg, lw),
        ]),
        Line::from(vec![
            Span::styled("Output: ", theme::DIM),
            Span::styled(output, theme::NORMAL),
        ]),
        Line::from(vec![Span::styled("Token:  ", theme::DIM), token_span(cfg)]),
    ]);
    frame.render_widget(left, cols[0]);

    // Right column: 10 columns of label.
    let rw = (cols[1].width as usize).saturating_sub(10 + 1);
    let last_run = fit(dash.last_backup_time.as_deref().unwrap_or("never"), rw);
    let last_repos = dash
        .last_backup_repos
        .map(|n| n.to_string())
        .unwrap_or_else(|| "-".into());
    let right = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Last run: ", theme::DIM),
            Span::styled(last_run, theme::NORMAL),
        ]),
        Line::from(vec![
            Span::styled("Repos:    ", theme::DIM),
            Span::styled(last_repos, theme::NORMAL),
        ]),
        Line::from(vec![
            Span::styled("Result:   ", theme::DIM),
            last_run_result(dash),
        ]),
    ]);
    frame.render_widget(right, cols[1]);
}

fn render_info_compact(frame: &mut Frame, dash: &DashboardState, cfg: &ConfigState, area: Rect) {
    let w = area.width as usize;
    let token = if cfg.token.trim().is_empty() {
        Span::styled("not set", theme::WARN_STYLE)
    } else {
        Span::styled("ok", theme::OK_STYLE)
    };
    let output = if cfg.output_dir.trim().is_empty() {
        "(not set)".to_string()
    } else {
        fit_tail(cfg.output_dir.trim(), w.saturating_sub(8))
    };
    let when = dash.last_backup_time.as_deref().unwrap_or("never");
    // Most important first: the area may only have room for the first rows.
    let lines = vec![
        Line::from(vec![
            Span::styled("Owner: ", theme::DIM),
            owner_span(cfg, w.saturating_sub(7 + 9 + 7)),
            Span::styled("  Token: ", theme::DIM),
            token,
        ]),
        Line::from(vec![
            Span::styled("Last:   ", theme::DIM),
            Span::styled(fit(when, w.saturating_sub(8)), theme::NORMAL),
        ]),
        Line::from(vec![
            Span::styled("Result: ", theme::DIM),
            last_run_result(dash),
        ]),
        Line::from(vec![
            Span::styled("Output: ", theme::DIM),
            Span::styled(output, theme::NORMAL),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_actions(frame: &mut Frame, dash: &DashboardState, area: Rect, bordered: bool) {
    let inner = if bordered {
        let block = Block::default()
            .title(Span::styled(" Actions ", theme::TITLE))
            .borders(Borders::ALL)
            .border_style(theme::DIM);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    } else {
        area
    };

    let items: Vec<ListItem> = DashboardState::ACTIONS
        .iter()
        .enumerate()
        .map(|(i, action)| {
            let is_selected = i == dash.selected_action;
            let prefix = if is_selected { "> " } else { "  " };
            let key = match i {
                0 => "[r]",
                1 => "[c]",
                2 => "[v]",
                3 => "[q]",
                _ => "   ",
            };
            let line = Line::from(vec![
                Span::styled(prefix, theme::ACCENT_STYLE),
                Span::styled(key, theme::KEY_HINT),
                Span::raw(" "),
                Span::styled(
                    *action,
                    if is_selected {
                        theme::ACCENT_BOLD
                    } else {
                        theme::NORMAL
                    },
                ),
            ]);
            let item = ListItem::new(line);
            if is_selected {
                item.style(Style::default().bg(theme::HIGHLIGHT_BG))
            } else {
                item
            }
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(dash.selected_action));

    let list = List::new(items).highlight_style(theme::SELECTED);
    frame.render_stateful_widget(list, inner, &mut state);
}

fn render_hint(frame: &mut Frame, area: Rect) {
    let lines = hint_lines(
        &[
            ("r", "run backup"),
            ("q", "quit"),
            ("c", "configure"),
            ("v", "verify"),
            ("j/k", "move"),
            ("Enter", "select"),
            ("1-5", "screens"),
        ],
        area.width as usize,
        area.height as usize,
    );
    frame.render_widget(Paragraph::new(lines), area);
}
