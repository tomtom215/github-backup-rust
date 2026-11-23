// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Process start-up helpers: logging, the embedded config template, the
//! default config file, config-file permission checks, owner validation and
//! credentials (including the GitHub CLI's token).

use std::path::{Path, PathBuf};
use std::time::Duration;

use tracing::{debug, info, warn};

use zeroize::Zeroizing;

use github_backup_client::oauth::device_flow_for;
use github_backup_types::config::{AppCredential, Credential};

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

/// Where the config file is looked for when `--config` is not given:
/// `$XDG_CONFIG_HOME/github-backup/config.toml`, else
/// `$HOME/.config/github-backup/config.toml` (also on macOS, matching the
/// template's advice), and `%APPDATA%\github-backup\config.toml` on Windows.
///
/// `var` is the environment lookup (a parameter so tests need not touch the
/// process environment).  Relative directories are ignored, as the XDG
/// specification requires.
pub(crate) fn default_config_path(var: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let dir = |name: &str| var(name).map(PathBuf::from).filter(|p| p.is_absolute());
    let base = if cfg!(windows) {
        dir("APPDATA")?
    } else {
        dir("XDG_CONFIG_HOME").or_else(|| dir("HOME").map(|h| h.join(".config")))?
    };
    Some(base.join("github-backup").join("config.toml"))
}

/// The host the GitHub CLI knows an API URL by: `github.com` for
/// `https://api.github.com`, otherwise the URL's host (GitHub Enterprise
/// Server: `https://ghe.example.com/api/v3` is `ghe.example.com`).
pub(crate) fn gh_hostname(api_url: Option<&str>) -> String {
    let Some(url) = api_url else {
        return "github.com".to_string();
    };
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = authority.rsplit('@').next().unwrap_or(authority);
    if host.eq_ignore_ascii_case("api.github.com") {
        "github.com".to_string()
    } else {
        host.to_string()
    }
}

/// How long `gh auth token` may take.
const GH_TIMEOUT: Duration = Duration::from_secs(5);

/// Asks the GitHub CLI for its token for `hostname` (`gh auth token
/// --hostname <host>`).
///
/// `None` when `gh` is not installed, not logged in to that host, or does not
/// answer within [`GH_TIMEOUT`].  The token is read from `gh`'s standard
/// output and never logged; `gh`'s own messages are discarded.
pub(crate) async fn token_from_gh_cli(program: &Path, hostname: &str) -> Option<String> {
    let child = tokio::process::Command::new(program)
        .args(["auth", "token", "--hostname", hostname])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn();
    let child = match child {
        Ok(c) => c,
        Err(e) => {
            debug!("GitHub CLI not available ({e})");
            return None;
        }
    };
    match tokio::time::timeout(GH_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(out)) if out.status.success() => {
            let token = String::from_utf8(out.stdout).ok()?.trim().to_string();
            (!token.is_empty()).then_some(token)
        }
        Ok(_) => {
            debug!(hostname, "the GitHub CLI has no token for this host");
            None
        }
        Err(_) => {
            warn!("`gh auth token` did not answer within {GH_TIMEOUT:?}; ignoring the GitHub CLI");
            None
        }
    }
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

/// Checks the mirror and S3 settings before anything else happens, so that a
/// missing token or key fails the run at once instead of after the whole
/// backup.  The checks are [`crate::post_process::validate_post_processing`];
/// an OWNER that is still to be taken from the token stands in for the
/// mirror owner it defaults to.
///
/// # Errors
///
/// A message naming the missing or malformed setting.
pub(crate) fn check_destinations(args: &Args) -> Result<(), String> {
    if args.owner.is_none() && args.mirror_owner.is_none() && args.mirror_to.is_some() {
        let mut resolved = args.clone();
        resolved.owner = Some("the token's account".to_string());
        return crate::post_process::validate_post_processing(&resolved);
    }
    crate::post_process::validate_post_processing(args)
}

/// The environment variable that may hold the GitHub App's private key itself
/// (the PEM text), for containers that cannot mount a file.
pub(crate) const APP_KEY_ENV: &str = "GITHUB_APP_PRIVATE_KEY";

