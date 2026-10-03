// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Configure screen — tabbed form for the options a TUI run really applies.
//!
//! Layout adapts to the terminal: below 56 columns the tab bar collapses to
//! one line ("Target 2/6"), the categories list becomes a single column when
//! it would not fit in two, and labels shrink so the value being edited is
//! always visible.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs, Wrap},
    Frame,
};

use super::util::{fit, fit_tail, hint_lines};
use crate::state::{CloneTypeForm, ConfigState};
use crate::theme;

/// Narrowest width at which the full bordered tab bar is used.
const WIDE: u16 = 56;

pub fn render(frame: &mut Frame, cfg: &ConfigState, area: Rect) {
    let wide = area.width >= WIDE && area.height >= 12;
    let hint_rows: u16 = if area.height >= 18 { 2 } else { 1 };
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if wide { 3 } else { 1 }), // tab bar
            Constraint::Min(0),                           // tab content
            Constraint::Length(hint_rows.min(area.height.saturating_sub(2))), // key hints
        ])
        .split(area);

    if wide {
        render_tab_bar(frame, cfg, outer[0]);
    } else {
        render_tab_bar_compact(frame, cfg, outer[0]);
    }
    render_tab_content(frame, cfg, outer[1], wide);
    render_hints(frame, cfg, outer[2]);
}

fn render_tab_bar(frame: &mut Frame, cfg: &ConfigState, area: Rect) {
    let titles: Vec<Line> = ConfigState::TAB_NAMES
        .iter()
        .map(|t| Line::from(*t))
        .collect();

    let tabs = Tabs::new(titles)
        .select(cfg.active_tab)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme::DIM)
                .title(Span::styled(" Configure ", theme::TITLE)),
        )
        .highlight_style(theme::TAB_ACTIVE)
        // No padding: all six titles then fit in 54 columns (the bar is only
        // used from 56), and the dividers already carry the spacing.
        .padding("", "")
        .divider(Span::styled(" | ", theme::DIM));

    frame.render_widget(tabs, area);
}

