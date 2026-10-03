// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Results screen — post-backup summary.
//!
//! The headline follows the engine's own verdict: a run that recorded any
//! failure is shown as INCOMPLETE (never "complete"), and every recorded
//! failure (scope, step, message) is listed in a scrollable panel.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table, Wrap},
    Frame,
};

use super::util::{fit, fit_tail, hint_lines, one_line};
use crate::state::{Outcome, ResultsState};
use crate::theme;

pub fn render(frame: &mut Frame, res: &ResultsState, area: Rect) {
    if res.outcome == Outcome::NotRun {
        render_not_run(frame, area);
        return;
    }

    let compact = area.height < 14;
    let banner_h: u16 = if compact { 2 } else { 3 };
    let hint_h: u16 = if area.height >= 18 { 2 } else { 1 };
    let remaining = area.height.saturating_sub(banner_h + hint_h);

    let has_panel = !res.failures.is_empty()
        || res.error_message.is_some()
        || matches!(res.outcome, Outcome::Cancelled);
    let stats = StatsLayout::choose(remaining, area.width, has_panel);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(banner_h),
            // A run that died early has no meaningful counters to show.
            Constraint::Length(if res.outcome == Outcome::Failed {
                0
            } else {
                stats.height()
            }),
            Constraint::Min(0), // failures / error
            Constraint::Length(hint_h),
        ])
        .split(area);

    render_banner(frame, res, outer[0]);
    render_stats(frame, res, outer[1], stats);
    if outer[2].height > 0 {
        render_detail(frame, res, outer[2]);
    }
    render_hints(frame, res, outer[3]);
}

