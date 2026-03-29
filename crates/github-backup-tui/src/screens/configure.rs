// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Configure screen — tabbed form covering every backup option.

use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs, Wrap},
    Frame,
};

use crate::state::{CloneTypeForm, ConfigState, MirrorTypeForm};
use crate::theme;

pub fn render(frame: &mut Frame, cfg: &ConfigState, area: ratatui::layout::Rect) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // tab bar
            Constraint::Min(0),    // tab content
            Constraint::Length(2), // keybinding hints
        ])
        .split(area);

    render_tab_bar(frame, cfg, outer[0]);
    render_tab_content(frame, cfg, outer[1]);
    render_hints(frame, cfg, outer[2]);
}

fn render_tab_bar(frame: &mut Frame, cfg: &ConfigState, area: ratatui::layout::Rect) {
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
        .divider(Span::styled(" | ", theme::DIM));

    frame.render_widget(tabs, area);
}

fn render_tab_content(frame: &mut Frame, cfg: &ConfigState, area: ratatui::layout::Rect) {
    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_style(theme::DIM);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    match cfg.active_tab {
        0 => render_auth_tab(frame, cfg, inner),
        1 => render_target_tab(frame, cfg, inner),
        2 => render_categories_tab(frame, cfg, inner),
        3 => render_clone_tab(frame, cfg, inner),
        4 => render_filter_tab(frame, cfg, inner),
        5 => render_mirror_tab(frame, cfg, inner),
        6 => render_s3_tab(frame, cfg, inner),
        7 => render_output_tab(frame, cfg, inner),
        _ => {}
    }
}

fn render_hints(frame: &mut Frame, cfg: &ConfigState, area: ratatui::layout::Rect) {
    let hint = if cfg.editing {
        Line::from(vec![
            Span::styled("Enter/Esc", theme::KEY_HINT),
            Span::styled(" confirm  ", theme::KEY_DESC),
            Span::styled("Backspace", theme::KEY_HINT),
            Span::styled(" delete char", theme::KEY_DESC),
        ])
    } else {
        Line::from(vec![
            Span::styled("Tab/Shift-Tab", theme::KEY_HINT),
            Span::styled(" switch tab  ", theme::KEY_DESC),
            Span::styled("j/k", theme::KEY_HINT),
            Span::styled(" field  ", theme::KEY_DESC),
            Span::styled("Space", theme::KEY_HINT),
            Span::styled(" toggle  ", theme::KEY_DESC),
            Span::styled("Enter", theme::KEY_HINT),
            Span::styled(" edit  ", theme::KEY_DESC),
            Span::styled("F5/s", theme::KEY_HINT),
            Span::styled(" start  ", theme::KEY_DESC),
            Span::styled("A", theme::KEY_HINT),
            Span::styled(" select all cats  ", theme::KEY_DESC),
            Span::styled("Esc", theme::KEY_HINT),
            Span::styled(" back", theme::KEY_DESC),
        ])
    };

    let para = Paragraph::new(hint).wrap(Wrap { trim: true });
    frame.render_widget(para, area);
}

// ── Tab renderers ─────────────────────────────────────────────────────────────

fn render_auth_tab(frame: &mut Frame, cfg: &ConfigState, area: ratatui::layout::Rect) {
    let fields: Vec<FieldDef> = vec![
        FieldDef::text(0, "GitHub Token", &cfg.token, true, cfg),
        FieldDef::text(1, "API URL (GHE)", &cfg.api_url, false, cfg),
        FieldDef::toggle(2, "Device Auth", cfg.device_auth, cfg),
        FieldDef::text(3, "OAuth Client ID", &cfg.oauth_client_id, false, cfg),
    ];
    render_field_list(frame, cfg, &fields, area);
}

fn render_target_tab(frame: &mut Frame, cfg: &ConfigState, area: ratatui::layout::Rect) {
    let fields: Vec<FieldDef> = vec![
        FieldDef::text(0, "Owner", &cfg.owner, false, cfg),
        FieldDef::text(1, "Output Directory", &cfg.output_dir, false, cfg),
        FieldDef::toggle(2, "Organisation Mode (--org)", cfg.org_mode, cfg),
        FieldDef::text(3, "Since (ISO 8601, incremental)", &cfg.since, false, cfg),
    ];
    render_field_list(frame, cfg, &fields, area);
}

