// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Environment-variable behaviour of the real `github-backup` binary.
//!
//! Every test spawns the compiled binary with a *scrubbed* environment, so the
//! outcome never depends on what the developer (or CI runner) has exported.
//!
//! Why this matters: launchers such as Docker Compose, Kubernetes and Unraid
//! routinely pass every optional variable to the container as an *empty*
//! string, and operators who also use the AWS CLI have `AWS_ACCESS_KEY_ID`
//! exported in their shell.  None of that may stop a plain
//! `github-backup OWNER` from starting.
//!
//! `--list-scopes` is used as the probe: it runs after argument parsing and
//! config merging, then exits without touching the network or the filesystem.

use std::process::{Command, Output};

/// Environment variables that the CLI reads through clap's `env =` attribute.
const ENV_BACKED: &[(&str, &str)] = &[
    ("GITHUB_TOKEN", "ghp_example"),
    ("GITHUB_OAUTH_CLIENT_ID", "Iv1.example"),
    ("GITHUB_API_URL", "https://ghe.example.com/api/v3"),
    ("GITHUB_CLONE_HOST", "git.example.com"),
    ("MIRROR_TOKEN", "mirror-secret"),
    ("AWS_ACCESS_KEY_ID", "AKIAEXAMPLE"),
    ("AWS_SECRET_ACCESS_KEY", "aws-secret"),
    (
        "BACKUP_ENCRYPT_KEY",
        "0000000000000000000000000000000000000000000000000000000000000000",
    ),
    ("BACKUP_NOTIFY_WEBHOOK", "https://hooks.example.com/backup"),
];

fn run(args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_github-backup"));
    cmd.env_clear().args(args);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd.output().expect("failed to spawn github-backup")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn clean_environment_parses() {
    let out = run(&["octocat", "--list-scopes"], &[]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
}

#[test]
fn any_single_env_var_with_a_value_does_not_break_parsing() {
    for (name, value) in ENV_BACKED {
        let out = run(&["octocat", "--list-scopes"], &[(name, value)]);
        assert!(
            out.status.success(),
            "{name}={value} made the CLI exit with {:?}: {}",
            out.status.code(),
            stderr(&out)
        );
    }
}

#[test]
fn any_single_env_var_set_to_empty_does_not_break_parsing() {
    for (name, _) in ENV_BACKED {
        let out = run(&["octocat", "--list-scopes"], &[(name, "")]);
        assert!(
            out.status.success(),
            "{name}= (empty) made the CLI exit with {:?}: {}",
            out.status.code(),
            stderr(&out)
        );
    }
}

#[test]
fn every_env_var_set_to_empty_at_once_does_not_break_parsing() {
    // Exactly what a Compose service that forwards the full variable set
    // renders when the operator has configured none of the optional ones.
    let empties: Vec<(&str, &str)> = ENV_BACKED.iter().map(|(name, _)| (*name, "")).collect();
    let out = run(&["octocat", "--list-scopes"], &empties);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
}

#[test]
fn every_env_var_with_a_value_at_once_does_not_break_parsing() {
    // An operator with AWS credentials, a mirror token and a webhook exported
    // who runs a plain local backup with none of the matching flags.
    let out = run(&["octocat", "--list-scopes"], ENV_BACKED);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
}

// ── Explicit command-line misuse must still be rejected ──────────────────────
//
// Dropping clap's `requires =` on the env-backed arguments (so that ambient
// variables cannot trip it) must not silently accept a flag the user typed
// without the flag it depends on.

#[test]
fn explicit_s3_access_key_without_bucket_is_rejected() {
    let out = run(
        &["octocat", "--list-scopes", "--s3-access-key", "AKIAEXAMPLE"],
        &[],
    );
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("--s3-bucket"), "{}", stderr(&out));
}

#[test]
fn explicit_s3_secret_key_without_bucket_is_rejected() {
    let out = run(
        &["octocat", "--list-scopes", "--s3-secret-key", "secret"],
        &[],
    );
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("--s3-bucket"), "{}", stderr(&out));
}

#[test]
fn explicit_mirror_token_without_destination_is_rejected() {
    let out = run(
        &["octocat", "--list-scopes", "--mirror-token", "secret"],
        &[],
    );
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("--mirror-to"), "{}", stderr(&out));
}

