// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Pre-flight diagnostics for the `--doctor` and `--check` flags.
//!
//! Designed for non-technical end users who would otherwise have to
//! decipher a 200-line backtrace to discover that `git` was not on the
//! `PATH`.  Every check produces a single line with one of three status
//! prefixes, optionally followed by a remediation hint:
//!
//! ```text
//! ✓  git binary           (version 2.43.0)
//! ✗  github connectivity  cannot resolve api.github.com
//!    → check your network, firewall, or set HTTPS_PROXY
//! ⚠  token scopes         missing read:org for org member backup
//!    → regenerate the token with the read:org scope added
//! ```
//!
//! The mapping is `Pass = 0`, `Warn = 0`, `Fail = 1` for exit-code
//! purposes — warnings do not block the run.

use std::time::Duration;

use github_backup_client::{ClientError, GitHubClient};
use github_backup_types::config::Credential;

use crate::cli::Args;

/// Outcome of a single check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The check succeeded outright.
    Pass,
    /// The check succeeded but the caller might want to know something.
    Warn,
    /// The check failed and the backup will almost certainly fail too.
    Fail,
}

impl Status {
    /// Returns `true` when this status indicates a problem the user
    /// should act on (currently only `Fail`).
    ///
    /// Exposed publicly so other tools can reuse the [`Report`] type
    /// without having to know about the variants directly.
    #[must_use]
    #[allow(dead_code)] // part of the public diagnostic surface
    pub fn is_blocking(self) -> bool {
        matches!(self, Status::Fail)
    }

    fn glyph(self, ansi: bool) -> &'static str {
        if !ansi {
            return match self {
                Status::Pass => "[ ok ]",
                Status::Warn => "[warn]",
                Status::Fail => "[fail]",
            };
        }
        match self {
            // Green check, yellow caution, red cross.
            Status::Pass => "\x1b[32m✓\x1b[0m",
            Status::Warn => "\x1b[33m⚠\x1b[0m",
            Status::Fail => "\x1b[31m✗\x1b[0m",
        }
    }
}

/// A single diagnostic line.
#[derive(Debug, Clone)]
pub struct Check {
    /// Status produced by the check.
    pub status: Status,
    /// Short label identifying the check (left-aligned to a 24-char column).
    pub label: String,
    /// Free-form detail to print after the label.
    pub detail: String,
    /// Optional remediation hint shown indented under the line.
    pub hint: Option<String>,
}

impl Check {
    /// Constructs a passing check.
    pub fn pass(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            status: Status::Pass,
            label: label.into(),
            detail: detail.into(),
            hint: None,
        }
    }

    /// Constructs a warning check (non-blocking).
    pub fn warn(
        label: impl Into<String>,
        detail: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self {
            status: Status::Warn,
            label: label.into(),
            detail: detail.into(),
            hint: Some(hint.into()),
        }
    }

    /// Constructs a failing check (blocking — backup will not start).
    pub fn fail(
        label: impl Into<String>,
        detail: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self {
            status: Status::Fail,
            label: label.into(),
            detail: detail.into(),
            hint: Some(hint.into()),
        }
    }

    /// Renders this check as a single (or multi-line, with hint) string.
    #[must_use]
    pub fn render(&self, ansi: bool) -> String {
        let mut out = format!(
            "{}  {:<24}  {}",
            self.status.glyph(ansi),
            self.label,
            self.detail
        );
        if let Some(ref hint) = self.hint {
            out.push('\n');
            out.push_str("     → ");
            out.push_str(hint);
        }
        out
    }
}

/// Aggregate result of a diagnostic run.
#[derive(Debug, Default)]
pub struct Report {
    /// Ordered list of checks performed.
    pub checks: Vec<Check>,
}

impl Report {
    /// Adds a check to the report.
    pub fn push(&mut self, check: Check) {
        self.checks.push(check);
    }

