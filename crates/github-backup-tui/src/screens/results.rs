// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Results screen — post-backup summary.

use ratatui::{
    layout::{Constraint, Direction, Layout},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Table, Row, Cell, Wrap},
    Frame,
};

use crate::state::ResultsState;
use crate::theme;

pub fn render(frame: &mut Frame, res: &ResultsState, area: ratatui::layout::Rect) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // status header
            Constraint::Min(0),    // stats table
            Constraint::Length(3), // hints
        ])
        .split(area);

    render_status(frame, res, outer[0]);
    render_stats(frame, res, outer[1]);
    render_hints(frame, outer[2]);
}

fn render_status(frame: &mut Frame, res: &ResultsState, area: ratatui::layout::Rect) {
    let (label, style) = if res.success {
        ("COMPLETE", theme::OK_STYLE)
    } else {
        ("FAILED", theme::ERR_STYLE)
    };

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let status_line = Line::from(vec![
        Span::styled("Backup ", theme::DIM),
        Span::styled(label, style.add_modifier(ratatui::style::Modifier::BOLD)),
    ]);

    let duration_line = Line::from(vec![
        Span::styled("Duration: ", theme::DIM),
        Span::styled(res.elapsed_str(), theme::NORMAL),
    ]);

    let owner_line = Line::from(vec![
        Span::styled("Owner:    ", theme::DIM),
        Span::styled(res.owner.clone(), theme::ACCENT_BOLD),
    ]);

    let output_line = Line::from(vec![
        Span::styled("Output:   ", theme::DIM),
        Span::styled(res.output_dir.clone(), theme::NORMAL),
    ]);

    let left_block = Block::default().borders(Borders::NONE);
    let right_block = Block::default().borders(Borders::NONE);

    let left = Paragraph::new(vec![status_line, owner_line]).block(left_block);
    let right = Paragraph::new(vec![duration_line, output_line]).block(right_block);

    frame.render_widget(left, cols[0]);
    frame.render_widget(right, cols[1]);
}

fn render_stats(frame: &mut Frame, res: &ResultsState, area: ratatui::layout::Rect) {
    let block = Block::default()
        .title(Span::styled(" Statistics ", theme::TITLE))
        .borders(Borders::ALL)
        .border_style(theme::DIM);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(ref err) = res.error_message {
        let para = Paragraph::new(Line::from(vec![
            Span::styled("Error: ", theme::ERR_STYLE),
            Span::styled(err.clone(), theme::ERR_STYLE),
        ]))
        .wrap(Wrap { trim: true });
        frame.render_widget(para, inner);
        return;
    }

    let rows = vec![
        stat_row("Repositories discovered", res.repos_discovered),
        stat_row("Repositories backed up", res.repos_backed_up),
        stat_row("Repositories skipped", res.repos_skipped),
        stat_row_styled(
            "Repositories errored",
            res.repos_errored,
            if res.repos_errored > 0 { theme::ERR_STYLE } else { theme::OK_STYLE },
        ),
        stat_row("Gists backed up", res.gists_backed_up),
        stat_row("Issues fetched", res.issues_fetched),
        stat_row("Pull requests fetched", res.prs_fetched),
        stat_row("Workflows fetched", res.workflows_fetched),
        stat_row("Discussions fetched", res.discussions_fetched),
    ];

    let widths = [Constraint::Percentage(50), Constraint::Percentage(50)];
    let table = Table::new(rows, widths)
        .column_spacing(2);
    frame.render_widget(table, inner);
}

fn stat_row(label: &str, value: u64) -> Row<'static> {
    Row::new(vec![
        Cell::from(Span::styled(label.to_string(), theme::DIM)),
        Cell::from(Span::styled(
            format_number(value),
            theme::NORMAL,
        )),
    ])
}

fn stat_row_styled(label: &str, value: u64, style: ratatui::style::Style) -> Row<'static> {
    Row::new(vec![
        Cell::from(Span::styled(label.to_string(), theme::DIM)),
        Cell::from(Span::styled(format_number(value), style)),
    ])
}

fn format_number(n: u64) -> String {
    // Simple thousands-separator formatting.
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn render_hints(frame: &mut Frame, area: ratatui::layout::Rect) {
    let line = Line::from(vec![
        Span::styled("r", theme::KEY_HINT),
        Span::styled(" run again  ", theme::KEY_DESC),
        Span::styled("d", theme::KEY_HINT),
        Span::styled(" dashboard  ", theme::KEY_DESC),
        Span::styled("c", theme::KEY_HINT),
        Span::styled(" reconfigure  ", theme::KEY_DESC),
        Span::styled("q", theme::KEY_HINT),
        Span::styled(" quit", theme::KEY_DESC),
    ]);

    let para = Paragraph::new(line)
        .block(Block::default().borders(Borders::TOP).border_style(theme::DIM));
    frame.render_widget(para, area);
}
