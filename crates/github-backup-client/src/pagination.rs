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
}
