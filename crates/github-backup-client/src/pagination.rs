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
    for part in link_header.split(',') {
        let part = part.trim();
        // Each segment looks like: <URL>; rel="relation"
        let Some((url_part, rel_part)) = part.split_once(';') else {
            continue;
        };

        let url_part = url_part.trim();
        let rel_part = rel_part.trim();

        if !rel_part.contains(r#"rel="next""#) {
            continue;
        }

        // Strip the surrounding `<` and `>`
        if url_part.starts_with('<') && url_part.ends_with('>') {
            return Some(url_part[1..url_part.len() - 1].to_string());
        }
    }
    None
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
}