#[test]
fn explicit_oauth_client_id_without_device_auth_is_rejected() {
    let out = run(
        &[
            "octocat",
            "--list-scopes",
            "--oauth-client-id",
            "Iv1.example",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("--device-auth"), "{}", stderr(&out));
}

#[test]
fn explicit_flags_with_their_dependency_are_accepted() {
    let out = run(
        &[
            "octocat",
            "--list-scopes",
            "--s3-bucket",
            "b",
            "--s3-access-key",
            "k",
            "--s3-secret-key",
            "s",
            "--mirror-to",
            "https://codeberg.org",
            "--mirror-token",
            "t",
            "--device-auth",
            "--oauth-client-id",
            "Iv1.example",
        ],
        &[],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
}

/// Regression (docs audit DA-05): the documented `--decrypt` command needed a
/// dummy OWNER even though it only reads a file.
#[test]
fn decrypt_needs_no_owner() {
    let dir = std::env::temp_dir().join(format!("gbk-decrypt-{}", std::process::id()));
    let out = run(
        &[
            "--decrypt",
            "--decrypt-input",
            dir.join("missing.enc").to_str().unwrap(),
            "--decrypt-output",
            dir.join("out.json").to_str().unwrap(),
            "--encrypt-key",
            &"ab".repeat(32),
        ],
        &[],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("no owner specified") && !stderr.contains("quickstart"),
        "decrypt must not ask for an OWNER: {stderr}"
    );
    assert!(!out.status.success(), "the input file does not exist");
    assert!(
        stderr.to_lowercase().contains("missing.enc") || stderr.to_lowercase().contains("read"),
        "the error should be about the missing input: {stderr}"
    );
}

// ── First contact and usability ──────────────────────────────────────────────

/// A scratch directory used as `HOME` (so no real config file is read).
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("gbk-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// U6: `--help` printed the value of `BACKUP_NOTIFY_WEBHOOK`, whose path is
/// the secret of a Slack or Discord webhook.
#[test]
fn help_never_prints_the_webhook_url() {
    let out = run(
        &["--help"],
        &[(
            "BACKUP_NOTIFY_WEBHOOK",
            "https://hooks.slack.com/services/T0/B0/SECRETPART",
        )],
    );
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("--notify-webhook"), "{help}");
    assert!(!help.contains("SECRETPART"), "{help}");
}

/// U10: writing to a closed stdout panicked (exit 101).
#[test]
fn a_closed_stdout_does_not_panic() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_github-backup"))
        .env_clear()
        .args(["--print-config-template"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    drop(child.stdout.take());
    let out = child.wait_with_output().expect("wait");
    assert_ne!(out.status.code(), Some(101), "stderr: {}", stderr(&out));
    assert!(!stderr(&out).contains("panicked"), "{}", stderr(&out));
}

#[test]
fn no_token_and_no_owner_prints_the_quickstart() {
    let home = scratch("quickstart");
    let out = run(&[], &[("HOME", home.to_str().unwrap())]);
    assert_eq!(out.status.code(), Some(1));
    let text = stderr(&out);
    assert!(
        text.contains("gh auth login") && text.contains("Quickstart"),
        "{text}"
    );
}

/// U5: `--doctor` ran before the validation a backup makes, and called a
/// configuration "ready" that the run then rejected.
#[test]
fn doctor_rejects_what_a_run_would_reject() {
    let out = run(&["octocat", "--doctor", "--since", "yesterday"], &[]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains("--since"), "{}", stderr(&out));
    let out = run(&["octocat", "--doctor", "--encrypt-key", "short"], &[]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
}

/// U5: a mirror or S3 upload without credentials failed only after the whole
/// backup; it is now refused before anything is fetched.
#[test]
fn missing_mirror_or_s3_credentials_fail_at_startup() {
    let out = run(
        &["octocat", "--mirror-to", "https://codeberg.org"],
        &[("GITHUB_TOKEN", "ghp_example")],
    );
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains("MIRROR_TOKEN"), "{}", stderr(&out));
    let out = run(
        &["octocat", "--s3-bucket", "b"],
        &[("GITHUB_TOKEN", "ghp_example")],
    );
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(stderr(&out).contains("access key"), "{}", stderr(&out));
}

/// U12: `--tui` ignored the config file and silently dropped flags it cannot
/// honour; they are now an error.
#[test]
fn tui_rejects_flags_it_cannot_honour() {
    let out = run(&["octocat", "--tui", "--report", "r.json"], &[]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("--report"), "{}", stderr(&out));
}

/// U8: the default config file is read without `--config`, and its settings
/// combine with the command line (U4: `--mirror-type` with `mirror_to` from
/// the file used to exit 2).
#[test]
fn the_default_config_file_is_loaded_and_combines_with_the_command_line() {
    let home = scratch("config");
    // The default location: `%APPDATA%\github-backup` on Windows,
    // `$HOME/.config/github-backup` elsewhere (no `XDG_CONFIG_HOME` here).
    let (var, dir) = if cfg!(windows) {
        ("APPDATA", home.join("github-backup"))
    } else {
        ("HOME", home.join(".config").join("github-backup"))
    };
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("config.toml"),
        "owner = \"octocat\"\nmirror_to = \"https://gitlab.com\"\nskip = [\"hooks\"]\n",
    )
    .unwrap();
    let out = run(
        &["--mirror-type", "gitlab", "--list-scopes"],
        &[(var, home.to_str().unwrap())],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("loaded config file"),
        "{}",
        stderr(&out)
    );
    // `skip = ["hooks"]` was applied: no admin:repo_hook.
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("admin:repo_hook"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn secrets_on_the_command_line_are_warned_about() {
    let out = run(&["octocat", "--list-scopes", "--token", "ghp_x"], &[]);
    assert!(out.status.success());
    let text = stderr(&out);
    assert!(
        text.contains("--token was given on the command line"),
        "{text}"
    );
    assert!(!text.contains("ghp_x"), "the value is never echoed: {text}");
}

/// Windows gives a program's main thread a 1 MiB stack (Linux 8 MiB), and a
/// debug build of this binary needs more than that to start: every test above
/// failed on Windows with "thread 'main' has overflowed its stack".  The
/// program therefore runs on a thread with its own 8 MiB stack.  On Unix the
/// main thread's stack is the `ulimit -s` of the shell, which reproduces the
/// Windows limit.
#[test]
#[cfg(unix)]
fn the_binary_starts_with_a_one_megabyte_main_thread_stack() {
    let out = Command::new("/bin/sh")
        .env_clear()
        .arg("-c")
        .arg("ulimit -s 1024 && exec \"$0\" --help")
        .arg(env!("CARGO_BIN_EXE_github-backup"))
        .output()
        .expect("failed to spawn sh");
    assert!(
        out.status.success(),
        "status {:?}: {}",
        out.status,
        stderr(&out)
    );
}