fn render_tab_bar_compact(frame: &mut Frame, cfg: &ConfigState, area: Rect) {
    let name = ConfigState::TAB_NAMES
        .get(cfg.active_tab)
        .copied()
        .unwrap_or("?");
    let line = Line::from(vec![
        Span::styled("Configure: ", theme::TITLE),
        Span::styled(name, theme::TAB_ACTIVE),
        Span::styled(
            format!("  {}/{}", cfg.active_tab + 1, ConfigState::TAB_COUNT),
            theme::DIM,
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_tab_content(frame: &mut Frame, cfg: &ConfigState, area: Rect, bordered: bool) {
    let inner = if bordered {
        let block = Block::default()
            .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
            .border_style(theme::DIM);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    } else {
        area
    };

    match cfg.active_tab {
        0 => render_auth_tab(frame, cfg, inner),
        1 => render_target_tab(frame, cfg, inner),
        2 => render_categories_tab(frame, cfg, inner),
        3 => render_clone_tab(frame, cfg, inner),
        4 => render_filter_tab(frame, cfg, inner),
        5 => render_output_tab(frame, cfg, inner),
        _ => {}
    }
}

fn render_hints(frame: &mut Frame, cfg: &ConfigState, area: Rect) {
    let items: Vec<(&str, &str)> = if cfg.editing {
        vec![
            ("Enter", "save"),
            ("Esc", "cancel"),
            ("Backspace", "delete"),
        ]
    } else {
        let mut v = vec![("s/F5", "start"), ("Esc", "back"), ("Tab", "next tab")];
        v.push(("j/k", "move"));
        v.push(("Space", "toggle"));
        v.push(("Enter", "edit"));
        if cfg.active_tab == ConfigState::TAB_CATEGORIES {
            v.push(("A", "all/none"));
        }
        if cfg.active_tab == 3 && cfg.active_field == 0 {
            v.push(("←/→", "change"));
        }
        v.push(("S-Tab", "prev tab"));
        v
    };
    let lines = hint_lines(&items, area.width as usize, area.height as usize);
    frame.render_widget(Paragraph::new(lines), area);
}

// ── Tab renderers ─────────────────────────────────────────────────────────────

fn render_auth_tab(frame: &mut Frame, cfg: &ConfigState, area: Rect) {
    let fields: Vec<FieldDef> = vec![
        FieldDef::text(0, "GitHub Token", &cfg.token, true, cfg)
            .help("Required. Always shown masked."),
        FieldDef::text(1, "API URL (GHE)", &cfg.api_url, false, cfg)
            .help("GitHub Enterprise API base, https:// only. Empty = github.com."),
    ];
    render_field_list(frame, cfg, &fields, area);
}

fn render_target_tab(frame: &mut Frame, cfg: &ConfigState, area: Rect) {
    let fields: Vec<FieldDef> = vec![
        FieldDef::text(0, "Owner", &cfg.owner, false, cfg).help("User or organisation to back up."),
        FieldDef::text(1, "Output Directory", &cfg.output_dir, false, cfg)
            .help("Backups go to <dir>/<owner>/."),
        FieldDef::toggle(2, "Organisation Mode (--org)", cfg.org_mode, cfg)
            .help("Treat the owner as an organisation."),
        FieldDef::text(3, "Since (date or ISO 8601)", &cfg.since, false, cfg)
            .help("e.g. 2024-01-01. Only fetch items updated after this. Empty = automatic."),
        FieldDef::toggle(4, "Full backup (ignore state)", cfg.full, cfg).help(
            "Ignore the saved incremental state and re-fetch everything, not just what changed.",
        ),
    ];
    render_field_list(frame, cfg, &fields, area);
}

fn category_labels(cfg: &ConfigState) -> Vec<(&'static str, bool)> {
    vec![
        ("Repositories (git clone)", cfg.repositories),
        ("Issues", cfg.issues),
        ("Issue Comments", cfg.issue_comments),
        ("Issue Events", cfg.issue_events),
        ("Pull Requests", cfg.pulls),
        ("PR Comments", cfg.pull_comments),
        ("PR Commits", cfg.pull_commits),
        ("PR Reviews", cfg.pull_reviews),
        ("Labels", cfg.labels),
        ("Milestones", cfg.milestones),
        ("Releases", cfg.releases),
        ("Release Assets", cfg.release_assets),
        ("Hooks (admin)", cfg.hooks),
        ("Security Advisories", cfg.security_advisories),
        ("Wikis", cfg.wikis),
        ("Starred Repos (list)", cfg.starred),
        ("Clone Starred Repos", cfg.clone_starred),
        ("Watched Repos", cfg.watched),
        ("Followers", cfg.followers),
        ("Following", cfg.following),
        ("Gists", cfg.gists),
        ("Starred Gists", cfg.starred_gists),
        ("Topics", cfg.topics),
        ("Branches", cfg.branches),
        ("Deploy Keys (admin)", cfg.deploy_keys),
        ("Collaborators (admin)", cfg.collaborators),
        ("Org Members", cfg.org_members),
        ("Org Teams", cfg.org_teams),
        ("Actions Workflows", cfg.actions),
        ("Action Runs (large)", cfg.action_runs),
        ("Environments", cfg.environments),
        ("Discussions", cfg.discussions),
        ("Projects", cfg.projects),
        ("Packages", cfg.packages),
    ]
}

fn render_categories_tab(frame: &mut Frame, cfg: &ConfigState, area: Rect) {
    let cats = category_labels(cfg);
    let item = |i: usize, label: &str, enabled: bool, width: usize| -> ListItem<'static> {
        let is_sel = i == cfg.active_field;
        let prefix = if is_sel { "> " } else { "  " };
        let label_w = width.saturating_sub(2 + 3 + 1);
        ListItem::new(Line::from(vec![
            Span::styled(prefix, theme::ACCENT_STYLE),
            Span::styled(
                if enabled { "[x]" } else { "[ ]" },
                if enabled { theme::OK_STYLE } else { theme::DIM },
            ),
            Span::raw(" "),
            Span::styled(
                fit(label, label_w),
                if is_sel {
                    theme::ACCENT_BOLD
                } else {
                    theme::NORMAL
                },
            ),
        ]))
    };

    // Two columns need 17 rows and 56 columns; otherwise one scrolling column.
    let two_cols = area.height >= 17 && area.width >= WIDE;
    if !two_cols {
        let items: Vec<ListItem> = cats
            .iter()
            .enumerate()
            .map(|(i, (l, e))| item(i, l, *e, area.width as usize))
            .collect();
        let mut state = ListState::default();
        state.select(Some(cfg.active_field));
        let list = List::new(items).highlight_style(theme::SELECTED);
        frame.render_stateful_widget(list, area, &mut state);
        return;
    }

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    let half = cats.len() / 2 + cats.len() % 2;

    for (col, range) in [(0usize, 0..half), (1usize, half..cats.len())] {
        let width = cols[col].width as usize;
        let items: Vec<ListItem> = range
            .clone()
            .map(|i| item(i, cats[i].0, cats[i].1, width))
            .collect();
        let sel = range
            .contains(&cfg.active_field)
            .then(|| cfg.active_field - range.start);
        let mut state = ListState::default();
        state.select(sel);
        let list = List::new(items).highlight_style(theme::SELECTED);
        frame.render_stateful_widget(list, cols[col], &mut state);
    }
}

fn render_clone_tab(frame: &mut Frame, cfg: &ConfigState, area: Rect) {
    let fields: Vec<FieldDef> = vec![
        FieldDef::select(
            0,
            "Clone Type",
            CloneTypeForm::OPTIONS,
            cfg.clone_type.idx(),
            cfg,
        )
        .help("mirror = full bare mirror (default); shallow = last 10 commits."),
        FieldDef::toggle(1, "Include Forks", cfg.forks, cfg),
        FieldDef::toggle(2, "Include Private", cfg.private, cfg),
        FieldDef::toggle(3, "Git LFS", cfg.lfs, cfg),
        FieldDef::toggle(4, "Prefer SSH", cfg.prefer_ssh, cfg),
        FieldDef::toggle(5, "No Prune", cfg.no_prune, cfg)
            .help("Keep refs that were deleted upstream."),
        FieldDef::text(6, "Concurrency (1-64)", &cfg.concurrency, false, cfg)
            .help("Repositories processed in parallel."),
    ];
    render_field_list(frame, cfg, &fields, area);
}

fn render_filter_tab(frame: &mut Frame, cfg: &ConfigState, area: Rect) {
    let note_h = if area.height >= 7 { 2 } else { 0 };
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(note_h), Constraint::Min(0)])
        .split(area);

    if note_h > 0 {
        let note = Paragraph::new(Line::from(vec![
            Span::styled("Comma-separated glob patterns, e.g. ", theme::DIM),
            Span::styled("rust-*, *-backup", theme::ACCENT_STYLE),
        ]))
        .wrap(Wrap { trim: true });
        frame.render_widget(note, parts[0]);
    }

    let fields: Vec<FieldDef> = vec![
        FieldDef::text(0, "Include Repos (globs)", &cfg.include_repos, false, cfg),
        FieldDef::text(1, "Exclude Repos (globs)", &cfg.exclude_repos, false, cfg),
    ];
    render_field_list(frame, cfg, &fields, parts[1]);
}

fn render_output_tab(frame: &mut Frame, cfg: &ConfigState, area: Rect) {
    let note_h = if area.height >= 9 { 3 } else { 0 };
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(note_h)])
        .split(area);

    let fields: Vec<FieldDef> = vec![
        FieldDef::toggle(0, "Write SHA-256 Manifest", cfg.manifest, cfg)
            .help("After the run, write backup_manifest.json (used by Verify)."),
        FieldDef::toggle(1, "Dry Run (no writes)", cfg.dry_run, cfg)
            .help("List what would be backed up; nothing is written."),
    ];
    render_field_list(frame, cfg, &fields, parts[0]);

    if note_h > 0 {
        let note = Paragraph::new(vec![Line::from(Span::styled(
            "Command line only: mirror push, S3 sync, JSON report, Prometheus \
             metrics, webhook notification and device-flow sign-in.",
            theme::WARN_STYLE,
        ))])
        .wrap(Wrap { trim: true });
        frame.render_widget(note, parts[1]);
    }
}

