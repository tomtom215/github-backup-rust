// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Glob pattern matching used for `--include-repos` / `--exclude-repos` filters.

/// Returns `true` if `text` matches `pattern`.
///
/// Pattern syntax:
/// - `*` matches any sequence of characters (including the empty string).
/// - `?` matches exactly one character.
/// - All other characters are matched literally, case-insensitively.
///
/// # Examples
///
/// ```
/// use github_backup_types::glob::glob_match;
///
/// assert!(glob_match("hello-*", "hello-world"));
/// assert!(glob_match("*test*", "my-test-repo"));
/// assert!(glob_match("repo?", "repos"));
/// assert!(!glob_match("foo", "bar"));
/// assert!(glob_match("*", "anything"));
/// assert!(glob_match("", ""));
/// assert!(!glob_match("", "nonempty"));
/// ```
#[must_use]
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.to_lowercase().chars().collect();
    let txt: Vec<char> = text.to_lowercase().chars().collect();
    glob_match_chars(&pat, &txt)
}

fn glob_match_chars(pat: &[char], txt: &[char]) -> bool {
    match (pat.first(), txt.first()) {
        // Both exhausted → match.
        (None, None) => true,
        // Pattern exhausted but text remains → no match.
        (None, _) => false,
        // Wildcard: match zero chars (skip `*`) or one char (advance txt).
        (Some('*'), _) => {
            glob_match_chars(&pat[1..], txt)
                || (!txt.is_empty() && glob_match_chars(pat, &txt[1..]))
        }
        // `?` matches any single char.
        (Some('?'), Some(_)) => glob_match_chars(&pat[1..], &txt[1..]),
        (Some('?'), None) => false,
        // Literal match.
        (Some(p), Some(t)) if p == t => glob_match_chars(&pat[1..], &txt[1..]),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_match_exact_returns_true() {
        assert!(glob_match("hello", "hello"));
    }

    #[test]
    fn glob_match_exact_wrong_returns_false() {
        assert!(!glob_match("hello", "world"));
    }

    #[test]
    fn glob_match_star_prefix_matches_suffix() {
        assert!(glob_match("*-world", "hello-world"));
    }

    #[test]
    fn glob_match_star_suffix_matches_prefix() {
        assert!(glob_match("hello-*", "hello-world"));
    }

    #[test]
    fn glob_match_star_alone_matches_anything() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", ""));
    }

    #[test]
    fn glob_match_star_star_matches_anything() {
        assert!(glob_match("**", "deep/path/here"));
    }

    #[test]
    fn glob_match_question_mark_matches_single_char() {
        assert!(glob_match("repo?", "repos"));
        assert!(!glob_match("repo?", "repoxx"));
        assert!(!glob_match("repo?", "repo"));
    }

    #[test]
    fn glob_match_case_insensitive() {
        assert!(glob_match("HELLO-*", "hello-world"));
        assert!(glob_match("rust-*", "Rust-Lang"));
    }

    #[test]
    fn glob_match_empty_pattern_matches_empty_text() {
        assert!(glob_match("", ""));
    }

    #[test]
    fn glob_match_empty_pattern_does_not_match_nonempty() {
        assert!(!glob_match("", "text"));
    }

    #[test]
    fn glob_match_middle_wildcard() {
        assert!(glob_match("*test*", "my-test-repo"));
        assert!(glob_match("*test*", "test"));
        assert!(!glob_match("*test*", "none"));
    }

    #[test]
    fn glob_match_consecutive_wildcards() {
        assert!(glob_match("a**b", "ab"));
        assert!(glob_match("a**b", "axyzb"));
    }
}
