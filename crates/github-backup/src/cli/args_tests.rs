// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Unit tests for [`super::Args`].

use github_backup_types::config::BackupTarget;

use crate::cli::clone_type::CliCloneType;
use crate::cli::test_support::{parse, try_parse, try_parse_with_matches};

#[test]
fn parse_minimal_with_token() {
    let args = parse(&["github-backup", "octocat", "--token", "ghp_test"]);
    assert_eq!(args.owner.as_deref(), Some("octocat"));
    assert_eq!(args.token.as_deref(), Some("ghp_test"));
    assert!(!args.all);
    assert!(!args.org);
}

#[test]
fn parse_all_flag() {
    let args = parse(&["github-backup", "octocat", "--token", "t", "--all"]);
    assert!(args.all);
}

#[test]
fn parse_org_flag() {
    let args = parse(&["github-backup", "myorg", "--token", "t", "--org", "--all"]);
    assert!(args.org);
    let (_, _, opts) = args.into_backup_options();
    assert_eq!(opts.target, BackupTarget::Org);
}

#[test]
fn into_backup_options_all_enables_repositories() {
    let args = parse(&["github-backup", "octocat", "--token", "t", "--all"]);
    let (_, _, opts) = args.into_backup_options();
    assert!(opts.repositories);
    assert!(opts.issues);
    assert!(opts.pulls);
}

#[test]
fn into_backup_options_individual_flags() {
    let args = parse(&[
        "github-backup",
        "octocat",
        "--token",
        "t",
        "--repositories",
        "--issues",
    ]);
    let (_, _, opts) = args.into_backup_options();
    assert!(opts.repositories);
    assert!(opts.issues);
    assert!(!opts.pulls);
}

#[test]
fn release_assets_requires_releases() {
    let result = try_parse(&[
        "github-backup",
        "octocat",
        "--token",
        "t",
        "--release-assets",
    ]);
    assert!(
        result.is_err(),
        "--release-assets without --releases must fail"
    );
}

#[test]
fn parse_quiet_and_verbose() {
    let args = parse(&["github-backup", "octocat", "--token", "t", "-q"]);
    assert!(args.quiet);

    let args = parse(&["github-backup", "octocat", "--token", "t", "-vv"]);
    assert_eq!(args.verbose, 2);
}

#[test]
fn parse_concurrency_and_dry_run() {
    let args = parse(&[
        "github-backup",
        "octocat",
        "--token",
        "t",
        "--concurrency",
        "8",
        "--dry-run",
    ]);
    assert_eq!(args.concurrency, Some(8));
    assert!(args.dry_run);
    let (_, _, opts) = args.into_backup_options();
    assert_eq!(opts.concurrency, 8);
    assert!(opts.dry_run);
}

#[test]
fn parse_no_prune() {
    let args = parse(&[
        "github-backup",
        "octocat",
        "--token",
        "t",
        "--repositories",
        "--no-prune",
    ]);
    assert!(args.no_prune);
    let (_, _, opts) = args.into_backup_options();
    assert!(opts.no_prune);
}

#[test]
fn parse_clone_type_full() {
    let args = parse(&[
        "github-backup",
        "octocat",
        "--token",
        "t",
        "--repositories",
        "--clone-type",
        "full",
    ]);
    assert_eq!(args.clone_type, CliCloneType::Full);
    let (_, _, opts) = args.into_backup_options();
    assert_eq!(
        opts.clone_type,
        github_backup_types::config::CloneType::Full
    );
}

#[test]
fn parse_s3_flags() {
    let args = parse(&[
        "github-backup",
        "octocat",
        "--token",
        "t",
        "--repositories",
        "--s3-bucket",
        "my-bucket",
        "--s3-region",
        "eu-west-1",
        "--s3-access-key",
        "AKID",
        "--s3-secret-key",
        "SECRET",
    ]);
    assert_eq!(args.s3_bucket.as_deref(), Some("my-bucket"));
    assert_eq!(args.s3_region.as_deref(), Some("eu-west-1"));
}

#[test]
fn parse_mirror_flags() {
    let args = parse(&[
        "github-backup",
        "octocat",
        "--token",
        "t",
        "--repositories",
        "--mirror-to",
        "https://codeberg.org",
        "--mirror-token",
        "cb_token",
        "--mirror-owner",
        "alice",
    ]);
    assert_eq!(args.mirror_to.as_deref(), Some("https://codeberg.org"));
    assert_eq!(args.mirror_token.as_deref(), Some("cb_token"));
    assert_eq!(args.mirror_owner.as_deref(), Some("alice"));
}

#[test]
fn parse_report_flag() {
    let args = parse(&[
        "github-backup",
        "octocat",
        "--token",
        "t",
        "--all",
        "--report",
        "/tmp/report.json",
    ]);
    assert_eq!(
        args.report.as_deref(),
        Some(std::path::Path::new("/tmp/report.json"))
    );
}

