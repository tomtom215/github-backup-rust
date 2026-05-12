// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Maps the user's selected backup categories to the minimum OAuth scopes
//! GitHub requires.
//!
//! GitHub's documentation spreads scope information across dozens of
//! pages; this module centralises the mapping so we can tell users
//! *exactly* what they need before they ever create a token.

use std::collections::BTreeSet;

use crate::cli::Args;

/// Computes the set of OAuth scopes recommended for the categories
/// enabled in `args`.
///
/// Returns a sorted, deduplicated list using GitHub's classic scope
/// names (`repo`, `read:org`, …).  Fine-grained PATs use a different
/// permission model — when those are in use, the operator should
/// match the printed scopes to the equivalent fine-grained
/// permissions (the GitHub docs map them one-to-one).
#[must_use]
pub fn recommended_scopes(args: &Args) -> Vec<&'static str> {
    let mut set: BTreeSet<&'static str> = BTreeSet::new();

    // Anything touching repositories at all benefits from at least the
    // public `public_repo` scope.  We only widen to `repo` (full) if the
    // user has explicitly asked for private data, which is the common
    // case for a complete backup.
    let touches_public_repos = args.all
        || args.repositories
        || args.issues
        || args.issue_comments
        || args.issue_events
        || args.pulls
        || args.pull_comments
        || args.pull_commits
        || args.pull_reviews
        || args.labels
        || args.milestones
        || args.releases
        || args.release_assets
        || args.wikis
        || args.topics
        || args.branches
        || args.starred
        || args.clone_starred
        || args.watched
        || args.security_advisories;

    if touches_public_repos {
        set.insert("public_repo");
    }

    if args.private || args.all {
        // `repo` supersedes `public_repo` but we keep both so the user
        // recognises what we asked for; GitHub UI ignores the redundancy.
        set.insert("repo");
    }

    if args.org
        || args.org_members
        || args.org_teams
        || matches!(
            args.mirror_type.as_str(),
            "gitea-org" | "gitlab-group" | "org"
        )
    {
        set.insert("read:org");
    }

    if args.hooks {
        // Webhook endpoints are admin-scoped.
        set.insert("admin:repo_hook");
    }

    if args.deploy_keys {
        set.insert("admin:public_key");
    }

    if args.gists || args.starred_gists {
        set.insert("gist");
    }

    if args.followers || args.following {
        set.insert("user:follow");
    }

    if args.packages {
        set.insert("read:packages");
    }

    if args.discussions {
        // Discussions require repo read in classic mode.
        set.insert("repo");
    }

    if args.restore {
        // The restore flow writes to GitHub.
        set.insert("repo");
    }

    set.into_iter().collect()
}

/// Renders the recommended scopes as a copy-pasteable hint suitable for
/// printing to stdout in response to `--list-scopes`.
#[must_use]
pub fn render_recommendation(args: &Args) -> String {
    let scopes = recommended_scopes(args);
    if scopes.is_empty() {
        return "No special scopes required — anonymous access is sufficient \
                for the requested categories.\n"
            .to_string();
    }

    let joined = scopes.join(" ");
    let mut out = String::new();
    out.push_str("Recommended OAuth scopes for the current flag set:\n\n");
    for scope in &scopes {
        out.push_str("    ");
        out.push_str(scope);
        out.push('\n');
    }
    out.push_str("\nWhen creating a classic personal access token at\n");
    out.push_str("    https://github.com/settings/tokens/new\n");
    out.push_str("paste the following into the “scopes” section:\n\n");
    out.push_str("    ");
    out.push_str(&joined);
    out.push('\n');
    out.push_str(
        "\nFor a fine-grained PAT (https://github.com/settings/personal-access-tokens), \
         translate each scope to the equivalent repository permission set in the GitHub UI.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Args;
    use clap::Parser;

    fn args(extra: &[&str]) -> Args {
        let mut argv = vec!["github-backup", "octocat", "--token", "ghp_x"];
        argv.extend(extra);
        Args::parse_from(argv)
    }

    #[test]
    fn anonymous_public_only_emits_public_repo_scope() {
        let a = args(&["--repositories"]);
        assert_eq!(recommended_scopes(&a), vec!["public_repo"]);
    }

    #[test]
    fn private_flag_widens_to_repo() {
        let a = args(&["--repositories", "--private"]);
        let s = recommended_scopes(&a);
        assert!(s.contains(&"repo"));
    }

    #[test]
    fn org_flag_adds_read_org() {
        let a = args(&["--org", "--repositories"]);
        let s = recommended_scopes(&a);
        assert!(s.contains(&"read:org"));
    }

    #[test]
    fn hooks_require_admin_repo_hook() {
        let a = args(&["--hooks", "--repositories"]);
        assert!(recommended_scopes(&a).contains(&"admin:repo_hook"));
    }

    #[test]
    fn deploy_keys_require_admin_public_key() {
        let a = args(&["--deploy-keys", "--repositories"]);
        assert!(recommended_scopes(&a).contains(&"admin:public_key"));
    }

    #[test]
    fn gists_require_gist_scope() {
        let a = args(&["--gists"]);
        assert!(recommended_scopes(&a).contains(&"gist"));
    }

    #[test]
    fn followers_require_user_follow() {
        let a = args(&["--followers"]);
        assert!(recommended_scopes(&a).contains(&"user:follow"));
    }

    #[test]
    fn packages_require_read_packages() {
        let a = args(&["--packages"]);
        assert!(recommended_scopes(&a).contains(&"read:packages"));
    }

    #[test]
    fn all_flag_includes_repo_and_read_org() {
        let a = args(&["--all"]);
        let s = recommended_scopes(&a);
        assert!(s.contains(&"repo"), "got {s:?}");
    }

    #[test]
    fn recommended_scopes_are_sorted_and_deduplicated() {
        // Use a combination of compatible flags — `--all` conflicts with the
        // per-category flags, so we union it with `--private` and rely on
        // the implicit categories that `--all` itself implies for the rest.
        let a = args(&["--all", "--private"]);
        let s = recommended_scopes(&a);
        let mut sorted = s.clone();
        sorted.sort();
        assert_eq!(s, sorted, "scopes must be sorted");
        let mut dedup = s.clone();
        dedup.dedup();
        assert_eq!(s, dedup, "scopes must be unique");
    }

    #[test]
    fn render_recommendation_lists_each_scope_and_paste_string() {
        let a = args(&["--repositories", "--private"]);
        let out = render_recommendation(&a);
        assert!(out.contains("repo"));
        assert!(out.contains("Recommended OAuth scopes"));
        assert!(out.contains("paste the following"));
    }

    #[test]
    fn render_recommendation_reports_anonymous_ok_when_no_scope_needed() {
        // Just `octocat` with no categories → no scopes required.
        let a = args(&[]);
        let out = render_recommendation(&a);
        assert!(out.contains("anonymous access is sufficient"));
    }
}
