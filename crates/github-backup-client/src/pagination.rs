// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub API pagination via `Link` response headers.
//!
//! GitHub signals the URL of the next page in the `Link` header:
//!
//! ```text
//! Link: <https://api.github.com/repos?page=2>; rel="next",
//!       <https://api.github.com/repos?page=5>; rel="last"
//! ```
//!
//! [`parse_next_link`] extracts the `rel="next"` URL so the client can
//! follow pages until the header is absent.

/// Parses the `Link` header value and returns the URL with `rel="next"`,
/// or `None` if this is the last page.
///
/// # Example
///
/// ```
/// use github_backup_client::parse_next_link;
///
/// let header = r#"<https://api.github.com/repos?page=2>; rel="next", <https://api.github.com/repos?page=5>; rel="last""#;
/// assert_eq!(
///     parse_next_link(header),
///     Some("https://api.github.com/repos?page=2".to_string())
/// );
/// ```
#[must_use]
pub fn parse_next_link(link_header: &str) -> Option<String> {
    // A link value is `<URI-reference>` followed by `;`-separated parameters
    // (RFC 8288, section 3).  The URI is delimited by the angle brackets, not
    // by commas: a URL such as `?q=a,b` contains commas, so the header is
    // scanned for `<...>` instead of being split on `,`.
    let mut rest = link_header;
    while let Some(open) = rest.find('<') {
        let after = &rest[open + 1..];
        let close = after.find('>')?;
        let url = &after[..close];
        let tail = &after[close + 1..];
        // The parameters run up to the next link value (parameter values
        // never contain `<` in GitHub's headers).
        let params_end = tail.find('<').unwrap_or(tail.len());
        if is_rel_next(&tail[..params_end]) {
            return Some(url.to_string());
        }
        rest = &tail[params_end..];
    }
    None
}

