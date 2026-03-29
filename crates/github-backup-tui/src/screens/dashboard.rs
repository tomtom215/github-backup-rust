// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Dashboard screen — the first screen a user sees.
//!
//! Shows the last backup summary and quick-action menu.

use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::state::{ConfigState, DashboardState};
use crate::theme;

pub fn render(
    frame: &mut Frame,
    dash: &DashboardState,
    cfg: &ConfigState,
    area: ratatui::layout::Rect,
) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),  // info panel
            Constraint::Min(8),     // actions
            Constraint::Length(3),  // status / hint
        ])
        .split(area);

    render_info(frame, dash, cfg, outer[0]);
    render_actions(frame, dash, outer[1]);
    render_hint(frame, dash, outer[2]);
}

fn render_info(
    frame: &mut Frame,
    dash: &DashboardState,
    cfg: &ConfigState,
    area: ratatui::layout::Rect,
) {
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

    // Left column
    let owner_display = if cfg.owner.is_empty() {
        Span::styled("(not configured)", theme::WARN_STYLE)
    } else {
        Span::styled(cfg.owner.clone(), theme::ACCENT_BOLD)
    };

    let left_lines = vec![
        Line::from(vec![
            Span::styled("Owner:  ", theme::DIM),
            owner_display,
        ]),
        Line::from(vec![
            Span::styled("Output: ", theme::DIM),
            Span::styled(
                if cfg.output_dir.is_empty() { "(not set)" } else { &cfg.output_dir },
                theme::NORMAL,
            ),
        ]),
        Line::from(vec![
            Span::styled("Token:  ", theme::DIM),
            if cfg.token.is_empty() {
                Span::styled("not set", theme::WARN_STYLE)
            } else {
                Span::styled("configured", theme::OK_STYLE)
            },
        ]),
    ];
    let left = Paragraph::new(left_lines);
    frame.render_widget(left, cols[0]);

    // Right column
    let last_run = dash
        .last_backup_time
        .as_deref()
        .unwrap_or("never");
    let last_repos = dash
        .last_backup_repos
        .map(|n| n.to_string())
        .unwrap_or_else(|| "-".into());

    let right_lines = vec![
        Line::from(vec![
            Span::styled("Last run:   ", theme::DIM),
            Span::styled(last_run, theme::NORMAL),
        ]),
        Line::from(vec![
            Span::styled("Repos:      ", theme::DIM),
            Span::styled(last_repos, theme::NORMAL),
        ]),
        Line::from(vec![
            Span::styled("Version:    ", theme::DIM),
            Span::styled(
                dash.last_tool_version.as_deref().unwrap_or("-"),
                theme::DIM,
            ),
        ]),
    ];
    let right = Paragraph::new(right_lines);
    frame.render_widget(right, cols[1]);
}

fn render_actions(frame: &mut Frame, dash: &DashboardState, area: ratatui::layout::Rect) {
    let block = Block::default()
        .title(Span::styled(" Actions ", theme::TITLE))
        .borders(Borders::ALL)
        .border_style(theme::DIM);

    let inner = block.inner(area);
    frame.render_widget(block, area);

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
                Span::styled(*action, if is_selected { theme::ACCENT_BOLD } else { theme::NORMAL }),
            ]);
            let item = ListItem::new(line);
            if is_selected {
                item.style(Style::default().bg(ratatui::style::Color::DarkGray))
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

fn render_hint(frame: &mut Frame, dash: &DashboardState, area: ratatui::layout::Rect) {
    let msg = if let Some(ref err) = dash.error_message {
        Line::from(vec![
            Span::styled("Error: ", theme::ERR_STYLE),
            Span::styled(err.as_str(), theme::ERR_STYLE),
        ])
    } else if let Some(ref status) = dash.status_message {
        Line::from(Span::styled(status.as_str(), theme::OK_STYLE))
    } else {
        Line::from(vec![
            Span::styled("j/k", theme::KEY_HINT),
            Span::styled(" navigate  ", theme::KEY_DESC),
            Span::styled("Enter", theme::KEY_HINT),
            Span::styled(" select  ", theme::KEY_DESC),
            Span::styled("r", theme::KEY_HINT),
            Span::styled(" run backup  ", theme::KEY_DESC),
            Span::styled("c", theme::KEY_HINT),
            Span::styled(" configure  ", theme::KEY_DESC),
            Span::styled("q", theme::KEY_HINT),
            Span::styled(" quit", theme::KEY_DESC),
        ])
    };

    let para = Paragraph::new(msg)
        .block(Block::default().borders(Borders::NONE))
        .wrap(Wrap { trim: true });
    frame.render_widget(para, area);
}
