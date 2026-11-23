// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub REST API endpoint methods, split by resource category.
//!
//! Each submodule adds `impl GitHubClient` methods for one concern:
//!
//! | Module          | Methods                                                  |
//! |-----------------|----------------------------------------------------------|
//! | [`repos`]       | user / org repository lists, one repository              |
//! | [`social`]      | followers, following, starred, watched, gists, gist comments |
//! | [`issues`]      | issues, comments, events, repository-wide comments       |
//! | [`pulls`]       | pull requests, review comments, commits, reviews, compare |
//! | [`repo_meta`]   | labels, milestones, releases, hooks, advisories, branches, protection, rulesets, commit comments |
//! | [`keys`]        | deploy keys, collaborators                               |
//! | [`org`]         | organisation members and teams                           |
//! | [`actions`]     | workflows, workflow runs, deployment environments        |
//! | [`community`]   | Projects (v2) and Packages                               |
//! | [`discussions`] | Discussions, their comments and categories (GraphQL)     |
//! | [`mod@assets`]  | release asset downloads                                  |
//! | [`mod@write`]   | label and milestone creation (used by restore mode)      |

mod actions;
mod assets;
mod community;
mod discussions;
mod issues;
mod keys;
mod org;
mod pulls;
mod repo_meta;
mod repos;
mod social;
mod write;

#[cfg(test)]
mod endpoint_tests;

use github_backup_types::{Page, Raw, Repository};
use serde_json::Value;

/// Percent-encodes `text` for use as one URL path segment: every byte except
/// the RFC 3986 unreserved characters (`A-Z a-z 0-9 - . _ ~`) becomes `%XX`.
///
/// Names that come from GitHub (branch names, package names) may contain `#`,
/// `?`, `%` or spaces; pasted into a URL verbatim they would end the path or
/// change its meaning (`release#2` would request `release`).
pub(crate) fn encode_segment(text: &str) -> String {
    encode(text, false)
}

/// Like [`encode_segment`] but keeps `/`, for the route parameters GitHub
/// declares as spanning several segments (`x-multi-segment`), such as the
/// branch of `/repos/{owner}/{repo}/branches/{branch}/protection`.
pub(crate) fn encode_path(text: &str) -> String {
    encode(text, true)
}

fn encode(text: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~')
            || (keep_slash && byte == b'/')
        {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Drops `temp_clone_token` from every repository object of `page`.
///
/// GitHub documents it as a short-lived read credential to be treated as
/// sensitive; the backup must never write a credential to a file.  Everything
/// else is kept exactly as received.
pub(crate) fn without_clone_tokens(page: Page<Repository>) -> Page<Repository> {
    if page.iter().all(|r| r.json().get(TOKEN_KEY).is_none())
        && page.unparsed().iter().all(|v| v.get(TOKEN_KEY).is_none())
    {
        return page;
    }
    let unparsed = page.unparsed().to_vec();
    let values = page
        .into_items()
        .into_iter()
        .map(Raw::into_json)
        .chain(unparsed)
        .map(strip_clone_token);
    Page::from_values(values)
}

/// [`without_clone_tokens`] for a single repository object.
pub(crate) fn strip_clone_token(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.shift_remove(TOKEN_KEY);
    }
    value
}

const TOKEN_KEY: &str = "temp_clone_token";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn segments_are_percent_encoded() {
        assert_eq!(encode_segment("release#2"), "release%232");
        assert_eq!(encode_segment("a b?c%d"), "a%20b%3Fc%25d");
        assert_eq!(encode_segment("@scope/pkg"), "%40scope%2Fpkg");
        assert_eq!(encode_segment("plain-name_1.0~x"), "plain-name_1.0~x");
        assert_eq!(encode_segment("ünï"), "%C3%BCn%C3%AF");
    }

    #[test]
    fn multi_segment_parameters_keep_their_slashes() {
        assert_eq!(encode_path("feature/x#1"), "feature/x%231");
    }

    #[test]
    fn clone_tokens_are_dropped_and_nothing_else() {
        let repo = |id: u64| {
            json!({
                "id": id, "name": "r", "full_name": "o/r",
                "owner": {"login": "o", "id": 1, "type": "User", "avatar_url": "", "html_url": ""},
                "private": true, "fork": false, "clone_url": "", "ssh_url": "", "html_url": "",
                "temp_clone_token": "SECRET", "size": 1
            })
        };
        let page: Page<Repository> =
            Page::from_values([repo(1), json!({"temp_clone_token": "SECRET2"})]);
        assert_eq!(page.len(), 1);
        let cleaned = without_clone_tokens(page);
        let written = serde_json::to_string(&cleaned).expect("json");
        assert!(!written.contains("SECRET"), "{written}");
        assert_eq!(cleaned.len(), 1);
        assert_eq!(cleaned.unparsed_count(), 1);
        assert_eq!(cleaned[0].json()["size"], 1);
    }
}