// ── Generic field list renderer ───────────────────────────────────────────────

enum FieldKind {
    Text {
        value: String,
        masked: bool,
    },
    Toggle {
        value: bool,
    },
    Select {
        options: Vec<&'static str>,
        selected: usize,
    },
}

struct FieldDef {
    index: usize,
    label: &'static str,
    kind: FieldKind,
    help: &'static str,
}

impl FieldDef {
    fn text(
        index: usize,
        label: &'static str,
        value: &str,
        masked: bool,
        cfg: &ConfigState,
    ) -> Self {
        let display = if cfg.editing && cfg.active_field == index {
            cfg.edit_buffer.clone()
        } else {
            value.to_string()
        };
        Self {
            index,
            label,
            kind: FieldKind::Text {
                value: display,
                masked,
            },
            help: "",
        }
    }

    fn toggle(index: usize, label: &'static str, value: bool, _cfg: &ConfigState) -> Self {
        Self {
            index,
            label,
            kind: FieldKind::Toggle { value },
            help: "",
        }
    }

    fn select(
        index: usize,
        label: &'static str,
        options: &'static [&'static str],
        selected: usize,
        _cfg: &ConfigState,
    ) -> Self {
        Self {
            index,
            label,
            kind: FieldKind::Select {
                options: options.to_vec(),
                selected,
            },
            help: "",
        }
    }

    fn help(mut self, help: &'static str) -> Self {
        self.help = help;
        self
    }
}

