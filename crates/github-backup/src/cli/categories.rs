// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! The backup categories by name: what `--skip` accepts, what decides whether
//! the user selected anything, and what the plan banner lists.
//!
//! A new category is one line in [`CATEGORIES`] (plus its flag and config key).

use github_backup_types::config::BackupOptions;

/// Gives access to one category switch of [`BackupOptions`].
type Switch = fn(&mut BackupOptions) -> &mut bool;

/// Every category, named as its command-line flag without `--`.
///
/// Some names do not select data on their own (see [`EXTRAS`]): they can be
/// skipped, but turning only them on does not count as selecting something
/// (see [`any_selected`]).
const CATEGORIES: &[(&str, Switch)] = &[
    ("repositories", |o| &mut o.repositories),
    ("forks", |o| &mut o.forks),
    ("private", |o| &mut o.private),
    ("wikis", |o| &mut o.wikis),
    ("issues", |o| &mut o.issues),
    ("issue-comments", |o| &mut o.issue_comments),
    ("issue-events", |o| &mut o.issue_events),
    ("pulls", |o| &mut o.pulls),
    ("pull-comments", |o| &mut o.pull_comments),
    ("pull-commits", |o| &mut o.pull_commits),
    ("pull-reviews", |o| &mut o.pull_reviews),
    ("labels", |o| &mut o.labels),
    ("milestones", |o| &mut o.milestones),
    ("releases", |o| &mut o.releases),
    ("release-assets", |o| &mut o.release_assets),
    ("hooks", |o| &mut o.hooks),
    ("security-advisories", |o| &mut o.security_advisories),
    ("topics", |o| &mut o.topics),
    ("branches", |o| &mut o.branches),
    ("deploy-keys", |o| &mut o.deploy_keys),
    ("collaborators", |o| &mut o.collaborators),
    ("starred", |o| &mut o.starred),
    ("clone-starred", |o| &mut o.clone_starred),
    ("watched", |o| &mut o.watched),
    ("followers", |o| &mut o.followers),
    ("following", |o| &mut o.following),
    ("gists", |o| &mut o.gists),
    ("starred-gists", |o| &mut o.starred_gists),
    ("org-members", |o| &mut o.org_members),
    ("org-teams", |o| &mut o.org_teams),
    ("actions", |o| &mut o.actions),
    ("action-runs", |o| &mut o.action_runs),
    ("environments", |o| &mut o.environments),
    ("discussions", |o| &mut o.discussions),
    ("projects", |o| &mut o.projects),
    ("packages", |o| &mut o.packages),
    ("commit-comments", |o| &mut o.commit_comments),
    ("attachments", |o| &mut o.attachments),
];

/// Names that modify the selection instead of making one: `forks` and
/// `private` narrow `repositories`, and the opt-in extras `clone-starred` and
/// `action-runs` add to it, so `github-backup --action-runs` is everything
/// plus the workflow run history.
const EXTRAS: &[&str] = &["forks", "private", "clone-starred", "action-runs"];

/// Every category name, for `--skip`.
pub(crate) fn names() -> impl Iterator<Item = &'static str> {
    CATEGORIES.iter().map(|(name, _)| *name)
}

/// The names of the categories `opts` enables, in [`CATEGORIES`] order.
pub(crate) fn enabled(opts: &BackupOptions) -> Vec<&'static str> {
    let mut probe = opts.clone();
    CATEGORIES
        .iter()
        .filter(|(_, switch)| *switch(&mut probe))
        .map(|(name, _)| *name)
        .collect()
}

/// Whether `opts` selects any category (the [`EXTRAS`] do not count).
pub(crate) fn any_selected(opts: &BackupOptions) -> bool {
    enabled(opts).iter().any(|name| !EXTRAS.contains(name))
}

/// Turns off every category named in `skip`.  Names are validated by the
/// argument parser and the config merge; an unknown one is ignored here.
pub(crate) fn apply_skip(opts: &mut BackupOptions, skip: &[String]) {
    for (name, switch) in CATEGORIES {
        if skip.iter().any(|s| s == name) {
            *switch(opts) = false;
        }
    }
}

/// Whether `name` is a category name.
pub(crate) fn is_known(name: &str) -> bool {
    names().any(|n| n == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_category_of_all_is_named_and_none_twice() {
        let all = BackupOptions::all();
        let on = enabled(&all);
        // `--all` leaves the two opt-in categories off.
        let off: Vec<_> = names().filter(|n| !on.contains(n)).collect();
        assert_eq!(off, ["clone-starred", "action-runs"]);
        let mut sorted: Vec<_> = names().collect();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), CATEGORIES.len(), "duplicate name");
    }

    #[test]
    fn extras_alone_do_not_count_as_a_selection() {
        let mut opts = BackupOptions {
            forks: true,
            private: true,
            clone_starred: true,
            action_runs: true,
            ..Default::default()
        };
        assert!(!any_selected(&opts));
        opts.gists = true;
        assert!(any_selected(&opts));
    }

    #[test]
    fn skip_turns_off_exactly_the_named_categories() {
        let mut opts = BackupOptions::all();
        apply_skip(&mut opts, &["hooks".into(), "release-assets".into()]);
        assert!(!opts.hooks && !opts.release_assets);
        assert!(opts.releases && opts.repositories);
    }
}
