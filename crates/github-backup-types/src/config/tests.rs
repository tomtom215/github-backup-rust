// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

use std::path::PathBuf;

use super::*;

#[test]
fn credential_token_authorization_header_has_bearer_prefix() {
    let cred = Credential::Token("ghp_test123".to_string());
    assert_eq!(
        cred.authorization_header(),
        Some("Bearer ghp_test123".to_string())
    );
}

#[test]
fn output_config_repos_dir_ends_with_expected_suffix() {
    let cfg = OutputConfig::new("/backup");
    let path = cfg.repos_dir("octocat");
    assert_eq!(path, PathBuf::from("/backup/octocat/git/repos"));
}

#[test]
fn output_config_repo_meta_dir_ends_with_expected_suffix() {
    let cfg = OutputConfig::new("/backup");
    let path = cfg.repo_meta_dir("octocat", "Hello-World");
    assert_eq!(
        path,
        PathBuf::from("/backup/octocat/json/repos/Hello-World")
    );
}

#[test]
fn backup_options_all_enables_repositories() {
    let opts = BackupOptions::all();
    assert!(opts.repositories);
    assert!(opts.issues);
    assert!(opts.pulls);
    assert!(opts.releases);
}

#[test]
fn backup_options_default_disables_all_categories() {
    let opts = BackupOptions::default();
    assert!(!opts.repositories);
    assert!(!opts.issues);
    assert!(!opts.pulls);
}

#[test]
fn backup_options_roundtrip_json() {
    let opts = BackupOptions::all();
    let json = serde_json::to_string(&opts).expect("serialise");
    let decoded: BackupOptions = serde_json::from_str(&json).expect("deserialise");
    assert_eq!(decoded.repositories, opts.repositories);
    assert_eq!(decoded.issues, opts.issues);
}

#[test]
fn clone_type_default_is_mirror() {
    assert_eq!(CloneType::default(), CloneType::Mirror);
}

#[test]
fn clone_type_serialises_as_lowercase_string() {
    let json = serde_json::to_string(&CloneType::Mirror).unwrap();
    assert_eq!(json, r#""mirror""#);
    let json = serde_json::to_string(&CloneType::Bare).unwrap();
    assert_eq!(json, r#""bare""#);
    let json = serde_json::to_string(&CloneType::Full).unwrap();
    assert_eq!(json, r#""full""#);
}

#[test]
fn clone_type_shallow_serialises_with_depth() {
    let json = serde_json::to_string(&CloneType::Shallow(10)).unwrap();
    assert_eq!(json, r#"{"shallow":10}"#);
}

#[test]
fn clone_type_roundtrips_json() {
    for ct in [
        CloneType::Mirror,
        CloneType::Bare,
        CloneType::Full,
        CloneType::Shallow(5),
    ] {
        let json = serde_json::to_string(&ct).unwrap();
        let decoded: CloneType = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, ct);
    }
}

#[test]
fn backup_options_clone_type_defaults_to_mirror() {
    let opts = BackupOptions::default();
    assert_eq!(opts.clone_type, CloneType::Mirror);
}

#[test]
fn config_file_from_toml_str_parses_all_fields() {
    let toml = r#"
owner = "octocat"
output = "/var/backup"
concurrency = 8
repositories = true
issues = true
pulls = true
"#;
    let cfg = ConfigFile::from_toml_str(toml).expect("parse");
    assert_eq!(cfg.owner.as_deref(), Some("octocat"));
    assert_eq!(cfg.output, Some(PathBuf::from("/var/backup")));
    assert_eq!(cfg.concurrency, Some(8));
    assert_eq!(cfg.repositories, Some(true));
    assert_eq!(cfg.issues, Some(true));
    assert_eq!(cfg.pulls, Some(true));
}

#[test]
fn config_file_api_url_parsed() {
    let toml = r#"
owner = "myorg"
api_url = "https://github.example.com/api/v3"
"#;
    let cfg = ConfigFile::from_toml_str(toml).expect("parse");
    assert_eq!(
        cfg.api_url.as_deref(),
        Some("https://github.example.com/api/v3")
    );
}

#[test]
fn config_file_from_toml_str_partial_config() {
    let toml = r#"owner = "octocat""#;
    let cfg = ConfigFile::from_toml_str(toml).expect("parse");
    assert_eq!(cfg.owner.as_deref(), Some("octocat"));
    assert!(cfg.repositories.is_none());
}

#[test]
fn config_file_from_toml_str_empty_is_valid() {
    let cfg = ConfigFile::from_toml_str("").expect("empty config is valid");
    assert!(cfg.owner.is_none());
}

#[test]
fn config_file_from_toml_str_invalid_returns_error() {
    let result = ConfigFile::from_toml_str("owner = {not a string}");
    assert!(result.is_err());
}

#[test]
fn config_file_default_has_all_none() {
    let cfg = ConfigFile::default();
    assert!(cfg.owner.is_none());
    assert!(cfg.token.is_none());
    assert!(cfg.output.is_none());
    assert!(cfg.concurrency.is_none());
    assert!(cfg.api_url.is_none());
}

#[test]
fn credential_anonymous_has_no_authorization_header() {
    let cred = Credential::Anonymous;
    assert_eq!(cred.authorization_header(), None);
}

#[test]
fn backup_options_all_enables_new_categories() {
    let opts = BackupOptions::all();
    assert!(opts.topics);
    assert!(opts.branches);
    assert!(opts.include_repos.is_empty());
    assert!(opts.exclude_repos.is_empty());
    assert!(opts.since.is_none());
}

#[test]
fn config_file_rejects_unknown_fields() {
    // Gap #11: typos in config keys must be caught, not silently ignored.
    let toml = r#"
owner = "octocat"
repostiories = true
"#;
    let result = ConfigFile::from_toml_str(toml);
    assert!(
        result.is_err(),
        "unknown field 'repostiories' must cause a parse error"
    );
}

#[test]
fn config_file_rejects_completely_unknown_keys() {
    let toml = r#"unknown_option = "value""#;
    assert!(ConfigFile::from_toml_str(toml).is_err());
}