/// Whether the parameters contain `rel="next"` (or `rel=next`, or a
/// space-separated relation list containing `next`, in any case).
fn is_rel_next(params: &str) -> bool {
    params.split(';').any(|param| {
        param.split_once('=').is_some_and(|(name, value)| {
            name.trim().eq_ignore_ascii_case("rel")
                && value
                    .trim()
                    .trim_end_matches(',')
                    .trim()
                    .trim_matches('"')
                    .split_ascii_whitespace()
                    .any(|rel| rel.eq_ignore_ascii_case("next"))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_next_link_returns_next_url_when_present() {
        let header = r#"<https://api.github.com/repos?page=2>; rel="next", <https://api.github.com/repos?page=5>; rel="last""#;
        assert_eq!(
            parse_next_link(header),
            Some("https://api.github.com/repos?page=2".to_string())
        );
    }

    #[test]
    fn parse_next_link_returns_none_on_last_page() {
        let header = r#"<https://api.github.com/repos?page=5>; rel="last""#;
        assert!(parse_next_link(header).is_none());
    }

    #[test]
    fn parse_next_link_returns_none_on_empty_header() {
        assert!(parse_next_link("").is_none());
    }

    #[test]
    fn parse_next_link_handles_first_page_with_prev_and_next() {
        let header = r#"<https://api.github.com/repos?page=1>; rel="prev", <https://api.github.com/repos?page=3>; rel="next""#;
        assert_eq!(
            parse_next_link(header),
            Some("https://api.github.com/repos?page=3".to_string())
        );
    }

    #[test]
    fn parse_next_link_handles_whitespace_variations() {
        let header = r#"  <https://api.github.com/repos?page=2>  ;  rel="next"  "#;
        assert_eq!(
            parse_next_link(header),
            Some("https://api.github.com/repos?page=2".to_string())
        );
    }

    /// Simulates the `get_all_pages` loop with two pages of data and asserts
    /// that results from both pages are concatenated in the correct order.
    ///
    /// This mirrors the logic in `GitHubClient::get_all_pages` without
    /// requiring a real HTTP server: each "page" returns a slice of items and
    /// an optional `Link` header; the loop continues while `parse_next_link`
    /// returns a URL.
    #[test]
    fn multi_page_results_are_concatenated() {
        let pages: &[(&[u32], Option<&str>)] = &[
            (
                &[1, 2, 3],
                Some(
                    r#"<https://api.github.com/items?page=2>; rel="next", <https://api.github.com/items?page=2>; rel="last""#,
                ),
            ),
            (&[4, 5, 6], None),
        ];

        // Replay the pagination loop exactly as `get_all_pages` does it.
        let mut all_items: Vec<u32> = Vec::new();
        let mut page_index = 0usize;
        let mut next_url: Option<String> = Some("https://api.github.com/items?page=1".to_string());

        while let Some(_url) = next_url.take() {
            let (items, link_header) = pages[page_index];
            page_index += 1;
            all_items.extend_from_slice(items);
            next_url = link_header.and_then(parse_next_link);
        }

        assert_eq!(all_items, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(page_index, 2, "exactly two pages should have been fetched");
    }

    // ── Malformed URL guards ──────────────────────────────────────────
    //
    // The function strips the surrounding `<...>` only when *both*
    // sentinels are present.  These tests pin down the `&&` so a
    // mutant that swaps it for `||` is observable: with `||`, the
    // unbracketed/single-bracket URLs below would slice into the
    // string and return a value rather than `None`.

    #[test]
    fn parse_next_link_returns_none_when_url_missing_opening_bracket() {
        let header = r#"https://api.github.com/repos?page=2>; rel="next""#;
        assert!(
            parse_next_link(header).is_none(),
            "URL without leading '<' must not be returned"
        );
    }

    #[test]
    fn parse_next_link_returns_none_when_url_missing_closing_bracket() {
        let header = r#"<https://api.github.com/repos?page=2; rel="next""#;
        assert!(
            parse_next_link(header).is_none(),
            "URL without trailing '>' must not be returned"
        );
    }

    #[test]
    fn parse_next_link_returns_none_when_url_has_no_brackets_at_all() {
        let header = r#"https://api.github.com/repos?page=2; rel="next""#;
        assert!(
            parse_next_link(header).is_none(),
            "unbracketed URL must not be returned"
        );
    }

    /// Three pages with explicit prev/next links on each page.
    #[test]
    fn three_page_chain_is_fully_traversed() {
        let page1_link = r#"<https://api.github.com/repos?page=2>; rel="next", <https://api.github.com/repos?page=3>; rel="last""#;
        let page2_link = r#"<https://api.github.com/repos?page=1>; rel="prev", <https://api.github.com/repos?page=3>; rel="next", <https://api.github.com/repos?page=3>; rel="last""#;

        let pages: &[(&[&str], Option<&str>)] = &[
            (&["repo-a", "repo-b"], Some(page1_link)),
            (&["repo-c", "repo-d"], Some(page2_link)),
            (&["repo-e"], None),
        ];

        let mut all_repos: Vec<&str> = Vec::new();
        let mut page_index = 0usize;
        let mut next_url: Option<String> = Some("https://api.github.com/repos?page=1".to_string());

        while let Some(_url) = next_url.take() {
            let (items, link_header) = pages[page_index];
            page_index += 1;
            all_repos.extend_from_slice(items);
            next_url = link_header.and_then(parse_next_link);
        }

        assert_eq!(
            all_repos,
            vec!["repo-a", "repo-b", "repo-c", "repo-d", "repo-e"]
        );
        assert_eq!(page_index, 3, "all three pages should have been fetched");
    }

    #[test]
    fn a_comma_inside_a_url_does_not_end_the_list() {
        let header = r#"<https://api.github.com/search?q=a,b&page=1>; rel="prev", <https://api.github.com/search?q=a,b&page=3>; rel="next""#;
        assert_eq!(
            parse_next_link(header),
            Some("https://api.github.com/search?q=a,b&page=3".to_string())
        );
    }

    #[test]
    fn rel_may_be_unquoted_in_any_case_or_one_of_several_relations() {
        for header in [
            "<https://h/2>; rel=next",
            "<https://h/2>; REL=\"Next\"",
            "<https://h/2>; title=\"x\"; rel=\"next last\"",
        ] {
            assert_eq!(
                parse_next_link(header),
                Some("https://h/2".to_string()),
                "{header}"
            );
        }
        assert!(parse_next_link("<https://h/2>; rel=\"nextpage\"").is_none());
    }
}
