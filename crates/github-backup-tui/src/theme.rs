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

// ── Mode indicator ────────────────────────────────────────────────────────────

/// Status bar label shown when navigating (not editing).
pub const MODE_NAV: Style = Style::new().fg(MUTED);

/// Status bar label shown when editing a text field.
pub const MODE_EDIT: Style = Style::new()
    .fg(Color::Black)
    .bg(ACCENT)
    .add_modifier(Modifier::BOLD);

/// Status bar label shown when log search is active.
pub const MODE_SEARCH: Style = Style::new()
    .fg(Color::Black)
    .bg(WARNING)
    .add_modifier(Modifier::BOLD);

// ── Tab badge styles ──────────────────────────────────────────────────────────

/// Badge on a configure tab that has a validation error.
pub const TAB_ERROR_BADGE: Style = Style::new().fg(ERROR).add_modifier(Modifier::BOLD);

/// Badge on a configure tab that requires attention (env var overriding).
pub const TAB_INFO_BADGE: Style = Style::new().fg(WARNING);

// ── Help overlay ─────────────────────────────────────────────────────────────

/// Background style for the help overlay panel.
pub const HELP_TITLE: Style = Style::new()
    .fg(ACCENT)
    .add_modifier(Modifier::BOLD)
    .add_modifier(Modifier::UNDERLINED);

/// A key name inside the help overlay.
pub const HELP_KEY: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);

/// A description line inside the help overlay.
pub const HELP_DESC: Style = Style::new().fg(FG);

// ── Search ────────────────────────────────────────────────────────────────────

/// Highlighted log line that matches the current search query.
pub const SEARCH_MATCH: Style = Style::new()
    .fg(Color::Black)
    .bg(WARNING)
    .add_modifier(Modifier::BOLD);

/// Active search-input field.
pub const SEARCH_INPUT: Style = Style::new().fg(WARNING).add_modifier(Modifier::BOLD);

// ── Field description ────────────────────────────────────────────────────────

/// Descriptive help text shown below the focused field in Configure.
pub const FIELD_DESC: Style = Style::new().fg(Color::Gray);

/// Inline validation error shown below an invalid field.
pub const FIELD_ERR: Style = Style::new().fg(ERROR);

/// Indicator that a value is coming from an environment variable.
pub const ENV_OVERRIDE: Style = Style::new().fg(SUCCESS).add_modifier(Modifier::ITALIC);

// ── Log level styles ──────────────────────────────────────────────────────────

/// Returns the style appropriate for a tracing log level string.
pub fn log_level_style(level: &str) -> Style {
    match level {
        "ERROR" => ERR_STYLE,
        "WARN" => WARN_STYLE,
        "INFO" => ACCENT_STYLE,
        "DEBUG" => DIM,
        _ => DIM,
    }
}
