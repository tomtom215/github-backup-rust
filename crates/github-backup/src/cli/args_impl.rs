// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! `impl Args` — config-file merge and conversion to `BackupOptions`.

use super::args::Args;

impl Args {
    /// Merges a loaded `ConfigFile` into this [`Args`], with CLI values taking
    /// precedence over config file values.
    ///
    /// Call this after parsing CLI args but before calling
    /// [`into_backup_options`][Args::into_backup_options].
    pub fn merge_config(&mut self, cfg: &github_backup_types::config::ConfigFile) {
        // Owner: config file wins only if CLI did not provide it.
        if self.owner.is_none() {
            if let Some(ref o) = cfg.owner {
                self.owner = Some(o.clone());
            }
        }
        // Token: CLI / env takes precedence.
        if self.token.is_none() {
            if let Some(ref t) = cfg.token {
                self.token = Some(t.clone());
            }
        }
        // Output dir.
        if self.output.is_none() {
            if let Some(ref p) = cfg.output {
                self.output = Some(p.clone());
            }
        }
        // Concurrency: CLI takes precedence; config supplies the default when
        // the flag was not explicitly provided on the command line.
        if self.concurrency.is_none() {
            if let Some(c) = cfg.concurrency {
                self.concurrency = Some(c);
            }
        }
        // api_url: CLI / env takes precedence.
        if self.api_url.is_none() {
            if let Some(ref u) = cfg.api_url {
                self.api_url = Some(u.clone());
            }
        }
        // clone_host: CLI / env takes precedence.
        if self.clone_host.is_none() {
            if let Some(ref h) = cfg.clone_host {
                self.clone_host = Some(h.clone());
            }
        }
        // Boolean categories: config activates them, CLI can also activate.
        self.repositories |= cfg.repositories.unwrap_or(false);
        self.issues |= cfg.issues.unwrap_or(false);
        self.issue_comments |= cfg.issue_comments.unwrap_or(false);
        self.issue_events |= cfg.issue_events.unwrap_or(false);
        self.pulls |= cfg.pulls.unwrap_or(false);
        self.pull_comments |= cfg.pull_comments.unwrap_or(false);
        self.pull_commits |= cfg.pull_commits.unwrap_or(false);
        self.pull_reviews |= cfg.pull_reviews.unwrap_or(false);
        self.labels |= cfg.labels.unwrap_or(false);
        self.milestones |= cfg.milestones.unwrap_or(false);
        self.releases |= cfg.releases.unwrap_or(false);
        self.release_assets |= cfg.release_assets.unwrap_or(false);
        self.hooks |= cfg.hooks.unwrap_or(false);
        self.security_advisories |= cfg.security_advisories.unwrap_or(false);
        self.wikis |= cfg.wikis.unwrap_or(false);
        self.starred |= cfg.starred.unwrap_or(false);
        self.clone_starred |= cfg.clone_starred.unwrap_or(false);
        self.watched |= cfg.watched.unwrap_or(false);
        self.followers |= cfg.followers.unwrap_or(false);
        self.following |= cfg.following.unwrap_or(false);
        self.gists |= cfg.gists.unwrap_or(false);
        self.starred_gists |= cfg.starred_gists.unwrap_or(false);
        self.forks |= cfg.forks.unwrap_or(false);
        self.private |= cfg.private.unwrap_or(false);
        self.all |= cfg.all.unwrap_or(false);
        self.topics |= cfg.topics.unwrap_or(false);
        self.branches |= cfg.branches.unwrap_or(false);
        self.deploy_keys |= cfg.deploy_keys.unwrap_or(false);
        self.collaborators |= cfg.collaborators.unwrap_or(false);
        self.org_members |= cfg.org_members.unwrap_or(false);
        self.org_teams |= cfg.org_teams.unwrap_or(false);
        self.actions |= cfg.actions.unwrap_or(false);
        self.action_runs |= cfg.action_runs.unwrap_or(false);
        self.environments |= cfg.environments.unwrap_or(false);
        // Repo filter lists: extend (union) rather than replace.
        if let Some(ref patterns) = cfg.include_repos {
            self.include_repos.extend(patterns.iter().cloned());
        }
        if let Some(ref patterns) = cfg.exclude_repos {
            self.exclude_repos.extend(patterns.iter().cloned());
        }
        // Since: CLI takes precedence; config supplies default.
        if self.since.is_none() {
            if let Some(ref s) = cfg.since {
                self.since = Some(s.clone());
            }
        }
    }

    /// Converts the parsed (and optionally merged) CLI arguments into an owner
    /// string, output path, and `BackupOptions`.
    ///
    /// # Panics
    ///
    /// Panics if no owner has been supplied (neither via positional arg nor
    /// config file). Callers should validate this before calling.
    #[must_use]
    pub fn into_backup_options(
        self,
    ) -> (
        String,
        std::path::PathBuf,
        github_backup_types::config::BackupOptions,
    ) {
        use github_backup_types::config::{BackupOptions, BackupTarget};

        let owner = self
            .owner
            .expect("owner must be set before calling into_backup_options");
        let output = self.output.unwrap_or_else(|| std::path::PathBuf::from("."));

        let target = if self.org {
            BackupTarget::Org
        } else {
            BackupTarget::User
        };

        let clone_type = self.clone_type.into_clone_type();
        let concurrency = self.concurrency.unwrap_or(4);

        if self.all {
            return (
                owner,
                output,
                BackupOptions {
                    target,
                    prefer_ssh: self.prefer_ssh,
                    clone_type,
                    lfs: self.lfs,
                    no_prune: self.no_prune,
                    dry_run: self.dry_run,
                    concurrency,
                    include_repos: self.include_repos,
                    exclude_repos: self.exclude_repos,
                    since: self.since,
                    clone_host: self.clone_host,
                    ..BackupOptions::all()
                },
            );
        }

        (
            owner,
            output,
            BackupOptions {
                target,
                repositories: self.repositories,
                forks: self.forks,
                private: self.private,
                prefer_ssh: self.prefer_ssh,
                clone_type,
                lfs: self.lfs,
                no_prune: self.no_prune,
                issues: self.issues,
                issue_comments: self.issue_comments,
                issue_events: self.issue_events,
                pulls: self.pulls,
                pull_comments: self.pull_comments,
                pull_commits: self.pull_commits,
                pull_reviews: self.pull_reviews,
                labels: self.labels,
                milestones: self.milestones,
                releases: self.releases,
                release_assets: self.release_assets,
                hooks: self.hooks,
                security_advisories: self.security_advisories,
                wikis: self.wikis,
                starred: self.starred,
                clone_starred: self.clone_starred,
                watched: self.watched,
                followers: self.followers,
                following: self.following,
                gists: self.gists,
                starred_gists: self.starred_gists,
                topics: self.topics,
                branches: self.branches,
                deploy_keys: self.deploy_keys,
                collaborators: self.collaborators,
                org_members: self.org_members,
                org_teams: self.org_teams,
                actions: self.actions,
                action_runs: self.action_runs,
                environments: self.environments,
                include_repos: self.include_repos,
                exclude_repos: self.exclude_repos,
                since: self.since,
                clone_host: self.clone_host,
                dry_run: self.dry_run,
                concurrency,
            },
        )
    }
}
