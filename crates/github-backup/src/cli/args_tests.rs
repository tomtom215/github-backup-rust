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
fn pruning_is_off_by_default_and_opt_in() {
    let off = parse(&["github-backup", "octocat", "--token", "t", "--repositories"]);
    let (_, _, opts) = off.into_backup_options();
    assert!(
        !opts.prune,
        "a backup keeps refs deleted upstream by default"
    );

    let on = parse(&[
        "github-backup",
        "octocat",
        "--token",
        "t",
        "--repositories",
        "--prune",
    ]);
    let (_, _, opts) = on.into_backup_options();
    assert!(opts.prune);
}

/// `--no-prune` is the default now; it is still accepted so existing
/// scripts keep working, and it cannot be combined with `--prune`.
#[test]
fn deprecated_no_prune_is_accepted_and_conflicts_with_prune() {
    let args = parse(&[
        "github-backup",
        "octocat",
        "--token",
        "t",
        "--repositories",
        "--no-prune",
    ]);
    let (_, _, opts) = args.into_backup_options();
    assert!(!opts.prune);
    assert!(try_parse(&[
        "github-backup",
        "octocat",
        "--token",
        "t",
        "--prune",
        "--no-prune",
    ])
    .is_err());
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
fn merge_config_sets_prefer_ssh_and_prune() {
    let mut args = parse(&["github-backup", "octocat", "--token", "t"]);
    assert!(!args.prefer_ssh);
    assert!(!args.prune);

    let cfg = github_backup_types::config::ConfigFile {
        prefer_ssh: Some(true),
        prune: Some(true),
        ..Default::default()
    };
    args.merge_config(&cfg);
    assert!(args.prefer_ssh);
    assert!(args.prune);
}

/// A config file written for the old default (`no_prune = true`) still loads,
/// and an explicit `--no-prune` on the command line beats `prune = true`.
#[test]
fn old_no_prune_config_key_still_loads_and_cli_no_prune_wins() {
    let cfg =
        github_backup_types::config::ConfigFile::from_toml_str("no_prune = true\nprune = true\n")
            .expect("both keys parse");
    let mut args = parse(&["github-backup", "octocat", "--token", "t", "--no-prune"]);
    args.merge_config(&cfg);
    assert!(!args.prune, "explicit --no-prune beats the file");
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

/// Regression (docs audit DA-13): `--all` silently dropped `--clone-starred`
/// and `--action-runs`, the two opt-in categories it leaves off.
#[test]
fn all_still_honours_the_opt_in_categories_asked_for_explicitly() {
    let args = parse(&[
        "github-backup",
        "octocat",
        "--all",
        "--clone-starred",
        "--action-runs",
    ]);
    let (_, _, opts) = args.into_backup_options();
    assert!(
        opts.clone_starred,
        "--all --clone-starred must clone starred repos"
    );
    assert!(
        opts.action_runs,
        "--all --action-runs must fetch workflow runs"
    );
    assert!(opts.issues, "--all still enables the rest");

    let plain = parse(&["github-backup", "octocat", "--all"]);
    let (_, _, opts) = plain.into_backup_options();
    assert!(
        !opts.clone_starred && !opts.action_runs,
        "still opt-in without the flags"
    );
}

/// Regression (DA-17): "CLI always overrides config" was false for
/// `--clone-type mirror`, because mirror is the default and so looked absent.
#[test]
fn an_explicit_clone_type_mirror_beats_the_config_file() {
    use github_backup_types::config::{CloneType, ConfigFile};
    let cfg = ConfigFile {
        clone_type: Some(CloneType::Bare),
        ..Default::default()
    };

    let (mut explicit, matches) =
        try_parse_with_matches(&["github-backup", "octocat", "--clone-type", "mirror"])
            .expect("parse");
    explicit.merge_config_with(&cfg, &matches);
    let (_, _, opts) = explicit.into_backup_options();
    assert_eq!(opts.clone_type, CloneType::Mirror);

    let (mut absent, matches) =
        try_parse_with_matches(&["github-backup", "octocat"]).expect("parse");
    absent.merge_config_with(&cfg, &matches);
    let (_, _, opts) = absent.into_backup_options();
    assert_eq!(
        opts.clone_type,
        CloneType::Bare,
        "the config applies when the CLI is silent"
    );
}

// ── Default categories, --skip ───────────────────────────────────────────────

/// U1: with no category flag a run backed up nothing (only `repos.json`) and
/// still reported success.  No selection now means everything, like `--all`.
#[test]
fn no_category_selected_backs_up_everything() {
    let args = parse(&["github-backup", "octocat"]);
    assert!(args.uses_default_categories());
    let (_, _, opts) = args.into_backup_options();
    let all = parse(&["github-backup", "octocat", "--all"])
        .into_backup_options()
        .2;
    assert_eq!(
        crate::cli::categories::enabled(&opts),
        crate::cli::categories::enabled(&all)
    );
    assert!(opts.repositories && opts.issues && opts.wikis);
}

#[test]
fn any_category_flag_selects_a_subset_but_extras_do_not() {
    let args = parse(&["github-backup", "octocat", "--issues"]);
    assert!(!args.uses_default_categories());
    let (_, _, opts) = args.into_backup_options();
    assert!(opts.issues && !opts.repositories);

    // `--private` and `--action-runs` modify the default instead.
    let args = parse(&["github-backup", "octocat", "--private", "--action-runs"]);
    assert!(args.uses_default_categories());
    let (_, _, opts) = args.into_backup_options();
    assert!(opts.repositories && opts.private && opts.action_runs);
}

#[test]
fn a_category_in_the_config_file_is_a_selection_too() {
    let mut args = parse(&["github-backup", "octocat"]);
    args.merge_config(&github_backup_types::config::ConfigFile {
        gists: Some(true),
        ..Default::default()
    });
    assert!(!args.uses_default_categories());
    let (_, _, opts) = args.into_backup_options();
    assert!(opts.gists && !opts.repositories);
}

/// `--skip` is the way to turn off what the default, `--all` or the config
/// file turned on (switches are otherwise OR-ed and cannot be switched off).
#[test]
fn skip_removes_categories_from_the_default_all_and_the_config_file() {
    let args = parse(&["github-backup", "octocat", "--skip", "hooks,packages"]);
    let (_, _, opts) = args.into_backup_options();
    assert!(!opts.hooks && !opts.packages && opts.issues);

    let mut args = parse(&["github-backup", "octocat", "--skip", "wikis"]);
    args.merge_config(&github_backup_types::config::ConfigFile {
        wikis: Some(true),
        issues: Some(true),
        gists: Some(true),
        skip: Some(vec!["issues".into()]),
        ..Default::default()
    });
    let (_, _, opts) = args.into_backup_options();
    assert!(!opts.wikis && !opts.issues && opts.gists);
}

#[test]
fn an_unknown_skip_name_is_rejected_from_the_command_line_and_the_config_file() {
    let e = try_parse(&["github-backup", "octocat", "--skip", "hookz"]).unwrap_err();
    assert!(e.to_string().contains("hookz"), "{e}");

    let (mut args, matches) = try_parse_with_matches(&["github-backup", "octocat"]).unwrap();
    args.merge_config(&github_backup_types::config::ConfigFile {
        skip: Some(vec!["hookz".into()]),
        ..Default::default()
    });
    let e = args.check_dependencies(&matches).unwrap_err().to_string();
    assert!(e.contains("hookz") && e.contains("hooks"), "{e}");
}

#[test]
fn concurrency_zero_is_clamped_to_one() {
    let (_, _, opts) =
        parse(&["github-backup", "octocat", "--concurrency", "0"]).into_backup_options();
    assert_eq!(opts.concurrency, 1);
}

// ── Mirror / S3 settings and the config file (U4, U7) ────────────────────────

/// U4: `mirror_to` in the config file plus `--mirror-type` on the command
/// line exited 2 ("requires --mirror-to"), because clap checked before the
/// merge.
#[test]
fn mirror_settings_on_the_command_line_accept_a_destination_from_the_config_file() {
    use github_backup_types::config::{ConfigFile, MirrorType};
    let (mut args, matches) = try_parse_with_matches(&[
        "github-backup",
        "octocat",
        "--mirror-type",
        "gitlab",
        "--mirror-owner",
        "me",
        "--mirror-public",
        "--s3-region",
        "eu-west-1",
        "--s3-include-assets",
    ])
    .expect("must parse without the destinations");
    args.merge_config(&ConfigFile {
        mirror_to: Some("https://gitlab.com".into()),
        s3_bucket: Some("b".into()),
        ..Default::default()
    });
    assert!(args.check_dependencies(&matches).is_ok());
    assert_eq!(args.mirror_type, Some(MirrorType::Gitlab));
}

#[test]
fn mirror_and_s3_settings_without_their_destination_are_rejected_after_the_merge() {
    for argv in [
        &["github-backup", "o", "--mirror-type", "gitlab"][..],
        &["github-backup", "o", "--mirror-owner", "me"][..],
        &["github-backup", "o", "--s3-prefix", "p"][..],
        &["github-backup", "o", "--s3-delete-stale"][..],
    ] {
        assert!(dependency_error(argv).is_some(), "{argv:?}");
    }
    // A config-file value without its destination is just as wrong.
    let (mut args, matches) = try_parse_with_matches(&["github-backup", "o"]).unwrap();
    args.merge_config(&github_backup_types::config::ConfigFile {
        mirror_type: Some(github_backup_types::config::MirrorType::Gitlab),
        ..Default::default()
    });
    let e = args.check_dependencies(&matches).unwrap_err().to_string();
    assert!(e.contains("mirror_type") && e.contains("mirror_to"), "{e}");
}

/// U7: `--mirror-type` was free text and a typo silently meant Gitea.
#[test]
fn a_mistyped_mirror_type_is_a_usage_error() {
    let e = try_parse(&["github-backup", "o", "--mirror-type", "gitlb"]).unwrap_err();
    let text = e.to_string();
    assert!(text.contains("gitlb") && text.contains("gitlab"), "{text}");
}

#[test]
fn the_command_line_mirror_type_beats_the_config_file() {
    use github_backup_types::config::{ConfigFile, MirrorType};
    let mut args = parse(&["github-backup", "o", "--mirror-type", "gitea"]);
    args.merge_config(&ConfigFile {
        mirror_type: Some(MirrorType::Gitlab),
        ..Default::default()
    });
    assert_eq!(args.mirror_type, Some(MirrorType::Gitea));
    let mut args = parse(&["github-backup", "o"]);
    args.merge_config(&ConfigFile {
        mirror_type: Some(MirrorType::Gitlab),
        ..Default::default()
    });
    assert_eq!(args.mirror_type, Some(MirrorType::Gitlab));
}

#[test]
fn mirror_type_in_a_config_file_is_validated_when_it_is_read() {
    use github_backup_types::config::{ConfigFile, MirrorType};
    let cfg = ConfigFile::from_toml_str("mirror_type = \"gitlab\"").unwrap();
    assert_eq!(cfg.mirror_type, Some(MirrorType::Gitlab));
    assert!(ConfigFile::from_toml_str("mirror_type = \"gitlb\"").is_err());
}

// ── Tokens and secrets (U8) ──────────────────────────────────────────────────

#[test]
fn gh_token_is_used_only_when_no_other_token_was_given() {
    let mut args = parse(&["github-backup", "o"]);
    args.token_from_gh_token(Some(" gho_x\n".into()));
    assert_eq!(args.token.as_deref(), Some("gho_x"));

    let mut args = parse(&["github-backup", "o", "--token", "ghp_cli"]);
    args.token_from_gh_token(Some("gho_x".into()));
    assert_eq!(args.token.as_deref(), Some("ghp_cli"));

    let mut args = parse(&["github-backup", "o"]);
    args.token_from_gh_token(Some(String::new()));
    assert_eq!(args.token, None);
}

#[test]
fn secrets_typed_on_the_command_line_are_named() {
    let (_, matches) = try_parse_with_matches(&[
        "github-backup",
        "o",
        "--token",
        "t",
        "--mirror-to",
        "https://x.example",
        "--mirror-token",
        "m",
        "--encrypt-key",
        "k",
        "--notify-webhook",
        "https://hooks.example/secret",
    ])
    .unwrap();
    assert_eq!(
        crate::cli::Args::secrets_on_command_line(&matches),
        [
            "--token",
            "--mirror-token",
            "--encrypt-key",
            "--notify-webhook"
        ]
    );
    let (_, matches) = try_parse_with_matches(&["github-backup", "o"]).unwrap();
    assert!(crate::cli::Args::secrets_on_command_line(&matches).is_empty());
}

// ── Modes (U15) ──────────────────────────────────────────────────────────────

#[test]
fn two_modes_at_once_are_a_usage_error() {
    for argv in [
        &["github-backup", "o", "--verify", "--restore"][..],
        &["github-backup", "o", "--doctor", "--tui"][..],
        &["github-backup", "o", "--every", "1h", "--list-scopes"][..],
    ] {
        assert!(try_parse(argv).is_err(), "{argv:?}");
    }
    // `--verify` no longer conflicts with category flags.
    assert!(try_parse(&["github-backup", "o", "--verify", "--all"]).is_ok());
}

/// `--print-config-template` used to be detected anywhere in argv, even as
/// the value of another option.
#[test]
fn mode_flags_given_as_option_values_are_values() {
    let args = parse(&["github-backup", "o", "--report=--print-config-template"]);
    assert!(!args.print_config_template);
    assert_eq!(
        args.report.as_deref(),
        Some(std::path::Path::new("--print-config-template"))
    );
}

#[test]
fn an_unknown_completion_shell_is_rejected_with_the_valid_ones() {
    let e = try_parse(&["github-backup", "--completions", "bogus"])
        .unwrap_err()
        .to_string();
    assert!(e.contains("bogus") && e.contains("bash"), "{e}");
}

#[test]
fn every_parses_an_interval() {
    let args = parse(&["github-backup", "--every", "6h"]);
    assert_eq!(args.every, Some(std::time::Duration::from_secs(6 * 3600)));
    assert!(try_parse(&["github-backup", "--every", "6"]).is_err());
}

#[test]
fn verify_git_and_git_timeout_come_from_the_flag_or_the_config_file() {
    let args = parse(&[
        "github-backup",
        "o",
        "--verify-git",
        "--git-timeout",
        "1200",
    ]);
    assert!(args.verify_git);
    assert_eq!(args.git_timeout, Some(1200));

    let mut args = parse(&["github-backup", "o"]);
    assert!(!args.verify_git);
    assert_eq!(args.git_timeout, None);
    args.merge_config(&github_backup_types::config::ConfigFile {
        verify_git: Some(true),
        git_timeout: Some(900),
        ..Default::default()
    });
    assert!(args.verify_git);
    assert_eq!(args.git_timeout, Some(900));

    // The command line wins over the file; a 0 in the file means "default".
    let mut args = parse(&["github-backup", "o", "--git-timeout", "60"]);
    args.merge_config(&github_backup_types::config::ConfigFile {
        git_timeout: Some(900),
        ..Default::default()
    });
    assert_eq!(args.git_timeout, Some(60));
    let mut args = parse(&["github-backup", "o"]);
    args.merge_config(&github_backup_types::config::ConfigFile {
        git_timeout: Some(0),
        ..Default::default()
    });
    assert_eq!(args.git_timeout, None);
}

#[test]
fn a_zero_git_timeout_on_the_command_line_is_a_usage_error() {
    let err =
        try_parse(&["github-backup", "o", "--git-timeout", "0"]).expect_err("0 must be rejected");
    assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
}
