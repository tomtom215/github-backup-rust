// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Unit tests for [`super::Args`].

use clap::Parser;
use github_backup_types::config::BackupTarget;

use super::Args;
use crate::cli::clone_type::CliCloneType;

fn parse(args: &[&str]) -> Args {
    Args::parse_from(args)
}

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
    let result = Args::try_parse_from([
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
    assert_eq!(args.s3_region, "eu-west-1");
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
    let mut args = Args::parse_from(["github-backup", "--token", "t", "--repositories"]);
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
