// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Property-based serde round-trip tests for all public types.
//!
//! Each test generates arbitrary instances of a type, serialises to JSON, and
//! deserialises back, asserting equality with the original. This catches
//! asymmetric Serialize/Deserialize implementations and field rename mismatches.

use proptest::prelude::*;

use github_backup_types::{
    config::{BackupOptions, BackupTarget, CloneType},
    gist::{Gist, GistFile},
    hook::Hook,
    issue::{EventLabel, EventMilestone, IssueComment, IssueEvent, IssuePullRequestRef},
    label::Label,
    milestone::Milestone,
    pull_request::{
        CommitDetail, GitIdentity, PullRequestComment, PullRequestCommit, PullRequestRef,
        PullRequestRepo, PullRequestReview,
    },
    release::{Release, ReleaseAsset},
    repository::Repository,
    security_advisory::{SecurityAdvisory, Vulnerability, VulnerablePackage},
    user::User,
};

// ── Leaf strategies ────────────────────────────────────────────────────────────

prop_compose! {
    fn arb_user()(
        id in 1u64..u32::MAX as u64,
        login in "[a-z][a-z0-9-]{0,19}",
        user_type in prop_oneof![Just("User"), Just("Organization"), Just("Bot")],
        avatar_url in "[a-z]{5,10}",
        html_url in "[a-z]{5,10}",
    ) -> User {
        User {
            id,
            login,
            user_type: user_type.to_string(),
            avatar_url: format!("https://example.com/{avatar_url}"),
            html_url: format!("https://github.com/{html_url}"),
        }
    }
}

prop_compose! {
    fn arb_label()(
        id in 1u64..u32::MAX as u64,
        name in "[a-z][a-z0-9-]{1,19}",
        color in "[0-9a-f]{6}",
        description in prop::option::of("[a-zA-Z ]{1,40}"),
        default in any::<bool>(),
    ) -> Label {
        Label { id, name, color, description, default }
    }
}

prop_compose! {
    fn arb_milestone()(
        id in 1u64..u32::MAX as u64,
        number in 1u64..1000u64,
        title in "[a-zA-Z0-9 .]{1,30}",
        description in prop::option::of("[a-zA-Z ]{1,40}"),
        state in prop_oneof![Just("open"), Just("closed")],
        creator in prop::option::of(arb_user()),
        open_issues in 0u64..100u64,
        closed_issues in 0u64..100u64,
        due_on in prop::option::of(Just("2025-01-01T00:00:00Z")),
        closed_at in prop::option::of(Just("2024-12-31T23:59:59Z")),
    ) -> Milestone {
        Milestone {
            id,
            number,
            title,
            description,
            state: state.to_string(),
            creator,
            open_issues,
            closed_issues,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-06-01T00:00:00Z".to_string(),
            due_on: due_on.map(str::to_string),
            closed_at: closed_at.map(str::to_string),
        }
    }
}

