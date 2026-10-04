// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Maps the user's selected backup categories to the minimum OAuth scopes
//! GitHub requires.
//!
//! GitHub's documentation spreads scope information across dozens of
//! pages; this module centralises the mapping so we can tell users
//! *exactly* what they need before they ever create a token.

use std::collections::BTreeSet;

use github_backup_types::config::{BackupOptions, BackupTarget};

use crate::cli::Args;

/// Computes the set of classic OAuth scopes recommended for the categories
/// enabled in `args`, **after** `--all` and the config file have been applied
/// (the individual flags of `args` do not say what `--all` turned on).
///
/// Returns a sorted, deduplicated list.  The mapping follows GitHub's REST
/// documentation for each endpoint; it has not been exercised against live
/// GitHub, and fine-grained tokens use a different permission model, so treat
/// it as a starting point and run `--doctor` with the token you create.
#[must_use]
pub fn recommended_scopes(args: &Args) -> Vec<&'static str> {
    // `into_backup_options` consumes its receiver and needs an owner.
    let mut resolved = args.clone();
    if resolved.owner.is_none() {
        resolved.owner = Some(String::new());
    }
    let (_, _, opts) = resolved.into_backup_options();
    let mut set = scopes_for(&opts);
    if args.restore {
        // The restore flow writes to GitHub.
        set.insert("repo");
    }
    if matches!(
        args.mirror_type.as_str(),
        "gitea-org" | "gitlab-group" | "org"
    ) {
        set.insert("read:org");
    }
    finish(set)
}

/// Removes `public_repo` when `repo` (which includes it) is present.
fn finish(mut set: BTreeSet<&'static str>) -> Vec<&'static str> {
    if set.contains("repo") {
        set.remove("public_repo");
    }
    set.into_iter().collect()
}

fn scopes_for(opts: &BackupOptions) -> BTreeSet<&'static str> {
    let mut set: BTreeSet<&'static str> = BTreeSet::new();

    // Reading public repositories and their issues, pull requests, releases
    // and so on needs no scope at all, but `public_repo` is the narrowest
    // scope that also lifts the unauthenticated rate limit and is what a
    // public-only token is created with.
    let touches_repos = opts.repositories
        || opts.issues
        || opts.issue_comments
        || opts.issue_events
        || opts.pulls
        || opts.pull_comments
        || opts.pull_commits
        || opts.pull_reviews
        || opts.labels
        || opts.milestones
        || opts.releases
        || opts.release_assets
        || opts.wikis
        || opts.topics
        || opts.branches
        || opts.starred
        || opts.clone_starred
        || opts.watched
        || opts.security_advisories
        || opts.actions
        || opts.action_runs;
    if touches_repos {
        set.insert("public_repo");
    }

    // Private repositories, and the categories that need admin access to a
    // repository (deploy keys, collaborators, environments), need `repo`.
    if opts.private || opts.deploy_keys || opts.collaborators || opts.environments {
        set.insert("repo");
    }

    if opts.hooks {
        // Webhook endpoints are admin-scoped.
        set.insert("admin:repo_hook");
    }

    if matches!(opts.target, BackupTarget::Org) || opts.org_members || opts.org_teams {
        set.insert("read:org");
    }

    if opts.gists || opts.starred_gists {
        set.insert("gist");
    }

    if opts.packages {
        set.insert("read:packages");
    }

    // Followers and following are public data: no scope.
    set
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
    use crate::cli::test_support::parse;
    use crate::cli::Args;

    fn args(extra: &[&str]) -> Args {
        let mut argv = vec!["github-backup", "octocat", "--token", "ghp_x"];
        argv.extend(extra);
        parse(&argv)
    }

    #[test]
    fn anonymous_public_only_emits_public_repo_scope() {
        let a = args(&["--repositories"]);
        assert_eq!(recommended_scopes(&a), vec!["public_repo"]);
    }

    #[test]
    fn private_flag_widens_to_repo_and_drops_the_redundant_public_repo() {
        let a = args(&["--repositories", "--private"]);
        assert_eq!(recommended_scopes(&a), vec!["repo"]);
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

    /// `admin:public_key` is for a user's SSH/GPG keys, not a repository's
    /// deploy keys (which need access to the repository: `repo`).
    #[test]
    fn deploy_keys_need_repo_not_admin_public_key() {
        let s = recommended_scopes(&args(&["--deploy-keys", "--repositories"]));
        assert!(s.contains(&"repo"), "{s:?}");
        assert!(!s.contains(&"admin:public_key"), "{s:?}");
    }

    #[test]
    fn gists_require_gist_scope() {
        let a = args(&["--gists"]);
        assert!(recommended_scopes(&a).contains(&"gist"));
    }

    /// `user:follow` lets a token *follow* people; reading followers needs
    /// nothing, and asking for it over-grants.
    #[test]
    fn followers_need_no_scope() {
        let s = recommended_scopes(&args(&["--followers", "--following"]));
        assert!(s.is_empty(), "{s:?}");
    }

    #[test]
    fn packages_require_read_packages() {
        let a = args(&["--packages"]);
        assert!(recommended_scopes(&a).contains(&"read:packages"));
    }

    /// Regression (docs audit): `--all` printed only `public_repo repo`,
    /// because the per-category flags of `args` are false when `--all` is used.
    #[test]
    fn all_flag_includes_everything_it_enables() {
        let s = recommended_scopes(&args(&["--all"]));
        for needed in ["repo", "gist", "read:packages", "admin:repo_hook"] {
            assert!(s.contains(&needed), "--all must recommend {needed}: {s:?}");
        }
        assert!(!s.contains(&"public_repo"), "repo supersedes it: {s:?}");
        assert!(
            !s.contains(&"user:follow") && !s.contains(&"admin:public_key"),
            "{s:?}"
        );
    }

    #[test]
    fn all_for_an_organisation_adds_read_org() {
        let s = recommended_scopes(&args(&["--all", "--org"]));
        assert!(s.contains(&"read:org"), "{s:?}");
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