#[test]
fn parse_output_flag() {
    let args = parse(&[
        "github-backup",
        "octocat",
        "--token",
        "t",
        "--output",
        "/data/backup",
    ]);
    assert_eq!(
        args.output.as_deref(),
        Some(std::path::Path::new("/data/backup"))
    );
}

#[test]
fn merge_config_applies_owner_when_cli_has_none() {
    let mut args = parse(&["github-backup", "--token", "t", "--repositories"]);
    assert!(args.owner.is_none());

    let cfg = github_backup_types::config::ConfigFile {
        owner: Some("config-user".to_string()),
        ..Default::default()
    };
    args.merge_config(&cfg);
    assert_eq!(args.owner.as_deref(), Some("config-user"));
}

#[test]
fn merge_config_cli_owner_wins() {
    let mut args = parse(&["github-backup", "cli-user", "--token", "t"]);
    let cfg = github_backup_types::config::ConfigFile {
        owner: Some("config-user".to_string()),
        ..Default::default()
    };
    args.merge_config(&cfg);
    // CLI owner should not be overridden.
    assert_eq!(args.owner.as_deref(), Some("cli-user"));
}

#[test]
fn merge_config_enables_categories() {
    let mut args = parse(&["github-backup", "octocat", "--token", "t"]);
    assert!(!args.issues);

    let cfg = github_backup_types::config::ConfigFile {
        issues: Some(true),
        repositories: Some(true),
        ..Default::default()
    };
    args.merge_config(&cfg);
    assert!(args.issues);
    assert!(args.repositories);
}

#[test]
fn merge_config_sets_org_from_config() {
    let mut args = parse(&["github-backup", "myorg", "--token", "t"]);
    assert!(!args.org);

    let cfg = github_backup_types::config::ConfigFile {
        org: Some(true),
        ..Default::default()
    };
    args.merge_config(&cfg);
    assert!(args.org);
}

#[test]
fn merge_config_sets_prefer_ssh_and_no_prune() {
    let mut args = parse(&["github-backup", "octocat", "--token", "t"]);
    assert!(!args.prefer_ssh);
    assert!(!args.no_prune);

    let cfg = github_backup_types::config::ConfigFile {
        prefer_ssh: Some(true),
        no_prune: Some(true),
        ..Default::default()
    };
    args.merge_config(&cfg);
    assert!(args.prefer_ssh);
    assert!(args.no_prune);
}

#[test]
fn merge_config_sets_mirror_fields() {
    let mut args = parse(&["github-backup", "octocat", "--token", "t"]);
    assert!(args.mirror_to.is_none());

    let cfg = github_backup_types::config::ConfigFile {
        mirror_to: Some("https://codeberg.org".to_string()),
        mirror_token: Some("cb_token".to_string()),
        mirror_owner: Some("alice".to_string()),
        mirror_private: Some(true),
        ..Default::default()
    };
    args.merge_config(&cfg);
    assert_eq!(args.mirror_to.as_deref(), Some("https://codeberg.org"));
    assert_eq!(args.mirror_token.as_deref(), Some("cb_token"));
    assert_eq!(args.mirror_owner.as_deref(), Some("alice"));
    assert!(args.mirror_private);
}

#[test]
fn merge_config_sets_s3_fields() {
    let mut args = parse(&["github-backup", "octocat", "--token", "t"]);
    assert!(args.s3_bucket.is_none());

    let cfg = github_backup_types::config::ConfigFile {
        s3_bucket: Some("my-bucket".to_string()),
        s3_region: Some("eu-west-1".to_string()),
        s3_prefix: Some("backups/".to_string()),
        s3_access_key: Some("AKID".to_string()),
        s3_secret_key: Some("SECRET".to_string()),
        ..Default::default()
    };
    args.merge_config(&cfg);
    assert_eq!(args.s3_bucket.as_deref(), Some("my-bucket"));
    assert_eq!(args.s3_region.as_deref(), Some("eu-west-1"));
    assert_eq!(args.s3_prefix.as_deref(), Some("backups/"));
}

#[test]
fn merge_config_cli_s3_bucket_wins() {
    let mut args = parse(&[
        "github-backup",
        "octocat",
        "--token",
        "t",
        "--s3-bucket",
        "cli-bucket",
        "--s3-region",
        "us-west-2",
    ]);
    let cfg = github_backup_types::config::ConfigFile {
        s3_bucket: Some("config-bucket".to_string()),
        s3_region: Some("eu-west-1".to_string()),
        ..Default::default()
    };
    args.merge_config(&cfg);
    // CLI wins for both bucket and region.
    assert_eq!(args.s3_bucket.as_deref(), Some("cli-bucket"));
    assert_eq!(args.s3_region.as_deref(), Some("us-west-2"));
}

