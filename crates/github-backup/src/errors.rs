// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Turning failures into something an operator can act on: secret scrubbing
//! for anything that is about to be printed, and plain-language hints for the
//! failure patterns users hit most.

use std::sync::OnceLock;

/// The secret values this run was configured with (see [`remember_secrets`]).
static KNOWN_SECRETS: OnceLock<Vec<String>> = OnceLock::new();

/// Records the configured secrets — the GitHub token, mirror token, S3 keys,
/// encryption key and webhook URL — so that [`redact_secrets`] removes them by
/// value from anything printed or recorded, whatever form an error takes.
///
/// Call once, after the configuration is complete; later calls are ignored.
pub(crate) fn remember_secrets(args: &crate::cli::Args) {
    let values = [
        &args.token,
        &args.mirror_token,
        &args.s3_access_key,
        &args.s3_secret_key,
        &args.s3_session_token,
        &args.encrypt_key,
        &args.notify_webhook,
    ]
    .into_iter()
    .flatten()
    .map(|v| v.trim().to_string())
    .filter(|v| !v.is_empty())
    .collect();
    let _ = KNOWN_SECRETS.set(values);
}

/// Redacts credentials in `s` before it is printed: the secrets this run was
/// configured with, GitHub and GitLab tokens of every official format, and
/// credentials in URLs.  See [`github_backup_core::redact`].
pub(crate) fn redact_secrets(s: &str) -> String {
    let known: Vec<&str> = KNOWN_SECRETS
        .get()
        .map(|v| v.iter().map(String::as_str).collect())
        .unwrap_or_default();
    github_backup_core::redact::secrets(s, &known)
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
        || r.contains("client error (connect)")
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

    // A response larger than the client accepts.
    if r.contains("response body exceeds") {
        return Some(
            "GitHub sent a response larger than the client accepts.  This is \
             usually a proxy or server error page; check HTTPS_PROXY and --api-url, \
             then re-run.",
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
    fn redact_secrets_delegates_to_the_core_scrubber() {
        let out = redact_secrets("401 ghp_abcdef1234567890 at https://u:pw@h/x");
        assert!(out.contains("ghp_<redacted>"), "{out}");
        assert!(!out.contains("pw@"), "{out}");
    }

    /// A GitHub App's installation tokens change during the run, so they cannot
    /// be remembered by value; their `ghs_` prefix identifies them.
    #[test]
    fn github_app_installation_tokens_are_redacted_by_their_prefix() {
        let out = redact_secrets("clone failed with ghs_16C7e42F292c6912E7710c838347Ae178B4a");
        assert!(out.contains("ghs_<redacted>"), "{out}");
        assert!(!out.contains("16C7e42F"), "{out}");
    }

    /// A mirror token or S3 key has no recognisable format, so only its value
    /// identifies it: post-processing errors must not carry it into the
    /// report, the webhook or the log.  (The only test that sets the
    /// process-wide list.)
    #[test]
    fn configured_secrets_are_redacted_by_value() {
        let args = crate::cli::test_support::parse(&[
            "github-backup",
            "o",
            "--mirror-to",
            "https://codeberg.org",
            "--mirror-token",
            "plain-mirror-secret-42",
            "--s3-bucket",
            "b",
            "--s3-secret-key",
            "S3SECRETKEYVALUE",
        ]);
        remember_secrets(&args);
        let out = redact_secrets(
            "push to https://codeberg.org failed: token plain-mirror-secret-42; s3 S3SECRETKEYVALUE",
        );
        assert!(!out.contains("plain-mirror-secret-42"), "{out}");
        assert!(!out.contains("S3SECRETKEYVALUE"), "{out}");
        assert!(out.contains("https://codeberg.org"), "{out}");
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

    /// hyper reports an unreachable host as "client error (Connect)".
    #[test]
    fn explain_error_recognises_hyper_connect_errors() {
        let hint = explain_error("HTTP error: client error (Connect)").expect("hint");
        assert!(hint.contains("Could not reach"), "{hint}");
    }

    #[test]
    fn explain_error_recognises_an_oversized_response() {
        assert!(explain_error("response body exceeds the 16 MiB cap").is_some());
    }

    #[test]
    fn explain_error_returns_none_for_unknown() {
        assert!(explain_error("an unrelated message").is_none());
    }
}