    /// Returns the number of failing (blocking) checks.
    #[must_use]
    pub fn failures(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.status == Status::Fail)
            .count()
    }

    /// Returns the number of warning checks.
    #[must_use]
    pub fn warnings(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.status == Status::Warn)
            .count()
    }

    /// Renders the full report (one line per check, hints inline).
    #[must_use]
    pub fn render(&self, ansi: bool) -> String {
        self.checks
            .iter()
            .map(|c| c.render(ansi))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// ── Individual checks ────────────────────────────────────────────────────

/// Checks that `git` is installed and at least `MIN_GIT_VERSION`.
///
/// Non-technical users frequently install `github-backup` and discover
/// only at the first clone that `git` was never on their `PATH`.
/// Catching this up-front saves a confusing error several minutes in.
pub fn check_git_binary() -> Check {
    const MIN_MAJOR: u32 = 2;
    const MIN_MINOR: u32 = 20;
    use std::process::Command;
    match Command::new("git").arg("--version").output() {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if let Some((maj, min)) = parse_git_version(&stdout) {
                if (maj, min) < (MIN_MAJOR, MIN_MINOR) {
                    return Check::warn(
                        "git binary",
                        format!("found {stdout}"),
                        format!("recommend git ≥ {MIN_MAJOR}.{MIN_MINOR}"),
                    );
                }
            }
            Check::pass("git binary", stdout)
        }
        Ok(o) => Check::fail(
            "git binary",
            format!("`git --version` exited {}", o.status.code().unwrap_or(-1)),
            "install git from https://git-scm.com/downloads",
        ),
        Err(_) => Check::fail("git binary", "`git` is not on the PATH", git_install_hint()),
    }
}

/// Returns a platform-appropriate hint for installing git.
fn git_install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "install with `brew install git` or from https://git-scm.com/downloads"
    } else if cfg!(target_os = "linux") {
        "install with your package manager: `apt install git` / `dnf install git` / `pacman -S git`"
    } else if cfg!(target_os = "windows") {
        "install from https://git-scm.com/download/win or `winget install Git.Git`"
    } else {
        "install git from https://git-scm.com/downloads"
    }
}

/// Parses `git version 2.43.0` (with optional trailing build info) into
/// `(major, minor)`.  Returns `None` if the output is in an unexpected
/// format.
pub(crate) fn parse_git_version(s: &str) -> Option<(u32, u32)> {
    let rest = s.strip_prefix("git version ")?;
    let mut parts = rest.split(['.', '-']);
    let maj = parts.next()?.parse().ok()?;
    let min = parts.next()?.parse().ok()?;
    Some((maj, min))
}

/// Checks that the output directory exists (or can be created) and is
/// writable by the current user.
pub fn check_output_dir(path: Option<&std::path::Path>) -> Check {
    let Some(dir) = path else {
        return Check::warn(
            "output directory",
            "no --output set; will default to current directory",
            "pass --output <dir> for a stable location",
        );
    };

    if let Err(e) = std::fs::create_dir_all(dir) {
        return Check::fail(
            "output directory",
            format!("cannot create {}: {e}", dir.display()),
            "check permissions on the parent directory",
        );
    }

    let probe = dir.join(".github-backup-doctor-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Check::pass("output directory", dir.display().to_string())
        }
        Err(e) => Check::fail(
            "output directory",
            format!("not writable: {e}"),
            "fix ownership / mount options, or choose a different --output",
        ),
    }
}

/// Inspects the configured credential and classifies it.
pub fn check_credential(args: &Args) -> Check {
    if let Some(ref tok) = args.token {
        let trimmed = tok.trim();
        if trimmed.is_empty() {
            return Check::fail(
                "credential",
                "--token / GITHUB_TOKEN is set but empty",
                "remove the empty value or supply a real token",
            );
        }
        let len = trimmed.len();
        return match token_kind(trimmed) {
            TokenKind::ClassicPat => {
                Check::pass("credential", format!("classic PAT ({len} chars)"))
            }
            TokenKind::FineGrainedPat => {
                Check::pass("credential", format!("fine-grained PAT ({len} chars)"))
            }
            TokenKind::OAuth => Check::pass("credential", format!("OAuth token ({len} chars)")),
            TokenKind::ServerToServer => {
                Check::pass("credential", format!("GitHub App token ({len} chars)"))
            }
            TokenKind::Unknown => Check::warn(
                "credential",
                format!("token does not match a known GitHub prefix ({len} chars)"),
                "expected one of: ghp_, gho_, ghu_, ghs_, ghr_, github_pat_",
            ),
        };
    }
    if args.device_auth {
        return Check::pass("credential", "OAuth device flow");
    }
    Check::warn(
        "credential",
        "no token configured",
        "set GITHUB_TOKEN or pass --token / --device-auth",
    )
}

