// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Turning failures into something an operator can act on: secret scrubbing
//! for anything that is about to be printed, and plain-language hints for the
//! failure patterns users hit most.

/// Redacts anything that looks like a GitHub token in `s`.
///
/// Last-line-of-defence — the rest of the codebase already takes care to
/// keep tokens out of error and log strings, but a misbehaving proxy
/// (which can echo a request URL) or an unusual GitHub error body could
/// in principle still surface a token in `--verbose` output.  This
/// scrubber recognises every official GitHub token prefix and replaces
/// the body with `<redacted>` while preserving the prefix so the
/// operator can still tell *what kind* of token it was.
pub(crate) fn redact_secrets(s: &str) -> String {
    // Order matters: `github_pat_` must be checked before `gh*_` so the
    // longer prefix wins.
    const PREFIXES: &[&str] = &["github_pat_", "ghp_", "gho_", "ghu_", "ghs_", "ghr_"];
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while !rest.is_empty() {
        let mut hit: Option<(usize, &'static str)> = None;
        for prefix in PREFIXES {
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

/// Translates a raw error message string into an actionable hint for the
/// user, or returns `None` when no specific advice applies.
///
/// Recognises every common failure pattern: 401/403, expired token,
/// missing scope, rate-limit exhaustion, git binary missing, network
/// timeout, TLS / proxy issues.  The patterns are matched on the
/// `Display` output of `CoreError` / `ClientError`, which is stable
/// because those errors live in our own crates.
pub(crate) fn explain_error(raw: &str) -> Option<&'static str> {
    let r = raw.to_ascii_lowercase();

    // Rate limit / abuse detection.
    if r.contains("rate limit") || r.contains("ratelimit") {
        return Some(
            "GitHub rate-limited the run.  Wait for the printed reset window, \
             use a token with higher limits, or lower --concurrency.",
        );
    }

    // 401 — token wrong or revoked.
    if r.contains("401")
        || r.contains("bad credentials")
        || r.contains("unauthorized")
        || r.contains("requires authentication")
    {
        return Some(
            "Authentication rejected.  Verify GITHUB_TOKEN is set to a current, \
             unrevoked token at https://github.com/settings/tokens.  Run \
             `github-backup --doctor` to confirm the token is reachable.",
        );
    }

    // 403 — usually a missing scope or org-restriction.
    if r.contains("403")
        || r.contains("forbidden")
        || r.contains("resource not accessible")
        || r.contains("must have admin")
    {
        return Some(
            "GitHub refused access.  The token likely lacks a required scope. \
             Run `github-backup --list-scopes` to see what the current flag \
             set needs, then regenerate the token with those scopes.",
        );
    }

    // 404 — wrong target.
    if r.contains("404") || r.contains("not found") {
        return Some(
            "GitHub returned 404.  Check OWNER spelling and capitalisation, and \
             confirm the token has access to that account / org.",
        );
    }

    // Git binary missing.
    if r.contains("could not start git")
        || r.contains("no such file or directory") && r.contains("git")
    {
        return Some(
            "The `git` binary could not be launched.  Install git \
             (https://git-scm.com/downloads) and ensure it is on the PATH, \
             then re-run.",
        );
    }

    // Network failures.
    if r.contains("connection refused")
        || r.contains("dns error")
        || r.contains("tcp connect")
        || r.contains("network is unreachable")
    {
        return Some(
            "Could not reach GitHub.  Check your network connection, DNS, or \
             set HTTPS_PROXY if you are behind a corporate proxy.",
        );
    }

    // TLS issues.
    if r.contains("tls") || r.contains("certificate") {
        return Some(
            "TLS handshake failed.  Update your system's CA bundle \
             (e.g. install ca-certificates), or set HTTPS_PROXY if traffic \
             must traverse a TLS-intercepting proxy.",
        );
    }

    // Disk space / I/O.
    if r.contains("no space left") || r.contains("disk full") {
        return Some(
            "The output disk is full.  Free space or choose a different \
             --output, then re-run; partial progress will resume from the \
             checkpoint.",
        );
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_secrets_replaces_classic_pat() {
        let s = "401 Unauthorized: ghp_abcdef1234567890";
        let out = redact_secrets(s);
        assert!(!out.contains("ghp_abcdef1234567890"));
        assert!(out.contains("ghp_<redacted>"));
    }

    #[test]
    fn redact_secrets_replaces_fine_grained_pat() {
        let s = "url=https://x@github.com?token=github_pat_X9Y8Z7Q";
        let out = redact_secrets(s);
        assert!(!out.contains("github_pat_X9Y8Z7Q"));
        assert!(out.contains("github_pat_<redacted>"));
    }

    #[test]
    fn redact_secrets_replaces_every_known_prefix() {
        for prefix in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
            let raw = format!("token={prefix}xyz123");
            let out = redact_secrets(&raw);
            assert!(
                out.contains(&format!("{prefix}<redacted>")),
                "prefix {prefix:?} not redacted: {out}"
            );
            assert!(!out.contains("xyz123"), "literal body leaked: {out}");
        }
    }

    #[test]
    fn redact_secrets_preserves_text_around_token() {
        let s = "Before: ghp_LEAKED After";
        let out = redact_secrets(s);
        assert_eq!(out, "Before: ghp_<redacted> After");
    }

    #[test]
    fn redact_secrets_handles_text_without_secrets() {
        assert_eq!(redact_secrets("just a regular log"), "just a regular log");
    }

    #[test]
    fn redact_secrets_handles_multiple_tokens() {
        let s = "first ghp_AAA second github_pat_BBB done";
        let out = redact_secrets(s);
        assert!(!out.contains("ghp_AAA"));
        assert!(!out.contains("github_pat_BBB"));
        assert!(out.contains("ghp_<redacted>"));
        assert!(out.contains("github_pat_<redacted>"));
    }

    #[test]
    fn explain_error_recognises_rate_limit() {
        assert!(explain_error("GitHub rate limit exceeded").is_some());
        assert!(explain_error("ratelimit hit").is_some());
    }

    #[test]
    fn explain_error_recognises_401_403_404() {
        assert!(explain_error("status 401 Unauthorized").is_some());
        assert!(explain_error("status 403 Forbidden").is_some());
        assert!(explain_error("status 404 Not Found").is_some());
        assert!(explain_error("Bad credentials").is_some());
        assert!(explain_error("Resource not accessible by integration").is_some());
    }

    #[test]
    fn explain_error_recognises_git_missing() {
        assert!(explain_error("could not start git: ENOENT").is_some());
    }

    #[test]
    fn explain_error_returns_none_for_unknown() {
        assert!(explain_error("an unrelated message").is_none());
    }
}