fn render_not_run(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(Span::styled(
            "No backup has finished in this session.",
            theme::NORMAL,
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Press ", theme::DIM),
            Span::styled("1", theme::KEY_HINT),
            Span::styled(" for the dashboard, then ", theme::DIM),
            Span::styled("r", theme::KEY_HINT),
            Span::styled(" to run one.", theme::DIM),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
}

fn headline(res: &ResultsState) -> (String, Style) {
    let n = res.failures.len();
    match (res.outcome, res.dry_run) {
        (Outcome::Complete, true) => (
            "DRY RUN COMPLETE - nothing was written".into(),
            theme::OK_STYLE,
        ),
        (Outcome::Complete, false) => ("BACKUP COMPLETE".into(), theme::OK_STYLE),
        (Outcome::Incomplete, dry) => {
            let count = if n > 0 {
                format!(" - {n} failure{}", if n == 1 { "" } else { "s" })
            } else {
                format!(
                    " - {} repositor{} failed",
                    res.repos_errored,
                    if res.repos_errored == 1 { "y" } else { "ies" }
                )
            };
            (
                format!(
                    "{}INCOMPLETE{count}",
                    if dry { "DRY RUN " } else { "BACKUP " }
                ),
                theme::ERR_STYLE,
            )
        }
        (Outcome::Failed, _) => ("BACKUP FAILED".into(), theme::ERR_STYLE),
        (Outcome::Cancelled, _) => ("BACKUP CANCELLED".into(), theme::WARN_STYLE),
        (Outcome::NotRun, _) => (String::new(), theme::DIM),
    }
}

fn render_banner(frame: &mut Frame, res: &ResultsState, area: Rect) {
    let w = area.width as usize;
    let (label, style) = headline(res);

    let mut lines = vec![Line::from(Span::styled(
        fit(&label, w),
        style.add_modifier(Modifier::BOLD),
    ))];

    let owner_w = w.saturating_sub(10 + 12 + 12);
    lines.push(Line::from(vec![
        Span::styled("Owner: ", theme::DIM),
        Span::styled(fit(&res.owner, owner_w.max(8)), theme::ACCENT_BOLD),
        Span::styled("  Duration: ", theme::DIM),
        Span::styled(res.elapsed_str(), theme::NORMAL),
    ]));
    if area.height >= 3 {
        lines.push(Line::from(vec![
            Span::styled("Output: ", theme::DIM),
            Span::styled(
                fit_tail(&res.output_dir, w.saturating_sub(8)),
                theme::NORMAL,
            ),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

// ── Statistics ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StatsLayout {
    /// Bordered one-column table, one row per counter.
    Table,
    /// Bordered table with two counters per row.
    TwoColumn,
    /// Two plain lines, no border.
    Summary,
}

impl StatsLayout {
    /// Picks the richest layout that still leaves room for the failure panel.
    fn choose(remaining: u16, width: u16, has_panel: bool) -> Self {
        let panel_min = if has_panel { 6 } else { 0 };
        if remaining >= 10 + panel_min {
            Self::Table
        } else if remaining >= 6 + panel_min && width >= 56 {
            Self::TwoColumn
        } else {
            Self::Summary
        }
    }

    fn height(self) -> u16 {
        match self {
            Self::Table => 10,
            Self::TwoColumn => 6,
            Self::Summary => 2,
        }
    }
}

fn counters(res: &ResultsState) -> Vec<(&'static str, u64, Style)> {
    let errored_style = if res.repos_errored > 0 {
        theme::ERR_STYLE
    } else {
        theme::OK_STYLE
    };
    vec![
        (
            "Repositories discovered",
            res.repos_discovered,
            theme::NORMAL,
        ),
        ("Repositories backed up", res.repos_backed_up, theme::NORMAL),
        ("Repositories skipped", res.repos_skipped, theme::NORMAL),
        ("Repositories failed", res.repos_errored, errored_style),
        ("Gists backed up", res.gists_backed_up, theme::NORMAL),
        ("Issues fetched", res.issues_fetched, theme::NORMAL),
        ("Pull requests fetched", res.prs_fetched, theme::NORMAL),
        ("Workflows fetched", res.workflows_fetched, theme::NORMAL),
    ]
}

fn render_stats(frame: &mut Frame, res: &ResultsState, area: Rect, layout: StatsLayout) {
    if area.height == 0 {
        return;
    }
    // A run that stopped early has no meaningful counters.
    if matches!(res.outcome, Outcome::Failed) {
        return;
    }
    let all = counters(res);

    if layout == StatsLayout::Summary {
        let w = area.width as usize;
        let l1 = format!(
            "repos {} ok, {} skipped, {} failed of {}",
            fmt_n(res.repos_backed_up),
            fmt_n(res.repos_skipped),
            fmt_n(res.repos_errored),
            fmt_n(res.repos_discovered)
        );
        let l2 = format!(
            "gists {}  issues {}  PRs {}  workflows {}",
            fmt_n(res.gists_backed_up),
            fmt_n(res.issues_fetched),
            fmt_n(res.prs_fetched),
            fmt_n(res.workflows_fetched)
        );
        let s1 = if res.repos_errored > 0 {
            theme::ERR_STYLE
        } else {
            theme::NORMAL
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(fit(&l1, w), s1)),
                Line::from(Span::styled(fit(&l2, w), theme::DIM)),
            ]),
            area,
        );
        return;
    }

    let block = Block::default()
        .title(Span::styled(" Statistics ", theme::TITLE))
        .borders(Borders::ALL)
        .border_style(theme::DIM);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows: Vec<Row> = match layout {
        StatsLayout::Table => all
            .iter()
            .map(|(l, v, s)| {
                Row::new(vec![
                    Cell::from(Span::styled((*l).to_string(), theme::DIM)),
                    Cell::from(Span::styled(fmt_n(*v), *s)),
                ])
            })
            .collect(),
        _ => all
            .chunks(2)
            .map(|pair| {
                let mut cells = Vec::new();
                for (l, v, s) in pair {
                    cells.push(Cell::from(Span::styled((*l).to_string(), theme::DIM)));
                    cells.push(Cell::from(Span::styled(fmt_n(*v), *s)));
                }
                Row::new(cells)
            })
            .collect(),
    };
    let widths: Vec<Constraint> = if layout == StatsLayout::Table {
        vec![Constraint::Percentage(50), Constraint::Percentage(50)]
    } else {
        vec![
            Constraint::Percentage(30),
            Constraint::Percentage(20),
            Constraint::Percentage(30),
            Constraint::Percentage(20),
        ]
    };
    frame.render_widget(Table::new(rows, widths).column_spacing(1), inner);
}

fn fmt_n(n: u64) -> String {
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

// ── Failures / error panel ────────────────────────────────────────────────────

fn render_detail(frame: &mut Frame, res: &ResultsState, area: Rect) {
    if !res.failures.is_empty() {
        render_failures(frame, res, area);
        return;
    }

    let (title, style, text) = if let Some(err) = &res.error_message {
        (" Error ", theme::ERR_STYLE, err.clone())
    } else if res.outcome == Outcome::Cancelled {
        (
            " Cancelled ",
            theme::WARN_STYLE,
            "The run was stopped before it finished. Nothing was marked complete; \
             run it again to continue."
                .to_string(),
        )
    } else if res.outcome == Outcome::Incomplete {
        (
            " Failures ",
            theme::ERR_STYLE,
            format!(
                "{} repositor{} did not finish, but no detail was recorded. \
                 Check the log on the Run screen (3).",
                res.repos_errored,
                if res.repos_errored == 1 { "y" } else { "ies" }
            ),
        )
    } else {
        return;
    };

    let block = Block::default()
        .title(Span::styled(title, style.add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL)
        .border_style(style);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(Span::styled(text, theme::NORMAL)).wrap(Wrap { trim: true }),
        inner,
    );
}

fn render_failures(frame: &mut Frame, res: &ResultsState, area: Rect) {
    let n = res.failures.len();
    let sel = res.failure_selected.min(n - 1);
    let title = format!(" Failures ({}/{n}) ", sel + 1);
    let block = Block::default()
        .title(Span::styled(
            fit(&title, (area.width as usize).saturating_sub(2)),
            theme::ERR_STYLE.add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(theme::ERR_STYLE);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width < 8 {
        return;
    }

    // Show the full text of the selected failure underneath when there is room.
    let detail_h: u16 = if inner.height >= 8 { 3 } else { 0 };
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(detail_h)])
        .split(inner);

    let width = parts[0].width as usize;
    let scope_w = (width * 3 / 10).clamp(8, 32).min(width.saturating_sub(4));
    let step_w = res
        .failures
        .iter()
        .map(|f| f.step.chars().count())
        .max()
        .unwrap_or(4)
        .clamp(4, 14)
        .min(width.saturating_sub(scope_w + 6));
    let msg_w = width.saturating_sub(2 + scope_w + 1 + step_w + 1);

    let items: Vec<ListItem> = res
        .failures
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let prefix = if i == sel { "> " } else { "  " };
            ListItem::new(Line::from(vec![
                Span::styled(prefix, theme::ERR_STYLE),
                Span::styled(
                    format!("{:<scope_w$}", fit(&f.scope, scope_w)),
                    theme::ACCENT_STYLE,
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{:<step_w$}", fit(&f.step, step_w)),
                    theme::WARN_STYLE,
                ),
                Span::raw(" "),
                Span::styled(fit(&one_line(&f.message), msg_w), theme::NORMAL),
            ]))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(sel));
    frame.render_stateful_widget(
        List::new(items).highlight_style(theme::SELECTED),
        parts[0],
        &mut state,
    );

    if detail_h > 0 {
        let f = &res.failures[sel];
        let text = format!("{} / {}: {}", f.scope, f.step, one_line(&f.message));
        frame.render_widget(
            Paragraph::new(Span::styled(text, theme::DIM)).wrap(Wrap { trim: true }),
            parts[1],
        );
    }
}

fn render_hints(frame: &mut Frame, res: &ResultsState, area: Rect) {
    let mut items: Vec<(&str, &str)> = vec![("r", "run again"), ("d", "dashboard")];
    if res.failures.len() > 1 {
        items.push(("j/k", "select failure"));
    }
    items.push(("c", "reconfigure"));
    items.push(("q", "quit"));
    if res.failures.len() > 1 {
        items.push(("g/G", "first/last"));
    }
    let lines = hint_lines(&items, area.width as usize, area.height as usize);
    frame.render_widget(Paragraph::new(lines), area);
}
