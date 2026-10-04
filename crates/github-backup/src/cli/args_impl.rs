// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! `impl Args` — process-level parsing, config-file merge and conversion to
//! `BackupOptions`.

use clap::error::ErrorKind;
use clap::parser::ValueSource;
use clap::{ArgMatches, CommandFactory, FromArgMatches};

use super::args::Args;

/// Treats a blank string option as unset and trims surrounding whitespace.
fn normalize(value: &mut Option<String>) {
    if let Some(text) = value.as_mut() {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            *value = None;
        } else if trimmed.len() != text.len() {
            *text = trimmed.to_string();
        }
    }
}

impl Args {
    /// Parses the process arguments and environment.
    ///
    /// Behaves like [`clap::Parser::parse`] (a clap-formatted message and exit
    /// status 2 on invalid input) but additionally returns the [`ArgMatches`],
    /// which [`Args::check_dependencies`] needs to tell command-line flags
    /// from environment variables, and normalises blank option values.
    pub fn parse_cli() -> (Self, ArgMatches) {
        let matches = Self::command().get_matches();
        let mut args = Self::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
        args.normalize_env_values();
        (args, matches)
    }

    /// Treats blank string options as unset and trims surrounding whitespace.
    ///
    /// Container launchers (Docker Compose, Kubernetes manifests, Unraid) pass
    /// every optional variable to the process as an *empty string*, and clap
    /// counts a set-but-empty variable as a supplied value: an empty
    /// `GITHUB_API_URL` would otherwise be used as an (invalid) API URL and an
    /// empty `GITHUB_TOKEN` as an (invalid) credential.  Whitespace — usually
    /// a trailing newline pasted along with a token — is trimmed because it
    /// would make the value an invalid HTTP header.
    pub(crate) fn normalize_env_values(&mut self) {
        for value in [
            &mut self.token,
            &mut self.oauth_client_id,
            &mut self.api_url,
            &mut self.clone_host,
            &mut self.mirror_token,
            &mut self.s3_access_key,
            &mut self.s3_secret_key,
            &mut self.encrypt_key,
            &mut self.notify_webhook,
        ] {
            normalize(value);
        }
    }

    /// Rejects command-line flags whose companion flag is missing.
    ///
    /// These pairs used to be declared with clap's `requires`, but clap
    /// applies `requires` to environment variables as well, so an ambient
    /// `AWS_ACCESS_KEY_ID` (or an empty `MIRROR_TOKEN` forwarded by Compose)
    /// made every run fail with "required arguments were not provided".  A
    /// credential that merely arrives through the environment is now ignored
    /// when its feature is not in use; one the user *typed* without the
    /// matching flag is still an error.
    ///
    /// Call after `merge_config_with` so a companion value supplied by the
    /// config file counts.
    ///
    /// # Errors
    ///
    /// Returns a clap error (print it with [`clap::Error::exit`]).
    pub fn check_dependencies(&self, matches: &ArgMatches) -> Result<(), clap::Error> {
        let typed = |id: &str| matches.value_source(id) == Some(ValueSource::CommandLine);
        let missing = |flag: &str, needs: &str| {
            Err(Self::command().error(
                ErrorKind::MissingRequiredArgument,
                format!("the argument '{flag}' requires '{needs}' to be given as well"),
            ))
        };

        if typed("s3_access_key") && self.s3_access_key.is_some() && self.s3_bucket.is_none() {
            return missing("--s3-access-key", "--s3-bucket <BUCKET>");
        }
        if typed("s3_secret_key") && self.s3_secret_key.is_some() && self.s3_bucket.is_none() {
            return missing("--s3-secret-key", "--s3-bucket <BUCKET>");
        }
        if typed("mirror_token") && self.mirror_token.is_some() && self.mirror_to.is_none() {
            return missing("--mirror-token", "--mirror-to <URL>");
        }
        if typed("oauth_client_id") && self.oauth_client_id.is_some() && !self.device_auth {
            return missing("--oauth-client-id", "--device-auth");
        }
        Ok(())
    }

    /// Merges a loaded `ConfigFile` into this [`Args`], with CLI values taking
    /// precedence over config file values.
    ///
    /// Call this after parsing CLI args but before calling
    /// [`into_backup_options`][Args::into_backup_options].
    #[cfg(test)]
    pub fn merge_config(&mut self, cfg: &github_backup_types::config::ConfigFile) {
        // Without the parse matches the best available guess is "anything but
        // the default was typed".
        let explicit = self.clone_type != crate::cli::clone_type::CliCloneType::Mirror;
        self.merge_config_inner(cfg, explicit);
    }