fn render_categories_tab(frame: &mut Frame, cfg: &ConfigState, area: ratatui::layout::Rect) {
    let cats: Vec<(&str, bool)> = vec![
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
    ];

    let items: Vec<ListItem> = cats
        .iter()
        .enumerate()
        .map(|(i, (label, enabled))| {
            let is_sel = i == cfg.active_field;
            let check = if *enabled { "[x]" } else { "[ ]" };
            let prefix = if is_sel { "> " } else { "  " };
            let line = Line::from(vec![
                Span::styled(prefix, theme::ACCENT_STYLE),
                Span::styled(
                    check,
                    if *enabled {
                        theme::OK_STYLE
                    } else {
                        theme::DIM
                    },
                ),
                Span::raw(" "),
                Span::styled(
                    *label,
                    if is_sel {
                        theme::ACCENT_BOLD
                    } else {
                        theme::NORMAL
                    },
                ),
            ]);
            let item = ListItem::new(line);
            if is_sel {
                item.style(Style::default().bg(ratatui::style::Color::DarkGray))
            } else {
                item
            }
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(cfg.active_field));

    // Split into two columns
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let half = items.len() / 2 + items.len() % 2;
    let left_items: Vec<ListItem> = items[..half.min(items.len())].to_vec();
    let right_items: Vec<ListItem> = if items.len() > half {
        items[half..].to_vec()
    } else {
        vec![]
    };

    // Determine which column is active
    let left_sel = if cfg.active_field < half {
        Some(cfg.active_field)
    } else {
        None
    };
    let right_sel = if cfg.active_field >= half {
        Some(cfg.active_field - half)
    } else {
        None
    };

    let mut left_state = ListState::default();
    left_state.select(left_sel);
    let mut right_state = ListState::default();
    right_state.select(right_sel);

    let left_list = List::new(left_items).highlight_style(theme::SELECTED);
    let right_list = List::new(right_items).highlight_style(theme::SELECTED);

    frame.render_stateful_widget(left_list, cols[0], &mut left_state);
    frame.render_stateful_widget(right_list, cols[1], &mut right_state);
}

fn render_clone_tab(frame: &mut Frame, cfg: &ConfigState, area: ratatui::layout::Rect) {
    let fields: Vec<FieldDef> = vec![
        FieldDef::select(
            0,
            "Clone Type",
            CloneTypeForm::OPTIONS,
            cfg.clone_type.idx(),
            cfg,
        ),
        FieldDef::toggle(1, "Include Forks", cfg.forks, cfg),
        FieldDef::toggle(2, "Include Private", cfg.private, cfg),
        FieldDef::toggle(3, "Git LFS", cfg.lfs, cfg),
        FieldDef::toggle(4, "Prefer SSH", cfg.prefer_ssh, cfg),
        FieldDef::toggle(5, "No Prune", cfg.no_prune, cfg),
        FieldDef::text(6, "Concurrency", &cfg.concurrency, false, cfg),
    ];
    render_field_list(frame, cfg, &fields, area);
}

fn render_filter_tab(frame: &mut Frame, cfg: &ConfigState, area: ratatui::layout::Rect) {
    let note_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let note = Paragraph::new(Line::from(vec![
        Span::styled("Comma-separated glob patterns, e.g. ", theme::DIM),
        Span::styled("rust-*, *-backup", theme::ACCENT_STYLE),
    ]))
    .wrap(Wrap { trim: true });
    frame.render_widget(note, note_area[0]);

    let fields: Vec<FieldDef> = vec![
        FieldDef::text(0, "Include Repos (globs)", &cfg.include_repos, false, cfg),
        FieldDef::text(1, "Exclude Repos (globs)", &cfg.exclude_repos, false, cfg),
    ];
    render_field_list(frame, cfg, &fields, note_area[1]);
}

fn render_mirror_tab(frame: &mut Frame, cfg: &ConfigState, area: ratatui::layout::Rect) {
    let fields: Vec<FieldDef> = vec![
        FieldDef::text(0, "Mirror To (URL)", &cfg.mirror_to, false, cfg),
        FieldDef::select(
            1,
            "Mirror Type",
            MirrorTypeForm::OPTIONS,
            cfg.mirror_type.idx(),
            cfg,
        ),
        FieldDef::text(2, "Mirror Token", &cfg.mirror_token, true, cfg),
        FieldDef::text(3, "Mirror Owner", &cfg.mirror_owner, false, cfg),
        FieldDef::toggle(4, "Create Private", cfg.mirror_private, cfg),
    ];
    render_field_list(frame, cfg, &fields, area);
}

fn render_s3_tab(frame: &mut Frame, cfg: &ConfigState, area: ratatui::layout::Rect) {
    let fields: Vec<FieldDef> = vec![
        FieldDef::text(0, "Bucket", &cfg.s3_bucket, false, cfg),
        FieldDef::text(1, "Region", &cfg.s3_region, false, cfg),
        FieldDef::text(2, "Key Prefix", &cfg.s3_prefix, false, cfg),
        FieldDef::text(3, "Endpoint URL (custom)", &cfg.s3_endpoint, false, cfg),
        FieldDef::text(4, "Access Key ID", &cfg.s3_access_key, false, cfg),
        FieldDef::text(5, "Secret Access Key", &cfg.s3_secret_key, true, cfg),
        FieldDef::toggle(6, "Include Release Assets", cfg.s3_include_assets, cfg),
    ];
    render_field_list(frame, cfg, &fields, area);
}

fn render_output_tab(frame: &mut Frame, cfg: &ConfigState, area: ratatui::layout::Rect) {
    let fields: Vec<FieldDef> = vec![
        FieldDef::toggle(0, "Write SHA-256 Manifest", cfg.manifest, cfg),
        FieldDef::toggle(1, "Dry Run (no writes)", cfg.dry_run, cfg),
        FieldDef::text(2, "JSON Report File", &cfg.report, false, cfg),
        FieldDef::text(
            3,
            "Prometheus Metrics File",
            &cfg.prometheus_metrics,
            false,
            cfg,
        ),
        FieldDef::text(4, "Keep Last N Snapshots", &cfg.keep_last, false, cfg),
        FieldDef::text(5, "Max Age (days)", &cfg.max_age_days, false, cfg),
    ];
    render_field_list(frame, cfg, &fields, area);
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
        }
    }

    fn toggle(index: usize, label: &'static str, value: bool, _cfg: &ConfigState) -> Self {
        Self {
            index,
            label,
            kind: FieldKind::Toggle { value },
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
        }
    }
}

fn render_field_list(
    frame: &mut Frame,
    cfg: &ConfigState,
    fields: &[FieldDef],
    area: ratatui::layout::Rect,
) {
    let items: Vec<ListItem> = fields
        .iter()
        .map(|f| {
            let is_active = f.index == cfg.active_field;
            let is_editing = is_active && cfg.editing;

            let prefix = if is_active { "> " } else { "  " };

            let value_span = match &f.kind {
                FieldKind::Text { value, masked } => {
                    let display = if *masked && !is_editing {
                        "*".repeat(value.len().min(16))
                    } else {
                        value.clone()
                    };
                    let cursor = if is_editing { "_" } else { "" };
                    Span::styled(
                        format!("[{display}{cursor}]"),
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
                    format!("{:<28}", f.label),
                    if is_active {
                        theme::ACCENT_BOLD
                    } else {
                        theme::NORMAL
                    },
                ),
                Span::raw("  "),
                value_span,
            ]);

            let item = ListItem::new(line);
            if is_active {
                item.style(Style::default().bg(ratatui::style::Color::DarkGray))
            } else {
                item
            }
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(cfg.active_field));

    let list = List::new(items).highlight_style(theme::SELECTED);
    frame.render_stateful_widget(list, area, &mut state);
}
