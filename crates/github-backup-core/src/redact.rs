// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Scrubbing secrets out of text that is about to leave the process.
//!
//! Failure messages end up on the console, in the JSON report, in the webhook
//! payload and in log files.  The codebase takes care to keep credentials out of
//! error values, but git's stderr and a misbehaving proxy can still echo a URL
//! or header, so everything that is stored or sent is passed through
//! [`secrets`] once, at the point it is recorded.
//!
//! Three kinds of secret are recognised:
//!
//! 1. **Exact values** the caller knows about (the access token in use).
//! 2. **GitHub token formats** — every official prefix, whatever the value.
//! 3. **Credentials embedded in a URL** — `scheme://user:password@host`.

/// Exact values shorter than this are not redacted: a three-character
/// "secret" would corrupt unrelated text and is not worth hiding.
const MIN_EXACT_LEN: usize = 6;

/// GitHub token prefixes, longest first so `github_pat_` wins over `gh*_`.
const GITHUB_PREFIXES: &[&str] = &["github_pat_", "ghp_", "gho_", "ghu_", "ghs_", "ghr_"];

/// Returns `text` with every secret replaced by `<redacted>`.
///
/// `known` lists exact secret values (for example the token the client uses);
/// pass an empty slice when there are none.  The GitHub token *prefix* is kept
/// (`ghp_<redacted>`) so a reader can still tell what kind of credential it was.
#[must_use]
pub fn secrets(text: &str, known: &[&str]) -> String {
    let mut out = text.to_string();
    for value in known {
        if value.len() >= MIN_EXACT_LEN {
            out = out.replace(value, "<redacted>");
        }
    }
    out = github_tokens(&out);
    url_credentials(&out)
}

/// Replaces the body of every GitHub-format token, keeping its prefix.
fn github_tokens(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while !rest.is_empty() {
        let mut hit: Option<(usize, &'static str)> = None;
        for prefix in GITHUB_PREFIXES {
            if let Some(idx) = rest.find(prefix) {
                if hit.map(|(j, _)| idx < j).unwrap_or(true) {
                    hit = Some((idx, prefix));
                }
            }
        }
        match hit {
            Some((idx, prefix)) => {
                out.push_str(&rest[..idx]);
                out.push_str(prefix);
                out.push_str("<redacted>");
                let after = &rest[idx + prefix.len()..];
                // Skip the alphanumeric run that constitutes the token body.
                let body_end = after
                    .char_indices()
                    .find(|(_, c)| !c.is_ascii_alphanumeric() && *c != '_')
                    .map(|(i, _)| i)
                    .unwrap_or(after.len());
                rest = &after[body_end..];
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
    }
    out
}

/// Replaces the `user:password` part of `scheme://user:password@host` URLs.
///
/// Only a userinfo part that contains a `:` is treated as a credential; a bare
/// `ssh://git@github.com/...` user name is left alone because it is not secret
/// and the message would be less useful without it.
fn url_credentials(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find("://") {
        let (head, tail) = rest.split_at(pos + 3);
        out.push_str(head);
        // The authority ends at the first '/', whitespace or quote.
        let authority_end = tail
            .find(|c: char| c == '/' || c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(tail.len());
        let authority = &tail[..authority_end];
        match authority.rfind('@') {
            Some(at) if authority[..at].contains(':') => {
                out.push_str("<redacted>");
                out.push_str(&authority[at..]);
            }
            _ => out.push_str(authority),
        }
        rest = &tail[authority_end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_classic_pat() {
        let out = secrets("401 Unauthorized: ghp_abcdef1234567890", &[]);
        assert!(!out.contains("ghp_abcdef1234567890"));
        assert!(out.contains("ghp_<redacted>"));
    }

    #[test]
    fn replaces_fine_grained_pat() {
        let out = secrets("url=https://x@github.com?token=github_pat_X9Y8Z7Q", &[]);
        assert!(!out.contains("github_pat_X9Y8Z7Q"));
        assert!(out.contains("github_pat_<redacted>"));
    }

    #[test]
    fn replaces_every_known_prefix() {
        for prefix in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
            let out = secrets(&format!("token={prefix}xyz123"), &[]);
            assert!(
                out.contains(&format!("{prefix}<redacted>")),
                "prefix {prefix:?} not redacted: {out}"
            );
            assert!(!out.contains("xyz123"), "literal body leaked: {out}");
        }
    }

    #[test]
    fn preserves_text_around_a_token() {
        assert_eq!(
            secrets("Before: ghp_LEAKED After", &[]),
            "Before: ghp_<redacted> After"
        );
    }

    #[test]
    fn leaves_text_without_secrets_alone() {
        assert_eq!(secrets("just a regular log", &[]), "just a regular log");
    }

    #[test]
    fn handles_multiple_tokens() {
        let out = secrets("first ghp_AAA second github_pat_BBB done", &[]);
        assert!(!out.contains("ghp_AAA") && !out.contains("github_pat_BBB"));
        assert!(out.contains("ghp_<redacted>") && out.contains("github_pat_<redacted>"));
    }

    #[test]
    fn replaces_exact_values_of_any_format() {
        let out = secrets(
            "proxy said: bad token s3cr3t-value-123 rejected",
            &["s3cr3t-value-123"],
        );
        assert!(!out.contains("s3cr3t-value-123"), "{out}");
        assert!(out.contains("<redacted>"));
    }

    #[test]
    fn ignores_exact_values_too_short_to_be_secret() {
        let text = "the cat sat";
        assert_eq!(secrets(text, &["cat"]), text);
        assert_eq!(secrets(text, &[""]), text);
    }

    #[test]
    fn redacts_credentials_embedded_in_a_url() {
        let out = secrets(
            "fatal: unable to access 'https://user:hunter2@git.example.com/o/r.git/': 401",
            &[],
        );
        assert!(!out.contains("hunter2") && !out.contains("user:"), "{out}");
        assert!(
            out.contains("https://<redacted>@git.example.com/o/r.git/"),
            "{out}"
        );
    }

    #[test]
    fn keeps_a_url_user_name_without_a_password() {
        let text = "ssh://git@github.com/o/r.git";
        assert_eq!(secrets(text, &[]), text);
    }

    #[test]
    fn leaves_urls_without_userinfo_untouched() {
        let text = "GET https://api.github.com/repos/o/r/issues?per_page=100 failed";
        assert_eq!(secrets(text, &[]), text);
    }

    #[test]
    fn handles_several_urls_and_trailing_text() {
        let out = secrets("a https://u:p@h1/x b https://h2/y c https://v:w@h3", &[]);
        assert_eq!(
            out,
            "a https://<redacted>@h1/x b https://h2/y c https://<redacted>@h3"
        );
    }
}
