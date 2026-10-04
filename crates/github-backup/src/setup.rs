// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Process start-up helpers: logging, the embedded config template, config-file
//! permission checks, completion detection, owner validation and credentials.

use clap_complete::Shell;
use tracing::{info, warn};

use github_backup_client::oauth::device_flow;
use github_backup_types::config::Credential;

use crate::cli::Args;

/// Initialises the `tracing` subscriber.
///
/// Respects the standard observability conventions:
/// - `RUST_LOG` overrides the level filter when set;
/// - `NO_COLOR` (non-empty, per <https://no-color.org>) disables ANSI;
/// - `CLICOLOR_FORCE=1` forces colour even when stderr is not a TTY.
///
/// When stderr is not a TTY (e.g. a log file, CI, journald) we default to
/// no colour so the file contains plain UTF-8 — anyone who wants colour back
/// can set `CLICOLOR_FORCE=1`.
pub(crate) fn init_tracing(quiet: bool, verbose: u8) {
    use std::io::IsTerminal as _;
    use tracing_subscriber::{fmt, EnvFilter};

    let level = if quiet {
        "error"
    } else {
        match verbose {
            0 => "info",
            1 => "debug",
            _ => "trace",
        }
    };

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    let no_color = crate::ui::no_color_env_set();
    let force_color = std::env::var("CLICOLOR_FORCE")
        .map(|v| v == "1")
        .unwrap_or(false);
    let ansi = !no_color && (force_color || std::io::stderr().is_terminal());

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(ansi)
        .with_writer(std::io::stderr)
        .init();
}

/// Returns an annotated TOML configuration template.
///
/// All entries are commented out so an unedited template parses as the
/// default configuration.  Lines marked `# REQUIRED` flag the minimum
/// fields a working config typically needs.
pub(crate) fn config_template() -> &'static str {
    include_str!("config_template.toml")
}

/// Checks config file permissions and warns if it is group- or world-readable.
///
/// A config file commonly contains `token`, `s3_access_key`, or
/// `s3_secret_key`.  If the file is readable by other users, those credentials
/// are exposed.
pub(crate) fn check_config_permissions(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if let Ok(meta) = std::fs::metadata(path) {
            let mode = meta.mode();
            if mode & 0o077 != 0 {
                warn!(
                    path = %path.display(),
                    mode = format!("{:o}", mode & 0o777),
                    "config file is readable by group or others; credentials stored \
                     in it may be exposed. Run: chmod 600 {}",
                    path.display()
                );
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path; // permission checks not supported on this platform
    }
}

/// Checks raw args for `--completions <shell>` before clap parses them,
/// returning the requested [`Shell`] if found.
pub(crate) fn detect_completions_request() -> Option<Shell> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--completions" {
            return args.next().and_then(|s| s.parse().ok());
        }
    }
    None
}

/// Validates a GitHub owner / organisation name as a safe path segment.
///
/// Rejects anything that could escape the output directory or break path
/// construction across operating systems.  Mirrors GitHub's own rules
/// (alphanumerics and hyphens, no leading/trailing hyphens, 1–39 chars)
/// but is intentionally a little more permissive on length so that future
/// GitHub policy changes do not break this client.
///
/// The function never tries to be the authoritative "is this a real GitHub
/// account" check — that responsibility belongs to the API server.  Its
/// only job is to refuse traversal payloads (`..`, `/`, `\`, NUL) and
/// other typos that would manifest as confusing later errors.
pub(crate) fn validate_owner_name(owner: &str) -> Result<(), &'static str> {
    if owner.is_empty() {
        return Err("name is empty");
    }
    if owner.len() > 100 {
        return Err("name is longer than 100 characters");
    }
    if owner == "." || owner == ".." {
        return Err("name must not be '.' or '..'");
    }
    for c in owner.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => {}
            '/' | '\\' => return Err("name must not contain path separators"),
            '\0' => return Err("name must not contain a NUL byte"),
            _ if c.is_control() => return Err("name must not contain control characters"),
            _ => return Err("name contains an unsupported character"),
        }
    }
    Ok(())
}