    /// Like `merge_config`, but uses the parse matches to
    /// tell an explicit `--clone-type mirror` from the default, so the command
    /// line always beats the config file — including when it asks for the default.
    pub fn merge_config_with(
        &mut self,
        cfg: &github_backup_types::config::ConfigFile,
        matches: &clap::ArgMatches,
    ) {
        use clap::parser::ValueSource;
        let explicit = matches!(
            matches.value_source("clone_type"),
            Some(ValueSource::CommandLine | ValueSource::EnvVariable)
        );
        self.merge_config_inner(cfg, explicit);
    }

    fn merge_config_inner(
        &mut self,
        cfg: &github_backup_types::config::ConfigFile,
        clone_type_explicit: bool,
    ) {
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
        // org: config activates it; CLI `--org` also activates it.
        self.org |= cfg.org.unwrap_or(false);
        // Clone behaviour flags.
        self.prefer_ssh |= cfg.prefer_ssh.unwrap_or(false);
        self.lfs |= cfg.lfs.unwrap_or(false);
        // `--no-prune` (deprecated) is the default now; an explicit one still
        // beats a `prune = true` in the file.
        if !self.no_prune {
            self.prune |= cfg.prune.unwrap_or(false);
        }
        // Clone type: the config supplies it only when the command line did not.
        if !clone_type_explicit {
            if let Some(ref ct) = cfg.clone_type {
                use github_backup_types::config::CloneType;
                self.clone_type = match ct {
                    CloneType::Mirror => crate::cli::clone_type::CliCloneType::Mirror,
                    CloneType::Bare => crate::cli::clone_type::CliCloneType::Bare,
                    CloneType::Full => crate::cli::clone_type::CliCloneType::Full,
                    CloneType::Shallow(d) => crate::cli::clone_type::CliCloneType::Shallow(*d),
                };
            }
        }
        // Report path: CLI takes precedence.
        if self.report.is_none() {
            if let Some(ref p) = cfg.report {
                self.report = Some(p.clone());
            }
        }
        // Mirror destination: CLI takes precedence.
        if self.mirror_to.is_none() {
            if let Some(ref u) = cfg.mirror_to {
                self.mirror_to = Some(u.clone());
            }
        }
        if self.mirror_token.is_none() {
            if let Some(ref t) = cfg.mirror_token {
                self.mirror_token = Some(t.clone());
            }
        }
        if self.mirror_owner.is_none() {
            if let Some(ref o) = cfg.mirror_owner {
                self.mirror_owner = Some(o.clone());
            }
        }
        // The command line decides when it says either; the file fills in only
        // when it is silent.  `mirror_private` wins over `mirror_public` if a
        // file sets both (see `build_mirror_dest`).
        if !self.mirror_public {
            self.mirror_private |= cfg.mirror_private.unwrap_or(false);
        }
        if !self.mirror_private {
            self.mirror_public |= cfg.mirror_public.unwrap_or(false);
        }
        // S3 storage: CLI takes precedence.
        if self.s3_bucket.is_none() {
            if let Some(ref b) = cfg.s3_bucket {
                self.s3_bucket = Some(b.clone());
            }
        }
        if self.s3_region.is_none() {
            if let Some(ref r) = cfg.s3_region {
                self.s3_region = Some(r.clone());
            }
        }
        if self.s3_prefix.is_none() {
            if let Some(ref p) = cfg.s3_prefix {
                self.s3_prefix = Some(p.clone());
            }
        }
        if self.s3_endpoint.is_none() {
            if let Some(ref e) = cfg.s3_endpoint {
                self.s3_endpoint = Some(e.clone());
            }
        }
        if self.s3_access_key.is_none() {
            if let Some(ref k) = cfg.s3_access_key {
                self.s3_access_key = Some(k.clone());
            }
        }
        if self.s3_secret_key.is_none() {
            if let Some(ref s) = cfg.s3_secret_key {
                self.s3_secret_key = Some(s.clone());
            }
        }
        self.s3_include_assets |= cfg.s3_include_assets.unwrap_or(false);
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
        self.discussions |= cfg.discussions.unwrap_or(false);
        self.projects |= cfg.projects.unwrap_or(false);
        self.packages |= cfg.packages.unwrap_or(false);
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
                    prune: self.prune,
                    dry_run: self.dry_run,
                    concurrency,
                    include_repos: self.include_repos,
                    exclude_repos: self.exclude_repos,
                    since: self.since,
                    full: self.full,
                    clone_host: self.clone_host,
                    // `--all` leaves these opt-in categories off; an explicit
                    // flag must still turn them on.
                    clone_starred: self.clone_starred,
                    action_runs: self.action_runs,
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
                prune: self.prune,
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
                discussions: self.discussions,
                projects: self.projects,
                packages: self.packages,
                include_repos: self.include_repos,
                exclude_repos: self.exclude_repos,
                since: self.since,
                full: self.full,
                clone_host: self.clone_host,
                dry_run: self.dry_run,
                concurrency,
            },
        )
    }
}
