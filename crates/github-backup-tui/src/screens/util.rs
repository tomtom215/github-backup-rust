// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Width-aware text helpers shared by every screen.
//!
//! Nothing here wraps: text that does not fit is cut with an ellipsis so one
//! logical row is always exactly one terminal row and a long value can never
//! push other rows out of view.

use ratatui::text::{Line, Span};

use crate::theme;

/// Cuts `s` to at most `width` columns, ending in `…` when something was cut.
pub fn fit(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n <= width {
        return s.to_string();
    }
    match width {
        0 => String::new(),
        1 => "…".to_string(),
        _ => {
            let mut out: String = s.chars().take(width - 1).collect();
            out.push('…');
            out
        }
    }
}

/// Like [`fit`] but keeps the END of the string (`…tail`): the part of a path
/// or of a value being typed that matters most.
pub fn fit_tail(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n <= width {
        return s.to_string();
    }
    match width {
        0 => String::new(),
        1 => "…".to_string(),
        _ => {
            let tail: String = s.chars().skip(n - (width - 1)).collect();
            format!("…{tail}")
        }
    }
}

/// Replaces line breaks so a multi-line message stays on one row.
pub fn one_line(s: &str) -> String {
    s.split(['\n', '\r'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" / ")
}

/// Lays key hints out over at most `rows` rows of `width` columns, in the
/// order given.  Hints that do not fit are dropped from the end, so the most
/// important ones go first.
pub fn hint_lines(items: &[(&str, &str)], width: usize, rows: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;

    for (key, desc) in items {
        let w = key.chars().count() + 1 + desc.chars().count();
        let sep = if used == 0 { 0 } else { 2 };
        if used + sep + w > width {
            if spans.is_empty() {
                // A single hint wider than a whole row: show what fits of it.
                spans.push(Span::styled(fit(key, width), theme::KEY_HINT));
            }
            if lines.len() + 1 >= rows {
                break;
            }
            lines.push(Line::from(std::mem::take(&mut spans)));
            used = 0;
            if w > width {
                continue;
            }
        }
        if used > 0 {
            spans.push(Span::raw("  "));
            used += 2;
        }
        spans.push(Span::styled((*key).to_string(), theme::KEY_HINT));
        spans.push(Span::styled(format!(" {desc}"), theme::KEY_DESC));
        used += w;
    }
    if !spans.is_empty() && lines.len() < rows {
        lines.push(Line::from(spans));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_cuts_with_ellipsis() {
        assert_eq!(fit("abcdef", 6), "abcdef");
        assert_eq!(fit("abcdef", 5), "abcd…");
        assert_eq!(fit("abcdef", 1), "…");
        assert_eq!(fit("abcdef", 0), "");
    }

    #[test]
    fn fit_is_char_safe() {
        assert_eq!(fit("héllo日本語🙂x", 4), "hél…");
        assert_eq!(fit_tail("héllo日本語🙂x", 4), "…語🙂x");
    }

    #[test]
    fn fit_tail_keeps_the_end() {
        assert_eq!(fit_tail("/a/very/long/path", 8), "…ng/path");
        assert_eq!(fit_tail("short", 8), "short");
    }

    #[test]
    fn one_line_joins_lines() {
        assert_eq!(one_line("a\nb\r\n\nc"), "a / b / c");
    }

    #[test]
    fn hint_lines_drop_what_does_not_fit() {
        let items = [("a", "one"), ("b", "two"), ("c", "three")];
        let one = hint_lines(&items, 12, 1);
        assert_eq!(one.len(), 1);
        let text: String = one[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "a one  b two");
        assert!(one[0].width() <= 12);
        let two = hint_lines(&items, 12, 2);
        assert_eq!(two.len(), 2);
        assert!(hint_lines(&items, 80, 3).len() == 1);
    }
}