/// Resolves the GitHub credential from CLI args.
///
/// Returns a [`Credential::Token`] (PAT or OAuth), or
/// [`Credential::Anonymous`] when no auth method is provided.
pub(crate) async fn obtain_credential(args: &Args) -> Result<Credential, String> {
    if let Some(token) = &args.token {
        return Ok(Credential::Token(token.clone()));
    }

    if args.device_auth {
        let client_id = args
            .oauth_client_id
            .as_deref()
            .ok_or_else(|| "--oauth-client-id is required when using --device-auth".to_string())?;

        info!("starting OAuth device flow");
        let scope = args.oauth_scopes.as_str();

        let token = device_flow(client_id, scope, |code, url| {
            eprintln!();
            eprintln!("──────────────────────────────────────────────────────");
            eprintln!("  GitHub OAuth device authorisation");
            eprintln!("──────────────────────────────────────────────────────");
            eprintln!("  1. Open:  {url}");
            eprintln!("  2. Enter: {code}");
            eprintln!("──────────────────────────────────────────────────────");
            eprintln!("  Waiting for authorisation…");
            eprintln!();
        })
        .await
        .map_err(|e| e.to_string())?;

        return Ok(Credential::Token(token));
    }

    Ok(Credential::Anonymous)
}

#[cfg(test)]
mod tests {
    use super::*;
    use github_backup_types::config::ConfigFile;

    #[test]
    fn config_template_is_valid_toml() {
        let toml = config_template();
        ConfigFile::from_toml_str(toml).expect("embedded template must parse as ConfigFile");
    }

    #[test]
    fn config_template_unedited_parses_to_defaults() {
        // An untouched template (all keys commented out) should yield the
        // default `ConfigFile` — i.e. every Option<…> is `None`.  This guards
        // against an accidental uncommented line shipping with the binary.
        let cfg = ConfigFile::from_toml_str(config_template()).expect("parse");
        assert!(cfg.owner.is_none(), "untouched template must not set owner");
        assert!(cfg.token.is_none(), "untouched template must not set token");
        assert!(
            cfg.output.is_none(),
            "untouched template must not set output"
        );
        assert!(cfg.all.is_none(), "untouched template must not set all");
    }

    #[test]
    fn config_template_documents_required_fields() {
        let toml = config_template();
        // Defensive: any key marked REQUIRED in the README + docs must
        // still be present in the template, otherwise onboarding silently
        // regresses.
        assert!(
            toml.contains("REQUIRED — the GitHub user"),
            "template must flag owner as REQUIRED"
        );
        // `output` is optional (it defaults to the current directory), and the
        // template must not claim otherwise.
        assert!(
            !toml.contains("REQUIRED — root directory"),
            "output is optional and must not be flagged REQUIRED"
        );
        assert!(
            toml.contains("default: the current directory"),
            "template must state the default for output"
        );
    }

    #[test]
    fn validate_owner_accepts_realistic_github_names() {
        for ok in [
            "octocat",
            "GitHub",
            "tom-tom215",
            "a",
            "rust-lang",
            "github-actions",
            "user_with_underscore",
            "ORG-Name42",
        ] {
            assert!(
                validate_owner_name(ok).is_ok(),
                "{ok:?} should pass validation"
            );
        }
    }

    #[test]
    fn validate_owner_rejects_path_traversal_attempts() {
        for bad in ["..", ".", "../etc", "foo/bar", "foo\\bar", "/abs", "a/b"] {
            assert!(
                validate_owner_name(bad).is_err(),
                "{bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn validate_owner_rejects_control_and_special_characters() {
        for bad in ["foo\0bar", "foo\nbar", "foo\tbar", "foo bar", "foo$bar"] {
            assert!(
                validate_owner_name(bad).is_err(),
                "{bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn validate_owner_rejects_empty_or_huge() {
        assert!(validate_owner_name("").is_err());
        let huge: String = "a".repeat(101);
        assert!(validate_owner_name(&huge).is_err());
    }
}
