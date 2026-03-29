// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Verify screen — SHA-256 manifest integrity check.

use ratatui::{
    layout::{Constraint, Direction, Layout},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::state::{ConfigState, VerifyState};
use crate::theme;

pub fn render(
    frame: &mut Frame,
    verify: &VerifyState,
    cfg: &ConfigState,
    area: ratatui::layout::Rect,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // header / path info
            Constraint::Min(0),    // results
            Constraint::Length(2), // hints
        ])
        .split(area);

    render_header(frame, verify, cfg, rows[0]);
    render_results(frame, verify, rows[1]);
    render_hints(frame, verify, rows[2]);
}

fn render_header(
    frame: &mut Frame,
    verify: &VerifyState,
    cfg: &ConfigState,
    area: ratatui::layout::Rect,
) {
    let block = Block::default()
        .title(Span::styled(" Verify Integrity ", theme::TITLE))
        .borders(Borders::ALL)
        .border_style(if verify.running {
            theme::ACCENT_STYLE
        } else {
            theme::DIM
        });

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let path = format!("{}/{}/json", cfg.output_dir, cfg.owner);
    let status = if verify.running {
        Span::styled("Running...", theme::ACCENT_STYLE)
    } else if verify.done {
        if verify.is_clean() {
            Span::styled("CLEAN - all files match", theme::OK_STYLE)
        } else {
            Span::styled(
                format!(
                    "ISSUES FOUND: {} tampered, {} missing",
                    verify.tampered.len(),
                    verify.missing.len()
                ),
                theme::ERR_STYLE,
            )
        }
    } else if verify.error.is_some() {
        Span::styled("ERROR", theme::ERR_STYLE)
    } else {
        Span::styled("Press [v] to start verification", theme::DIM)
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("Path:   ", theme::DIM),
            Span::styled(path, theme::NORMAL),
        ]),
        Line::from(vec![Span::styled("Status: ", theme::DIM), status]),
    ];
    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

fn render_results(frame: &mut Frame, verify: &VerifyState, area: ratatui::layout::Rect) {
    let block = Block::default()
        .title(Span::styled(" Results ", theme::TITLE))
        .borders(Borders::ALL)
        .border_style(theme::DIM);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(ref err) = verify.error {
        let para = Paragraph::new(Line::from(Span::styled(err.clone(), theme::ERR_STYLE)))
            .wrap(Wrap { trim: true });
        frame.render_widget(para, inner);
        return;
    }

    if !verify.done {
        let para = Paragraph::new(Line::from(Span::styled("No results yet.", theme::DIM)));
        frame.render_widget(para, inner);
        return;
    }

    let mut items: Vec<ListItem> = Vec::new();

    items.push(ListItem::new(Line::from(vec![
        Span::styled("OK:          ", theme::DIM),
        Span::styled(verify.ok.to_string(), theme::OK_STYLE),
        Span::styled(" files verified", theme::DIM),
    ])));

    if !verify.tampered.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            format!("TAMPERED ({}):", verify.tampered.len()),
            theme::ERR_STYLE,
        ))));
        for f in verify.tampered.iter().take(50) {
            items.push(ListItem::new(Line::from(vec![
                Span::styled("  ! ", theme::ERR_STYLE),
                Span::styled(f.clone(), theme::NORMAL),
            ])));
        }
    }

    if !verify.missing.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            format!("MISSING ({}):", verify.missing.len()),
            theme::WARN_STYLE,
        ))));
        for f in verify.missing.iter().take(50) {
            items.push(ListItem::new(Line::from(vec![
                Span::styled("  - ", theme::WARN_STYLE),
                Span::styled(f.clone(), theme::DIM),
            ])));
        }
    }

    if !verify.unexpected.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            format!("UNEXPECTED ({}):", verify.unexpected.len()),
            theme::DIM,
        ))));
        for f in verify.unexpected.iter().take(20) {
            items.push(ListItem::new(Line::from(vec![
                Span::styled("  ? ", theme::DIM),
                Span::styled(f.clone(), theme::DIM),
            ])));
        }
    }

    let visible = inner.height as usize;
    let offset = verify.scroll.min(items.len().saturating_sub(visible));
    let visible_items: Vec<ListItem> = items.into_iter().skip(offset).take(visible).collect();

    let list = List::new(visible_items);
    frame.render_widget(list, inner);
}

fn render_hints(frame: &mut Frame, verify: &VerifyState, area: ratatui::layout::Rect) {
    let line = if verify.running {
        Line::from(Span::styled(
            "Verification in progress...",
            theme::ACCENT_STYLE,
        ))
    } else {
        Line::from(vec![
            Span::styled("v", theme::KEY_HINT),
            Span::styled(" start verify  ", theme::KEY_DESC),
            Span::styled("j/k", theme::KEY_HINT),
            Span::styled(" scroll  ", theme::KEY_DESC),
            Span::styled("Esc/d", theme::KEY_HINT),
            Span::styled(" dashboard", theme::KEY_DESC),
        ])
    };

    let para = Paragraph::new(line).wrap(Wrap { trim: true });
    frame.render_widget(para, area);
}
