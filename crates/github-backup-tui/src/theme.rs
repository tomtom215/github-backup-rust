// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Colour palette and style constants used throughout the TUI.

use ratatui::style::{Color, Modifier, Style};

// ── Palette ──────────────────────────────────────────────────────────────────

pub const FG: Color = Color::White;
pub const MUTED: Color = Color::DarkGray;
pub const ACCENT: Color = Color::Cyan;
#[allow(dead_code)]
pub const ACCENT2: Color = Color::Blue;
pub const SUCCESS: Color = Color::Green;
pub const WARNING: Color = Color::Yellow;
pub const ERROR: Color = Color::Red;
pub const HIGHLIGHT_BG: Color = Color::DarkGray;

// ── Base styles ───────────────────────────────────────────────────────────────

/// Normal body text.
pub const NORMAL: Style = Style::new().fg(FG);

/// Muted / secondary text.
pub const DIM: Style = Style::new().fg(MUTED);

/// Accent colour (cyan) — used for borders, selected items, active tabs.
pub const ACCENT_STYLE: Style = Style::new().fg(ACCENT);

/// Bold accent.
pub const ACCENT_BOLD: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);

/// Highlighted row / selection background.
pub const SELECTED: Style = Style::new()
    .bg(HIGHLIGHT_BG)
    .fg(FG)
    .add_modifier(Modifier::BOLD);

/// Success/OK indicator.
pub const OK_STYLE: Style = Style::new().fg(SUCCESS);

/// Warning indicator.
pub const WARN_STYLE: Style = Style::new().fg(WARNING);

/// Error indicator.
pub const ERR_STYLE: Style = Style::new().fg(ERROR);

/// Title in a Block header.
pub const TITLE: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);

/// Keyboard shortcut hint.
pub const KEY_HINT: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);

/// Keyboard description text after the hint.
pub const KEY_DESC: Style = Style::new().fg(MUTED);

/// Active tab label.
pub const TAB_ACTIVE: Style = Style::new()
    .fg(ACCENT)
    .add_modifier(Modifier::BOLD)
    .add_modifier(Modifier::UNDERLINED);

/// Editing cursor / focused input border.
pub const INPUT_FOCUSED: Style = Style::new().fg(ACCENT);

/// Log level styles.
pub fn log_level_style(level: &str) -> Style {
    match level {
        "ERROR" => ERR_STYLE,
        "WARN" => WARN_STYLE,
        "INFO" => ACCENT_STYLE,
        "DEBUG" => DIM,
        _ => DIM,
    }
}