// ── Blank option values (Compose / Kubernetes / Unraid pass "" for unset) ────

#[test]
fn normalize_env_values_drops_blank_and_whitespace_only_values() {
    let mut args = parse(&["github-backup", "octocat"]);
    args.token = Some(String::new());
    args.oauth_client_id = Some("   ".to_string());
    args.api_url = Some("\n".to_string());
    args.clone_host = Some("\t".to_string());
    args.mirror_token = Some(String::new());
    args.s3_access_key = Some(String::new());
    args.s3_secret_key = Some(String::new());
    args.encrypt_key = Some(String::new());
    args.notify_webhook = Some(String::new());

    args.normalize_env_values();

    assert_eq!(args.token, None);
    assert_eq!(args.oauth_client_id, None);
    assert_eq!(args.api_url, None);
    assert_eq!(args.clone_host, None);
    assert_eq!(args.mirror_token, None);
    assert_eq!(args.s3_access_key, None);
    assert_eq!(args.s3_secret_key, None);
    assert_eq!(args.encrypt_key, None);
    assert_eq!(args.notify_webhook, None);
}

#[test]
fn normalize_env_values_trims_surrounding_whitespace_but_keeps_the_value() {
    let mut args = parse(&["github-backup", "octocat"]);
    args.token = Some("ghp_abc\n".to_string());
    args.api_url = Some("  https://ghe.example.com/api/v3 ".to_string());

    args.normalize_env_values();

    assert_eq!(args.token.as_deref(), Some("ghp_abc"));
    assert_eq!(
        args.api_url.as_deref(),
        Some("https://ghe.example.com/api/v3")
    );
}

#[test]
fn normalize_env_values_leaves_unset_and_clean_values_alone() {
    let mut args = parse(&["github-backup", "octocat", "--token", "ghp_clean"]);
    args.normalize_env_values();
    assert_eq!(args.token.as_deref(), Some("ghp_clean"));
    assert_eq!(args.api_url, None);
}

// ── Flag dependencies that apply to the command line only ────────────────────

fn dependency_error(argv: &[&str]) -> Option<String> {
    let (args, matches) = try_parse_with_matches(argv).expect("argv must parse");
    args.check_dependencies(&matches)
        .err()
        .map(|e| e.to_string())
}

#[test]
fn typed_s3_credentials_without_bucket_are_rejected() {
    let err = dependency_error(&["github-backup", "octocat", "--s3-access-key", "k"])
        .expect("must be rejected");
    assert!(
        err.contains("--s3-access-key") && err.contains("--s3-bucket"),
        "{err}"
    );

    let err = dependency_error(&["github-backup", "octocat", "--s3-secret-key", "s"])
        .expect("must be rejected");
    assert!(
        err.contains("--s3-secret-key") && err.contains("--s3-bucket"),
        "{err}"
    );
}

#[test]
fn typed_mirror_token_without_destination_is_rejected() {
    let err = dependency_error(&["github-backup", "octocat", "--mirror-token", "t"])
        .expect("must be rejected");
    assert!(
        err.contains("--mirror-token") && err.contains("--mirror-to"),
        "{err}"
    );
}

#[test]
fn typed_oauth_client_id_without_device_auth_is_rejected() {
    let err = dependency_error(&["github-backup", "octocat", "--oauth-client-id", "Iv1.x"])
        .expect("must be rejected");
    assert!(
        err.contains("--oauth-client-id") && err.contains("--device-auth"),
        "{err}"
    );
}

#[test]
fn typed_flags_with_their_companion_are_accepted() {
    assert!(dependency_error(&[
        "github-backup",
        "octocat",
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
        "Iv1.x",
    ])
    .is_none());
}

#[test]
fn credential_not_typed_on_the_command_line_is_ignored_without_its_feature() {
    // Simulates a value that arrived from the environment (or a config file):
    // `matches` records no command-line occurrence, so there is nothing to reject.
    let (mut args, matches) = try_parse_with_matches(&["github-backup", "octocat"]).expect("parse");
    args.s3_access_key = Some("AKIAEXAMPLE".to_string());
    args.s3_secret_key = Some("secret".to_string());
    args.mirror_token = Some("secret".to_string());
    args.oauth_client_id = Some("Iv1.x".to_string());
    assert!(args.check_dependencies(&matches).is_ok());
}

#[test]
fn typed_credential_is_satisfied_by_a_companion_from_the_config_file() {
    let (mut args, matches) =
        try_parse_with_matches(&["github-backup", "octocat", "--s3-access-key", "k"])
            .expect("parse");
    assert!(args.check_dependencies(&matches).is_err(), "no bucket yet");

    let cfg = github_backup_types::config::ConfigFile {
        s3_bucket: Some("from-config".to_string()),
        ..Default::default()
    };
    args.merge_config(&cfg);
    assert!(args.check_dependencies(&matches).is_ok());
}