/// The GitHub App credential `args` configure, or `None` when no App is
/// configured (`--app-id` / `app_id` unset).
///
/// `key_env` is the value of [`APP_KEY_ENV`] (a parameter so tests need not
/// touch the process environment); it is ignored without an App ID, like any
/// credential that merely arrives through the environment.  A value without
/// line breaks but with literal `\n` sequences (as some secret stores write a
/// PEM on one line) has them turned into line breaks.
///
/// # Errors
///
/// A message naming the conflict or the missing piece: an App together with
/// a token or `--device-auth`, an App setting without `--app-id`, no key or
/// two keys, or a key file that cannot be read.
pub(crate) fn app_credential(
    args: &Args,
    key_env: Option<String>,
) -> Result<Option<AppCredential>, String> {
    let Some(app_id) = args.app_id.clone() else {
        if args.app_key.is_some() || args.app_installation_id.is_some() {
            return Err("--app-key / app_key_path and --app-installation-id / \
                        app_installation_id need --app-id (app_id): the GitHub App's \
                        client ID or App ID"
                .to_string());
        }
        return Ok(None);
    };
    if args.token.is_some() {
        return Err(
            "both a token (--token, GITHUB_TOKEN, GH_TOKEN or `token` in the config \
                    file) and a GitHub App (--app-id) are configured; use one of them"
                .to_string(),
        );
    }
    if args.device_auth {
        return Err("--device-auth cannot be combined with a GitHub App (--app-id)".to_string());
    }
    let key_env = key_env
        .map(Zeroizing::new)
        .filter(|pem| !pem.trim().is_empty());
    let private_key_pem = match (&args.app_key, key_env) {
        (Some(_), Some(_)) => {
            return Err(format!(
                "the GitHub App key is given twice (--app-key / app_key_path and {APP_KEY_ENV}); \
                 keep one"
            ))
        }
        (Some(path), None) => std::fs::read_to_string(path)
            .map(Zeroizing::new)
            .map_err(|e| format!("cannot read the GitHub App key {}: {e}", path.display()))?,
        (None, Some(pem)) if !pem.contains('\n') && pem.contains("\\n") => {
            Zeroizing::new(pem.replace("\\n", "\n"))
        }
        (None, Some(pem)) => pem,
        (None, None) => {
            return Err(format!(
                "--app-id needs the App's private key: --app-key <PATH>, app_key_path in the \
                 config file, or the key itself in {APP_KEY_ENV}"
            ))
        }
    };
    Ok(Some(AppCredential {
        app_id,
        private_key_pem,
        installation_id: args.app_installation_id,
    }))
}