fn render_field_list(frame: &mut Frame, cfg: &ConfigState, fields: &[FieldDef], area: Rect) {
    // Reserve one row for the help text of the focused field when there is room.
    let help_row = area.height as usize > fields.len() + 1;
    let (list_area, help_area) = if help_row {
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(2)])
            .split(area);
        (parts[0], Some(parts[1]))
    } else {
        (area, None)
    };

    let width = list_area.width as usize;
    // Label column: as wide as the longest label, but never so wide that the
    // value (at least 16 columns) is squeezed out.
    let longest = fields
        .iter()
        .map(|f| f.label.chars().count())
        .max()
        .unwrap_or(0);
    let label_w = longest.min(width.saturating_sub(2 + 2 + 16)).max(8);
    let value_w = width.saturating_sub(2 + label_w + 2);

    let items: Vec<ListItem> = fields
        .iter()
        .map(|f| {
            let is_active = f.index == cfg.active_field;
            let is_editing = is_active && cfg.editing;

            let prefix = if is_active { "> " } else { "  " };

            let value_span = match &f.kind {
                FieldKind::Text { value, masked } => {
                    let shown = if *masked {
                        // Never echo a secret, not even while it is typed.
                        "*".repeat(value.chars().count())
                    } else {
                        value.clone()
                    };
                    let cursor = if is_editing { "_" } else { "" };
                    let inner_w = value_w.saturating_sub(2);
                    let text = if shown.is_empty() && !is_editing {
                        fit("(empty)", inner_w)
                    } else {
                        fit_tail(&format!("{shown}{cursor}"), inner_w)
                    };
                    Span::styled(
                        if shown.is_empty() && !is_editing {
                            text
                        } else {
                            format!("[{text}]")
                        },
                        if is_editing {
                            theme::INPUT_FOCUSED
                        } else if is_active {
                            theme::ACCENT_STYLE
                        } else {
                            theme::DIM
                        },
                    )
                }
                FieldKind::Toggle { value } => Span::styled(
                    if *value { "[x]" } else { "[ ]" },
                    if *value { theme::OK_STYLE } else { theme::DIM },
                ),
                FieldKind::Select { options, selected } => {
                    let left = if is_active { "< " } else { "  " };
                    let right = if is_active { " >" } else { "  " };
                    Span::styled(
                        format!(
                            "{left}{}{right}",
                            options.get(*selected).copied().unwrap_or("?")
                        ),
                        if is_active {
                            theme::ACCENT_STYLE
                        } else {
                            theme::DIM
                        },
                    )
                }
            };

            let line = Line::from(vec![
                Span::styled(prefix, theme::ACCENT_STYLE),
                Span::styled(
                    format!("{:<label_w$}", fit(f.label, label_w)),
                    if is_active {
                        theme::ACCENT_BOLD
                    } else {
                        theme::NORMAL
                    },
                ),
                Span::raw("  "),
                value_span,
            ]);

            ListItem::new(line)
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(cfg.active_field));

    let list = List::new(items).highlight_style(theme::SELECTED);
    frame.render_stateful_widget(list, list_area, &mut state);

    if let Some(help_area) = help_area {
        if let Some(f) = fields.iter().find(|f| f.index == cfg.active_field) {
            if !f.help.is_empty() {
                frame.render_widget(
                    Paragraph::new(Span::styled(f.help, theme::DIM)).wrap(Wrap { trim: true }),
                    help_area,
                );
            }
        }
    }
}
