// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Verify screen — SHA-256 manifest integrity check.
//!
//! Only the JSON data covered by the manifest is checked; git mirrors are not.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::util::{fit, fit_tail, hint_lines};
use crate::state::{ConfigState, VerifyState, VERIFY_LIST_CAP};
use crate::theme;

pub fn render(frame: &mut Frame, verify: &VerifyState, cfg: &ConfigState, area: Rect) {
    let bordered = area.height >= 12;
    let hint_h: u16 = if area.height >= 14 { 2 } else { 1 };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if bordered { 4 } else { 2 }), // header / path info
            Constraint::Min(0),                               // results
            Constraint::Length(hint_h),                       // hints
        ])
        .split(area);

    render_header(frame, verify, cfg, rows[0], bordered);
    render_results(frame, verify, rows[1], bordered);
    render_hints(frame, verify, rows[2]);
}

fn render_header(
    frame: &mut Frame,
    verify: &VerifyState,
    cfg: &ConfigState,
    area: Rect,
    bordered: bool,
) {
    let inner = if bordered {
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
        inner
    } else {
        area
    };

    let w = inner.width as usize;
    let path = format!("{}/{}/json", cfg.output_dir.trim(), cfg.owner.trim());
    let status = if verify.running {
        Span::styled("Running...", theme::ACCENT_STYLE)
    } else if verify.done {
        if verify.is_clean() {
            Span::styled(
                fit("CLEAN - manifest matches", w.saturating_sub(8)),
                theme::OK_STYLE,
            )
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
            Span::styled(fit_tail(&path, w.saturating_sub(8)), theme::NORMAL),
        ]),
        Line::from(vec![Span::styled("Status: ", theme::DIM), status]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

/// One section of the result list: a heading plus up to the cap entries.
fn push_section(
    lines: &mut Vec<Line<'static>>,
    heading: String,
    heading_style: Style,
    marker: &'static str,
    item_style: Style,
    items: &[String],
    width: usize,
) {
    if items.is_empty() {
        return;
    }
    lines.push(Line::from(Span::styled(heading, heading_style)));
    for f in items.iter().take(VERIFY_LIST_CAP) {
        lines.push(Line::from(vec![
            Span::styled(marker, heading_style),
            Span::styled(fit_tail(f, width.saturating_sub(4)), item_style),
        ]));
    }
    if items.len() > VERIFY_LIST_CAP {
        lines.push(Line::from(Span::styled(
            format!("  ... and {} more", items.len() - VERIFY_LIST_CAP),
            theme::DIM,
        )));
    }
}

fn render_results(frame: &mut Frame, verify: &VerifyState, area: Rect, bordered: bool) {
    let inner = if bordered {
        let block = Block::default()
            .title(Span::styled(" Results ", theme::TITLE))
            .borders(Borders::ALL)
            .border_style(theme::DIM);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    } else {
        area
    };

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

    let w = inner.width as usize;
    let mut lines: Vec<Line<'static>> = vec![Line::from(vec![
        Span::styled("OK:          ", theme::DIM),
        Span::styled(verify.ok.to_string(), theme::OK_STYLE),
        Span::styled(
            " files verified (JSON data only; git mirrors are not covered)",
            theme::DIM,
        ),
    ])];
    push_section(
        &mut lines,
        format!("TAMPERED ({}):", verify.tampered.len()),
        theme::ERR_STYLE,
        "  ! ",
        theme::NORMAL,
        &verify.tampered,
        w,
    );
    push_section(
        &mut lines,
        format!("MISSING ({}):", verify.missing.len()),
        theme::WARN_STYLE,
        "  - ",
        theme::DIM,
        &verify.missing,
        w,
    );
    push_section(
        &mut lines,
        format!("UNEXPECTED ({}):", verify.unexpected.len()),
        theme::DIM,
        "  ? ",
        theme::DIM,
        &verify.unexpected,
        w,
    );

    let visible = inner.height as usize;
    let offset = verify.scroll.min(lines.len().saturating_sub(visible));
    let visible_lines: Vec<Line> = lines.into_iter().skip(offset).take(visible).collect();
    frame.render_widget(Paragraph::new(visible_lines), inner);
}

fn render_hints(frame: &mut Frame, verify: &VerifyState, area: Rect) {
    let lines = if verify.running {
        vec![Line::from(Span::styled(
            "Verification in progress...",
            theme::ACCENT_STYLE,
        ))]
    } else {
        hint_lines(
            &[
                ("v", "start verify"),
                ("Esc/d", "dashboard"),
                ("j/k", "scroll"),
                ("PgUp/PgDn", "page"),
                ("q", "quit"),
            ],
            area.width as usize,
            area.height as usize,
        )
    };
    frame.render_widget(Paragraph::new(lines), area);
}