/// Distinguishes between the recognised GitHub token formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    ClassicPat,
    FineGrainedPat,
    OAuth,
    ServerToServer,
    Unknown,
}

fn token_kind(token: &str) -> TokenKind {
    if token.starts_with("github_pat_") {
        TokenKind::FineGrainedPat
    } else if token.starts_with("ghp_") {
        TokenKind::ClassicPat
    } else if token.starts_with("gho_") {
        TokenKind::OAuth
    } else if token.starts_with("ghu_") || token.starts_with("ghs_") || token.starts_with("ghr_") {
        TokenKind::ServerToServer
    } else {
        TokenKind::Unknown
    }
}

/// How long `--doctor` waits for the API before calling it unreachable.
const API_CHECK_TIMEOUT: Duration = Duration::from_secs(20);

/// Checks the API URL, that the API is reachable (through the proxy
/// environment, exactly as the backup would reach it) and that the credential
/// is accepted.
///
/// This builds the very client the backup uses, so an invalid `--api-url`, a
/// missing CA bundle or a bad proxy shows up here and not minutes into a run.
/// The token is verified with `GET /rate_limit`, which does not consume rate
/// limit: a revoked, expired or mistyped token is a failure, not a pass.
pub async fn check_api(args: &Args) -> Vec<Check> {
    let url = args.api_url.as_deref().unwrap_or("https://api.github.com");
    let token = args
        .token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let credential = match token {
        Some(t) => Credential::Token(t.to_string()),
        None => Credential::Anonymous,
    };
    let client = match GitHubClient::with_api_url(credential, url) {
        Ok(c) => c,
        Err(e) => return vec![client_setup_failure(url, &e)],
    };
    check_api_with(&client, url, token.is_some()).await
}

fn client_setup_failure(url: &str, e: &ClientError) -> Check {
    match e {
        ClientError::InvalidApiUrl(why) => Check::fail(
            "API url",
            format!("{url}: {why}"),
            "use the https:// base URL of the API, e.g. https://ghe.example.com/api/v3",
        ),
        e => Check::fail(
            "system TLS roots",
            e.to_string(),
            "install your distribution's `ca-certificates` package",
        ),
    }
}

/// The checks behind [`check_api`], for an already-built client.
pub(crate) async fn check_api_with(
    client: &GitHubClient,
    url: &str,
    has_token: bool,
) -> Vec<Check> {
    let outcome = match tokio::time::timeout(API_CHECK_TIMEOUT, client.verify_token()).await {
        Ok(r) => r,
        Err(_) => Err(ClientError::Timeout {
            url: url.to_string(),
        }),
    };
    interpret_api_result(url, has_token, outcome)
}

fn interpret_api_result(
    url: &str,
    has_token: bool,
    outcome: Result<Option<u64>, ClientError>,
) -> Vec<Check> {
    let reachable = |detail: String| Check::pass("API connectivity", detail);
    let token_failure = |detail: String, hint: &str| Check::fail("token", detail, hint);
    match outcome {
        Ok(remaining) => {
            let mut checks = vec![reachable(format!("{url} reachable"))];
            if has_token {
                checks.push(Check::pass(
                    "token",
                    match remaining {
                        Some(n) => format!("accepted by GitHub ({n} API requests left)"),
                        None => "accepted by GitHub".to_string(),
                    },
                ));
            }
            checks
        }
        // The server answered, so the network is fine.
        Err(ClientError::ApiError { status: 401, body }) => vec![
            reachable(format!("{url} reachable")),
            token_failure(
                format!("rejected by GitHub (HTTP 401{})", message_suffix(&body)),
                "the token is revoked, expired or mistyped; create a new one at \
                 https://github.com/settings/tokens",
            ),
        ],
        Err(ClientError::ApiError { status: 403, body }) => vec![
            reachable(format!("{url} reachable")),
            token_failure(
                format!("refused by GitHub (HTTP 403{})", message_suffix(&body)),
                "check that the token is allowed to use this API (SSO authorisation, \
                 organisation token policy, IP allow list)",
            ),
        ],
        Err(ClientError::ApiError { status: 404, .. }) => vec![
            reachable(format!("{url} reachable")),
            Check::warn(
                "token",
                "could not be verified: the server has no /rate_limit endpoint",
                "check that --api-url is the API base (…/api/v3 for GitHub Enterprise Server)",
            ),
        ],
        Err(ClientError::RateLimitExceeded { .. }) => vec![
            reachable(format!("{url} reachable")),
            Check::warn(
                "token",
                "GitHub is rate limiting this client",
                "wait a while, then run --doctor again",
            ),
        ],
        Err(e) => vec![Check::fail(
            "API connectivity",
            format!("{url}: {e}"),
            "check your network, firewall, or set HTTPS_PROXY (and NO_PROXY) for proxied environments",
        )],
    }
}