prop_compose! {
    fn arb_release_asset()(
        id in 1u64..u32::MAX as u64,
        name in "[a-z][a-z0-9_-]{1,19}\\.[a-z]{2,4}",
        content_type in prop_oneof![
            Just("application/gzip"),
            Just("application/zip"),
            Just("application/octet-stream"),
        ],
        state in prop_oneof![Just("uploaded"), Just("open")],
        size in 0u64..10_000_000u64,
        download_count in 0u64..100_000u64,
    ) -> ReleaseAsset {
        ReleaseAsset {
            id,
            name: name.clone(),
            content_type: content_type.to_string(),
            state: state.to_string(),
            size,
            download_count,
            url: format!("https://api.github.com/repos/owner/repo/releases/assets/{id}"),
            browser_download_url: format!("https://github.com/owner/repo/releases/download/v1.0/{name}"),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }
}

prop_compose! {
    fn arb_gist_file()(
        filename in "[a-z][a-z0-9_-]{1,15}\\.(rs|py|js|txt)",
        mime_type in prop_oneof![Just("text/plain"), Just("text/x-rust"), Just("application/json")],
        language in prop::option::of(prop_oneof![
            Just("Rust"), Just("Python"), Just("JavaScript"),
        ]),
        size in 0u64..100_000u64,
        truncated in prop::option::of(any::<bool>()),
    ) -> GistFile {
        GistFile {
            filename: filename.clone(),
            mime_type: mime_type.to_string(),
            language: language.map(ToString::to_string),
            size,
            truncated,
        }
    }
}

prop_compose! {
    fn arb_pr_repo()(
        id in 1u64..u32::MAX as u64,
        full_name in "[a-z]{3,10}/[a-z]{3,10}",
        private in any::<bool>(),
    ) -> PullRequestRepo {
        PullRequestRepo {
            id,
            full_name: full_name.clone(),
            clone_url: format!("https://github.com/{full_name}.git"),
            private,
        }
    }
}

prop_compose! {
    fn arb_pr_ref()(
        label in "[a-z]{3,10}:[a-z]{3,10}",
        ref_name in "[a-z][a-z0-9-]{2,14}",
        sha in "[0-9a-f]{40}",
        repo in prop::option::of(arb_pr_repo()),
    ) -> PullRequestRef {
        PullRequestRef { label, ref_name, sha, repo }
    }
}

prop_compose! {
    fn arb_git_identity()(
        name in "[A-Za-z ]{3,30}",
        email in "[a-z]{3,10}@[a-z]{3,10}\\.[a-z]{2,4}",
    ) -> GitIdentity {
        GitIdentity {
            name,
            email,
            date: "2024-01-01T00:00:00Z".to_string(),
        }
    }
}

prop_compose! {
    fn arb_commit_detail()(
        message in "[A-Za-z ]{5,60}",
        author in arb_git_identity(),
        committer in arb_git_identity(),
    ) -> CommitDetail {
        CommitDetail { message, author, committer }
    }
}

prop_compose! {
    fn arb_vulnerable_package()(
        ecosystem in prop_oneof![Just("npm"), Just("pip"), Just("cargo"), Just("maven")],
        name in "[a-z][a-z0-9-]{2,19}",
    ) -> VulnerablePackage {
        VulnerablePackage {
            ecosystem: ecosystem.to_string(),
            name,
        }
    }
}

prop_compose! {
    fn arb_vulnerability()(
        package in arb_vulnerable_package(),
        vulnerable_version_range in prop::option::of("< 1\\.2\\.3"),
        first_patched_version in prop::option::of(Just("1.2.3")),
        severity in prop_oneof![
            Just("critical"), Just("high"), Just("medium"), Just("low"),
        ],
    ) -> Vulnerability {
        Vulnerability {
            package,
            vulnerable_version_range,
            first_patched_version: first_patched_version.map(ToString::to_string),
            severity: severity.to_string(),
        }
    }
}

// ── Round-trip tests ───────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn user_roundtrip(user in arb_user()) {
        let json = serde_json::to_string(&user).expect("serialise");
        let decoded: User = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(user, decoded);
    }

    #[test]
    fn label_roundtrip(label in arb_label()) {
        let json = serde_json::to_string(&label).expect("serialise");
        let decoded: Label = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(label, decoded);
    }

    #[test]
    fn milestone_roundtrip(ms in arb_milestone()) {
        let json = serde_json::to_string(&ms).expect("serialise");
        let decoded: Milestone = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(ms, decoded);
    }

    #[test]
    fn repository_roundtrip(
        id in 1u64..u32::MAX as u64,
        name in "[a-z][a-z0-9-]{2,19}",
        owner in arb_user(),
        private in any::<bool>(),
        fork in any::<bool>(),
        archived in any::<bool>(),
        disabled in any::<bool>(),
        description in prop::option::of("[a-zA-Z ]{1,40}"),
        default_branch in prop_oneof![Just("main"), Just("master"), Just("dev")],
        size in 0u64..1_000_000u64,
        has_issues in any::<bool>(),
        has_wiki in any::<bool>(),
        pushed_at in prop::option::of(Just("2024-06-01T00:00:00Z")),
    ) {
        let repo = Repository {
            id,
            full_name: format!("{}/{name}", owner.login),
            name: name.clone(),
            owner: owner.clone(),
            private,
            fork,
            archived,
            disabled,
            description,
            clone_url: format!("https://github.com/{}/{name}.git", owner.login),
            ssh_url: format!("git@github.com:{}/{name}.git", owner.login),
            default_branch: default_branch.to_string(),
            size,
            has_issues,
            has_wiki,
            created_at: "2020-01-01T00:00:00Z".to_string(),
            pushed_at: pushed_at.map(str::to_string),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            html_url: format!("https://github.com/{}/{name}", owner.login),
        };
        let json = serde_json::to_string(&repo).expect("serialise");
        let decoded: Repository = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(repo, decoded);
    }

    #[test]
    fn release_asset_roundtrip(asset in arb_release_asset()) {
        let json = serde_json::to_string(&asset).expect("serialise");
        let decoded: ReleaseAsset = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(asset, decoded);
    }

    #[test]
    fn release_roundtrip(
        id in 1u64..u32::MAX as u64,
        tag_name in "v[0-9]\\.[0-9]\\.[0-9]",
        name in prop::option::of("[a-zA-Z0-9 ]{1,30}"),
        body in prop::option::of("[a-zA-Z .]{1,80}"),
        draft in any::<bool>(),
        prerelease in any::<bool>(),
        author in arb_user(),
        assets in prop::collection::vec(arb_release_asset(), 0..4),
        published_at in prop::option::of(Just("2024-01-01T00:00:00Z")),
        tarball_url in prop::option::of(Just("https://api.github.com/repos/owner/repo/tarball/v1.0")),
        zipball_url in prop::option::of(Just("https://api.github.com/repos/owner/repo/zipball/v1.0")),
    ) {
        let release = Release {
            id,
            tag_name: tag_name.clone(),
            name,
            body,
            draft,
            prerelease,
            author,
            assets,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            published_at: published_at.map(str::to_string),
            html_url: format!("https://github.com/owner/repo/releases/tag/{tag_name}"),
            tarball_url: tarball_url.map(str::to_string),
            zipball_url: zipball_url.map(str::to_string),
        };
        let json = serde_json::to_string(&release).expect("serialise");
        let decoded: Release = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(release, decoded);
    }

    #[test]
    fn issue_pull_request_ref_roundtrip(
        url in "[a-z]{5,15}",
        html_url in "[a-z]{5,15}",
    ) {
        let val = IssuePullRequestRef {
            url: format!("https://api.github.com/{url}"),
            html_url: format!("https://github.com/{html_url}"),
        };
        let json = serde_json::to_string(&val).expect("serialise");
        let decoded: IssuePullRequestRef = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(val, decoded);
    }

    #[test]
    fn issue_comment_roundtrip(
        id in 1u64..u32::MAX as u64,
        user in arb_user(),
        body in prop::option::of("[a-zA-Z ]{1,80}"),
    ) {
        let comment = IssueComment {
            id,
            user,
            body,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            html_url: "https://github.com/owner/repo/issues/1#issuecomment-1".to_string(),
        };
        let json = serde_json::to_string(&comment).expect("serialise");
        let decoded: IssueComment = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(comment, decoded);
    }

    #[test]
    fn event_label_roundtrip(
        name in "[a-z][a-z0-9-]{1,19}",
        color in "[0-9a-f]{6}",
    ) {
        let val = EventLabel { name, color };
        let json = serde_json::to_string(&val).expect("serialise");
        let decoded: EventLabel = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(val, decoded);
    }

    #[test]
    fn event_milestone_roundtrip(title in "[a-zA-Z0-9 .]{1,30}") {
        let val = EventMilestone { title };
        let json = serde_json::to_string(&val).expect("serialise");
        let decoded: EventMilestone = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(val, decoded);
    }

    #[test]
    fn issue_event_roundtrip(
        id in 1u64..u32::MAX as u64,
        actor in prop::option::of(arb_user()),
        event in prop_oneof![
            Just("closed"), Just("reopened"), Just("labeled"),
            Just("assigned"), Just("milestoned"),
        ],
        label in prop::option::of(("[a-z]{3,10}", "[0-9a-f]{6}").prop_map(|(n, c)| EventLabel { name: n, color: c })),
        assignee in prop::option::of(arb_user()),
        milestone in prop::option::of("[a-z]{3,15}".prop_map(|t| EventMilestone { title: t })),
    ) {
        let val = IssueEvent {
            id,
            actor,
            event: event.to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            label,
            assignee,
            milestone,
        };
        let json = serde_json::to_string(&val).expect("serialise");
        let decoded: IssueEvent = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(val, decoded);
    }

    #[test]
    fn pr_repo_roundtrip(repo in arb_pr_repo()) {
        let json = serde_json::to_string(&repo).expect("serialise");
        let decoded: PullRequestRepo = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(repo, decoded);
    }

    #[test]
    fn pr_ref_roundtrip(pr_ref in arb_pr_ref()) {
        let json = serde_json::to_string(&pr_ref).expect("serialise");
        let decoded: PullRequestRef = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(pr_ref, decoded);
    }

    #[test]
    fn pr_comment_roundtrip(
        id in 1u64..u32::MAX as u64,
        user in arb_user(),
        path in "[a-z][a-z0-9/._-]{2,30}",
        body in prop::option::of("[a-zA-Z ]{1,80}"),
    ) {
        let val = PullRequestComment {
            id,
            user,
            path,
            body,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            html_url: "https://github.com/owner/repo/pull/1#discussion_r1".to_string(),
        };
        let json = serde_json::to_string(&val).expect("serialise");
        let decoded: PullRequestComment = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(val, decoded);
    }

    #[test]
    fn pr_commit_roundtrip(
        sha in "[0-9a-f]{40}",
        detail in arb_commit_detail(),
        author in prop::option::of(arb_user()),
        committer in prop::option::of(arb_user()),
    ) {
        let val = PullRequestCommit { sha, commit: detail, author, committer };
        let json = serde_json::to_string(&val).expect("serialise");
        let decoded: PullRequestCommit = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(val, decoded);
    }

    #[test]
    fn pr_review_roundtrip(
        id in 1u64..u32::MAX as u64,
        user in arb_user(),
        body in prop::option::of("[a-zA-Z ]{1,80}"),
        state in prop_oneof![
            Just("APPROVED"), Just("CHANGES_REQUESTED"),
            Just("COMMENTED"), Just("DISMISSED"), Just("PENDING"),
        ],
        submitted_at in prop::option::of(Just("2024-01-01T00:00:00Z")),
        commit_id in "[0-9a-f]{40}",
    ) {
        let val = PullRequestReview {
            id,
            user,
            body,
            state: state.to_string(),
            submitted_at: submitted_at.map(str::to_string),
            commit_id,
        };
        let json = serde_json::to_string(&val).expect("serialise");
        let decoded: PullRequestReview = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(val, decoded);
    }

    #[test]
    fn gist_file_roundtrip(file in arb_gist_file()) {
        let json = serde_json::to_string(&file).expect("serialise");
        let decoded: GistFile = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(file, decoded);
    }

    #[test]
    fn gist_roundtrip(
        id in "[0-9a-f]{20}",
        description in prop::option::of("[a-zA-Z ]{1,40}"),
        public in any::<bool>(),
        owner in prop::option::of(arb_user()),
        files in prop::collection::hash_map(
            "[a-z][a-z0-9_-]{1,12}\\.(rs|py|txt)",
            arb_gist_file(),
            0..4,
        ),
        git_pull_url in "[0-9a-f]{20}",
    ) {
        // Ensure file keys match GistFile.filename to maintain consistency.
        let files: std::collections::HashMap<String, GistFile> = files
            .into_iter()
            .map(|(k, mut v)| { v.filename = k.clone(); (k, v) })
            .collect();
        let gist = Gist {
            id,
            description,
            public,
            owner,
            files,
            git_pull_url: format!("https://gist.github.com/{git_pull_url}.git"),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            html_url: format!("https://gist.github.com/{git_pull_url}"),
        };
        let json = serde_json::to_string(&gist).expect("serialise");
        let decoded: Gist = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(gist, decoded);
    }

    #[test]
    fn hook_roundtrip(
        id in 1u64..u32::MAX as u64,
        name in prop_oneof![Just("web"), Just("email"), Just("slack")],
        active in any::<bool>(),
        events in prop::collection::vec(
            prop_oneof![Just("push"), Just("pull_request"), Just("issues")],
            1..4,
        ),
        config_url in "[a-z]{5,15}",
    ) {
        let mut config = std::collections::HashMap::new();
        config.insert("url".to_string(), serde_json::Value::String(format!("https://example.com/{config_url}")));
        config.insert("content_type".to_string(), serde_json::Value::String("json".to_string()));
        let events: Vec<String> = events.into_iter().map(ToString::to_string).collect();
        let hook = Hook {
            id,
            hook_type: "Repository".to_string(),
            name: name.to_string(),
            active,
            events,
            config,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&hook).expect("serialise");
        let decoded: Hook = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(hook, decoded);
    }

    #[test]
    fn vulnerable_package_roundtrip(pkg in arb_vulnerable_package()) {
        let json = serde_json::to_string(&pkg).expect("serialise");
        let decoded: VulnerablePackage = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(pkg, decoded);
    }

    #[test]
    fn vulnerability_roundtrip(vuln in arb_vulnerability()) {
        let json = serde_json::to_string(&vuln).expect("serialise");
        let decoded: Vulnerability = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(vuln, decoded);
    }

    #[test]
    fn security_advisory_roundtrip(
        ghsa_id in "GHSA-[0-9a-z]{4}-[0-9a-z]{4}-[0-9a-z]{4}",
        cve_id in prop::option::of("CVE-[0-9]{4}-[0-9]{5}"),
        summary in "[A-Za-z ]{10,60}",
        description in prop::option::of("[A-Za-z .]{10,80}"),
        severity in prop_oneof![Just("critical"), Just("high"), Just("medium"), Just("low")],
        state in prop_oneof![Just("published"), Just("withdrawn")],
        vulnerabilities in prop::collection::vec(arb_vulnerability(), 0..4),
        published_at in prop::option::of(Just("2024-01-01T00:00:00Z")),
    ) {
        let advisory = SecurityAdvisory {
            ghsa_id: ghsa_id.clone(),
            cve_id,
            summary,
            description,
            severity: severity.to_string(),
            state: state.to_string(),
            vulnerabilities,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            published_at: published_at.map(str::to_string),
            html_url: format!("https://github.com/advisories/{ghsa_id}"),
        };
        let json = serde_json::to_string(&advisory).expect("serialise");
        let decoded: SecurityAdvisory = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(advisory, decoded);
    }

    #[test]
    fn backup_target_roundtrip(is_org in any::<bool>()) {
        let target = if is_org { BackupTarget::Org } else { BackupTarget::User };
        let json = serde_json::to_string(&target).expect("serialise");
        let decoded: BackupTarget = serde_json::from_str(&json).expect("deserialise");
        prop_assert_eq!(target, decoded);
    }

    #[test]
    fn backup_options_roundtrip(
        repositories in any::<bool>(),
        forks in any::<bool>(),
        private in any::<bool>(),
        prefer_ssh in any::<bool>(),
        lfs in any::<bool>(),
        no_prune in any::<bool>(),
        issues in any::<bool>(),
        issue_comments in any::<bool>(),
        issue_events in any::<bool>(),
        pulls in any::<bool>(),
        pull_comments in any::<bool>(),
        pull_commits in any::<bool>(),
        pull_reviews in any::<bool>(),
        labels in any::<bool>(),
        milestones in any::<bool>(),
        releases in any::<bool>(),
        release_assets in any::<bool>(),
        hooks in any::<bool>(),
        security_advisories in any::<bool>(),
        wikis in any::<bool>(),
        starred in any::<bool>(),
        watched in any::<bool>(),
        followers in any::<bool>(),
        following in any::<bool>(),
        gists in any::<bool>(),
        starred_gists in any::<bool>(),
        deploy_keys in any::<bool>(),
        collaborators in any::<bool>(),
        org_members in any::<bool>(),
        org_teams in any::<bool>(),
        dry_run in any::<bool>(),
        concurrency in 1usize..16usize,
        is_org in any::<bool>(),
    ) {
        let target = if is_org { BackupTarget::Org } else { BackupTarget::User };
        let opts = BackupOptions {
            target,
            repositories, forks, private, prefer_ssh,
            clone_type: CloneType::Mirror,
            lfs, no_prune,
            issues, issue_comments, issue_events,
            pulls, pull_comments, pull_commits, pull_reviews,
            labels, milestones, releases, release_assets,
            hooks, security_advisories, wikis,
            starred, watched, followers, following,
            gists, starred_gists,
            topics: false,
            branches: false,
            deploy_keys,
            collaborators,
            org_members,
            org_teams,
            include_repos: vec![],
            exclude_repos: vec![],
            since: None,
            dry_run, concurrency,
        };
        let json = serde_json::to_string(&opts).expect("serialise");
        let decoded: BackupOptions = serde_json::from_str(&json).expect("deserialise");
        // Compare field by field since BackupOptions doesn't derive PartialEq
        prop_assert_eq!(decoded.repositories, opts.repositories);
        prop_assert_eq!(decoded.forks, opts.forks);
        prop_assert_eq!(decoded.private, opts.private);
        prop_assert_eq!(decoded.prefer_ssh, opts.prefer_ssh);
        prop_assert_eq!(decoded.lfs, opts.lfs);
        prop_assert_eq!(decoded.no_prune, opts.no_prune);
        prop_assert_eq!(decoded.issues, opts.issues);
        prop_assert_eq!(decoded.issue_comments, opts.issue_comments);
        prop_assert_eq!(decoded.issue_events, opts.issue_events);
        prop_assert_eq!(decoded.pulls, opts.pulls);
        prop_assert_eq!(decoded.pull_comments, opts.pull_comments);
        prop_assert_eq!(decoded.pull_commits, opts.pull_commits);
        prop_assert_eq!(decoded.pull_reviews, opts.pull_reviews);
        prop_assert_eq!(decoded.labels, opts.labels);
        prop_assert_eq!(decoded.milestones, opts.milestones);
        prop_assert_eq!(decoded.releases, opts.releases);
        prop_assert_eq!(decoded.release_assets, opts.release_assets);
        prop_assert_eq!(decoded.hooks, opts.hooks);
        prop_assert_eq!(decoded.security_advisories, opts.security_advisories);
        prop_assert_eq!(decoded.wikis, opts.wikis);
        prop_assert_eq!(decoded.starred, opts.starred);
        prop_assert_eq!(decoded.watched, opts.watched);
        prop_assert_eq!(decoded.followers, opts.followers);
        prop_assert_eq!(decoded.following, opts.following);
        prop_assert_eq!(decoded.gists, opts.gists);
        prop_assert_eq!(decoded.starred_gists, opts.starred_gists);
        prop_assert_eq!(decoded.topics, opts.topics);
        prop_assert_eq!(decoded.branches, opts.branches);
        prop_assert_eq!(decoded.deploy_keys, opts.deploy_keys);
        prop_assert_eq!(decoded.collaborators, opts.collaborators);
        prop_assert_eq!(decoded.org_members, opts.org_members);
        prop_assert_eq!(decoded.org_teams, opts.org_teams);
        prop_assert_eq!(decoded.dry_run, opts.dry_run);
        prop_assert_eq!(decoded.concurrency, opts.concurrency);
    }
}
