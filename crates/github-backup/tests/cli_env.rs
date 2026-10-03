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