/// Resolves the GitHub credential from CLI args.
///
/// Returns a [`Credential::App`] when a GitHub App is configured (see
/// [`app_credential`]), a [`Credential::Token`] (PAT or OAuth), or
/// [`Credential::Anonymous`] when no auth method is provided.
pub(crate) async fn obtain_credential(args: &Args) -> Result<Credential, String> {
    if let Some(app) = app_credential(args, std::env::var(APP_KEY_ENV).ok())? {
        return Ok(Credential::App(app));
    }
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

        let token = device_flow_for(args.api_url.as_deref(), client_id, scope, |code, url| {
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

    const PEM: &str = "-----BEGIN RSA PRIVATE KEY-----\nTUlJ\n-----END RSA PRIVATE KEY-----\n";

    fn app_args(extra: &[&str]) -> Args {
        let mut argv = vec!["github-backup", "acme", "--app-id", "Iv23liX"];
        argv.extend_from_slice(extra);
        crate::cli::test_support::parse(&argv)
    }

    #[test]
    fn the_config_file_supplies_app_settings_the_command_line_did_not() {
        let cfg = ConfigFile::from_toml_str(
            "app_id = \"Iv23liFile\"\napp_key_path = \"/k.pem\"\napp_installation_id = 5\n",
        )
        .unwrap();
        let mut args = crate::cli::test_support::parse(&["github-backup", "--app-id", "Iv23liCli"]);
        args.merge_config(&cfg);
        assert_eq!(args.app_id.as_deref(), Some("Iv23liCli"));
        assert_eq!(args.app_key.as_deref(), Some(Path::new("/k.pem")));
        assert_eq!(args.app_installation_id, Some(5));
    }

    #[test]
    fn no_app_id_means_no_app() {
        let args = crate::cli::test_support::parse(&["github-backup", "acme"]);
        assert!(app_credential(&args, Some(PEM.into())).unwrap().is_none());
    }

    #[test]
    fn the_app_key_comes_from_a_file_or_the_environment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.pem");
        std::fs::write(&path, PEM).unwrap();
        let args = app_args(&[
            "--app-key",
            path.to_str().unwrap(),
            "--app-installation-id",
            "9",
        ]);
        let app = app_credential(&args, None).unwrap().unwrap();
        assert_eq!(app.app_id, "Iv23liX");
        assert_eq!(app.installation_id, Some(9));
        assert_eq!(&*app.private_key_pem, PEM);

        let app = app_credential(&app_args(&[]), Some(PEM.into()))
            .unwrap()
            .unwrap();
        assert_eq!(&*app.private_key_pem, PEM);
        // A PEM flattened onto one line with `\n` escapes.
        let flat = PEM.replace('\n', "\\n");
        let app = app_credential(&app_args(&[]), Some(flat)).unwrap().unwrap();
        assert_eq!(&*app.private_key_pem, PEM);
    }

    #[test]
    fn an_app_conflicts_with_tokens_and_needs_exactly_one_key() {
        let token = app_args(&["--token", "ghp_x"]);
        let e = app_credential(&token, Some(PEM.into())).unwrap_err();
        assert!(e.contains("use one of them"), "{e}");
        let device = app_args(&["--device-auth"]);
        assert!(app_credential(&device, Some(PEM.into())).is_err());
        let e = app_credential(&app_args(&[]), None).unwrap_err();
        assert!(e.contains("private key"), "{e}");
        let e = app_credential(&app_args(&[]), Some("  ".into())).unwrap_err();
        assert!(e.contains("private key"), "{e}");
        let twice = app_args(&["--app-key", "/nonexistent.pem"]);
        let e = app_credential(&twice, Some(PEM.into())).unwrap_err();
        assert!(e.contains("twice"), "{e}");
        let e = app_credential(&twice, None).unwrap_err();
        assert!(e.contains("/nonexistent.pem"), "{e}");
        let orphan =
            crate::cli::test_support::parse(&["github-backup", "--app-installation-id", "3"]);
        assert!(app_credential(&orphan, None)
            .unwrap_err()
            .contains("need --app-id"));
    }

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
    fn config_template_documents_the_defaults_of_optional_keys() {
        let toml = config_template();
        // Nothing is required any more: OWNER defaults to the token's account
        // (this test used to insist that owner was flagged REQUIRED).
        assert!(!toml.contains("REQUIRED"), "no key is required");
        assert!(
            toml.contains("default: the account the token"),
            "template must state the default for owner"
        );
        assert!(
            toml.contains("everything is backed up"),
            "template must state the default categories"
        );
        assert!(
            toml.contains("default: the current directory"),
            "template must state the default for output"
        );
    }

    #[test]
    fn the_default_config_file_follows_xdg_then_home() {
        if cfg!(windows) {
            return;
        }
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert_eq!(
            default_config_path(env(&[("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")])),
            Some(PathBuf::from("/x/github-backup/config.toml"))
        );
        assert_eq!(
            default_config_path(env(&[("XDG_CONFIG_HOME", "rel"), ("HOME", "/h")])),
            Some(PathBuf::from("/h/.config/github-backup/config.toml"))
        );
        assert_eq!(default_config_path(env(&[])), None);
    }

    #[test]
    fn gh_hostname_maps_the_api_url_to_the_web_host() {
        assert_eq!(gh_hostname(None), "github.com");
        assert_eq!(gh_hostname(Some("https://api.github.com")), "github.com");
        assert_eq!(gh_hostname(Some("https://API.github.com/")), "github.com");
        assert_eq!(
            gh_hostname(Some("https://ghe.example.com/api/v3")),
            "ghe.example.com"
        );
        assert_eq!(
            gh_hostname(Some("https://ghe.example.com:8443/api/v3")),
            "ghe.example.com:8443"
        );
    }

    /// A fake `gh` that prints a token only for the expected host.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_gh_cli_token_is_read_for_the_right_host() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().expect("tempdir");
        let gh = dir.path().join("gh");
        std::fs::write(
            &gh,
            "#!/bin/sh\n[ \"$1 $2 $3 $4\" = \"auth token --hostname github.com\" ] || exit 1\n\
             echo 'gho_fromgh'\necho 'some notice' >&2\n",
        )
        .unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            token_from_gh_cli(&gh, "github.com").await.as_deref(),
            Some("gho_fromgh")
        );
        assert_eq!(token_from_gh_cli(&gh, "ghe.example.com").await, None);
        assert_eq!(
            token_from_gh_cli(&dir.path().join("missing"), "github.com").await,
            None
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