/// `: <message>` from a GitHub error body, or nothing.
fn message_suffix(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v["message"]
                .as_str()
                .map(|m| m.chars().take(80).collect::<String>())
        })
        .map(|m| format!(": {m}"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_git_version_extracts_major_minor() {
        assert_eq!(parse_git_version("git version 2.43.0"), Some((2, 43)));
        assert_eq!(
            parse_git_version("git version 2.39.5 (Apple Git-154)"),
            Some((2, 39))
        );
        assert_eq!(parse_git_version("git version 1.8.3-rc1"), Some((1, 8)));
    }

    #[test]
    fn parse_git_version_rejects_unexpected_output() {
        assert!(parse_git_version("Git for Windows 2.43.0").is_none());
        assert!(parse_git_version("").is_none());
        assert!(parse_git_version("git version foo").is_none());
    }

    #[test]
    fn token_kind_recognises_all_official_prefixes() {
        assert_eq!(token_kind("ghp_abc"), TokenKind::ClassicPat);
        assert_eq!(token_kind("github_pat_abc"), TokenKind::FineGrainedPat);
        assert_eq!(token_kind("gho_abc"), TokenKind::OAuth);
        assert_eq!(token_kind("ghu_abc"), TokenKind::ServerToServer);
        assert_eq!(token_kind("ghs_abc"), TokenKind::ServerToServer);
        assert_eq!(token_kind("ghr_abc"), TokenKind::ServerToServer);
        assert_eq!(token_kind("plain-text"), TokenKind::Unknown);
    }

    #[test]
    fn status_is_blocking_only_for_fail() {
        assert!(!Status::Pass.is_blocking());
        assert!(!Status::Warn.is_blocking());
        assert!(Status::Fail.is_blocking());
    }

    #[test]
    fn check_render_includes_hint_when_present() {
        let c = Check::fail("git binary", "missing", "install git");
        let rendered = c.render(false);
        assert!(rendered.contains("git binary"));
        assert!(rendered.contains("missing"));
        assert!(rendered.contains("install git"));
    }

    #[test]
    fn check_render_omits_hint_when_passing() {
        let c = Check::pass("output directory", "/tmp/x");
        let rendered = c.render(false);
        assert!(
            !rendered.contains("→"),
            "passing checks must not show a hint arrow"
        );
    }

    #[test]
    fn report_failures_warnings_counts() {
        let mut r = Report::default();
        r.push(Check::pass("a", "ok"));
        r.push(Check::warn("b", "soft", "hint"));
        r.push(Check::fail("c", "bad", "fix"));
        r.push(Check::fail("d", "bad", "fix"));
        assert_eq!(r.failures(), 2);
        assert_eq!(r.warnings(), 1);
    }

    #[test]
    fn check_output_dir_returns_warn_when_no_path() {
        let c = check_output_dir(None);
        assert_eq!(c.status, Status::Warn);
    }

    #[test]
    fn check_output_dir_passes_for_writable_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let c = check_output_dir(Some(dir.path()));
        assert_eq!(c.status, Status::Pass, "{}", c.render(false));
        // Ensure probe file was cleaned up.
        assert!(!dir.path().join(".github-backup-doctor-probe").exists());
    }
}

#[cfg(test)]
#[path = "doctor_api_tests.rs"]
mod api_tests;
