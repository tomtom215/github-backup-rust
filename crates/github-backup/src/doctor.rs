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

/// Pings the configured API base to confirm network connectivity.
///
/// Uses a fresh hyper client so a misconfigured proxy / firewall is
/// reported as a connectivity failure rather than producing a confusing
/// `Transport` error several seconds later.
pub async fn check_connectivity(api_url: Option<&str>) -> Check {
    let url = api_url
        .map(str::to_string)
        .unwrap_or_else(|| "https://api.github.com".to_string());
    let url_for_log = url.clone();

    // We don't need authentication for a connectivity ping — `GET /` on
    // the root returns 200 with a JSON map of endpoints.
    use bytes::Bytes;
    use http_body_util::Full;
    use hyper::{Method, Request};
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;

    let mut root_store = rustls::RootCertStore::empty();
    let certs = rustls_native_certs::load_native_certs();
    if certs.certs.is_empty() {
        return Check::fail(
            "system TLS roots",
            "no CA certificates found on this host",
            "install your distribution's `ca-certificates` package",
        );
    }
    root_store.add_parsable_certificates(certs.certs);
    let tls = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls)
        .https_or_http()
        .enable_http1()
        .build();
    let client: Client<_, Full<Bytes>> = Client::builder(TokioExecutor::new()).build(https);

    let req = match Request::builder()
        .method(Method::GET)
        .uri(&url)
        .header(
            "User-Agent",
            concat!("github-backup-rust/", env!("CARGO_PKG_VERSION")),
        )
        .body(Full::new(Bytes::new()))
    {
        Ok(r) => r,
        Err(e) => {
            return Check::fail(
                "API connectivity",
                format!("could not build request to {url_for_log}: {e}"),
                "verify the URL is well-formed (https://… )",
            );
        }
    };

    match tokio::time::timeout(Duration::from_secs(10), client.request(req)).await {
        Ok(Ok(resp)) => Check::pass(
            "API connectivity",
            format!("{url_for_log} → HTTP {}", resp.status().as_u16()),
        ),
        Ok(Err(e)) => Check::fail(
            "API connectivity",
            format!("{url_for_log}: {e}"),
            "check your network, firewall, or set HTTPS_PROXY",
        ),
        Err(_) => Check::fail(
            "API connectivity",
            format!("{url_for_log}: timed out after 10s"),
            "check your network or set HTTPS_PROXY for proxied environments",
        ),
    }
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
