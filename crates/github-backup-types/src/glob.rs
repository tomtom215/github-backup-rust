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
/// # Complexity
///
/// O(m × n) time and O(m × n) space where m = `pattern.len()` and
/// n = `text.len()`.  The previous recursive implementation had exponential
/// worst-case for patterns with many consecutive `*` wildcards (e.g.
/// `*a*a*a*a*` against a string of `a`s followed by a non-matching char).
/// This iterative DP formulation eliminates that.
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
    let m = pat.len();
    let n = txt.len();

    // dp[i][j] is true iff pat[..i] matches txt[..j].
    let mut dp = vec![vec![false; n + 1]; m + 1];
    dp[0][0] = true;

    // A leading sequence of `*` patterns can match an empty string.
    for i in 1..=m {
        if pat[i - 1] == '*' {
            dp[i][0] = dp[i - 1][0];
        }
    }

    for i in 1..=m {
        for j in 1..=n {
            if pat[i - 1] == '*' {
                // `*` matches zero characters (dp[i-1][j]) or
                // one more character (dp[i][j-1]).
                dp[i][j] = dp[i - 1][j] || dp[i][j - 1];
            } else if pat[i - 1] == '?' || pat[i - 1] == txt[j - 1] {
                dp[i][j] = dp[i - 1][j - 1];
            }
        }
    }

    dp[m][n]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Deterministic unit tests ──────────────────────────────────────────────

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

    /// Adversarial pattern that caused exponential blowup in the old
    /// recursive implementation.  This must complete instantly.
    #[test]
    fn glob_match_adversarial_pattern_completes() {
        // Old implementation: O(2^n); new DP: O(n*m) — finishes in microseconds.
        //
        // Pattern with trailing '*' — the star absorbs the trailing 'b', so
        // this IS a match.
        assert!(glob_match(
            "*a*a*a*a*a*a*a*a*a*a*",
            "aaaaaaaaaaaaaaaaaaaaab"
        ));

        // Without the trailing star the pattern must end on 'a', so the text
        // ending in 'b' must NOT match.
        assert!(!glob_match(
            "*a*a*a*a*a*a*a*a*a*a",
            "aaaaaaaaaaaaaaaaaaaaab"
        ));
    }

    #[test]
    fn glob_match_adversarial_many_stars_matches() {
        assert!(glob_match("*a*a*a*", "aaa"));
        assert!(glob_match("*a*a*a*", "xaxaxax"));
    }

    // ── Property-based tests ──────────────────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        /// `*` alone always matches any text.
        #[test]
        fn prop_star_matches_everything(text in "[a-z0-9_/-]{0,50}") {
            prop_assert!(glob_match("*", &text));
        }

        /// A literal pattern always matches itself (case-insensitively).
        #[test]
        fn prop_exact_match_is_reflexive(text in "[a-z0-9_-]{1,40}") {
            prop_assert!(glob_match(&text, &text));
        }

        /// A literal pattern does not match a different string (unless one is a
        /// prefix/suffix that happens to equal the other — guarded by inequality).
        #[test]
        fn prop_different_literals_dont_match(
            a in "[a-z]{3,20}",
            b in "[a-z]{3,20}",
        ) {
            if a != b {
                prop_assert!(!glob_match(&a, &b));
            }
        }

        /// `**` (double wildcard) matches any text.
        #[test]
        fn prop_double_star_matches_everything(text in "[a-z0-9/_-]{0,50}") {
            prop_assert!(glob_match("**", &text));
        }

        /// Adversarial `*x*x*x*` style patterns terminate quickly for any
        /// reasonable input length.  If this hangs the DP is broken.
        #[test]
        fn prop_adversarial_pattern_terminates(
            text in "[ab]{0,30}",
        ) {
            // Just assert it doesn't hang/panic; correctness tested elsewhere.
            let _ = glob_match("*a*a*a*a*a*", &text);
        }
    }
}
