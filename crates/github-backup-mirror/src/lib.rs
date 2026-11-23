// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Push-mirror backup to Gitea, Codeberg, Forgejo, GitLab and other
//! self-hosted Git services.
//!
//! # Overview
//!
//! After the primary GitHub backup completes, this crate can push every
//! cloned repository to a secondary Git host as a mirror.  This provides:
//!
//! - **Redundancy** — a second copy on an independent platform.
//! - **Availability** — access to the source code even if GitHub is
//!   unavailable.
//! - **Sovereignty** — full control over where your data lives.
//!
//! The mirror is a **replica** of the local backup, not an independent
//! backup: whatever the local clone holds is pushed (branches are
//! force-updated), and with pruning, branches and tags deleted locally are
//! deleted there too.
//!
//! # Supported Destinations
//!
//! Any host that implements the [Gitea REST API v1] is supported
//! ([`GiteaClient`]), including:
//!
//! - [Codeberg] — free, ad-free, non-profit hosting powered by Forgejo.
//! - [Gitea] — self-hostable, lightweight Git service.
//! - [Forgejo] — community-driven fork of Gitea.
//! - Self-hosted instances of any of the above.
//!
//! GitLab.com and self-hosted GitLab use the REST API v4 ([`GitLabClient`]).
//! Both clients speak HTTPS only, because the token travels in a header.
//!
//! [Gitea REST API v1]: https://gitea.io/api/swagger
//! [Codeberg]: https://codeberg.org
//! [Gitea]: https://gitea.io
//! [Forgejo]: https://forgejo.org
//!
//! # Usage
//!
//! ```no_run
//! use github_backup_mirror::runner::{push_mirrors, MirrorPlan};
//! use github_backup_mirror::{config::GiteaConfig, GiteaClient};
//! use std::collections::HashSet;
//! use std::path::Path;
//!
//! # async fn example() -> Result<(), github_backup_mirror::MirrorError> {
//! let client = GiteaClient::new(GiteaConfig {
//!     base_url: "https://codeberg.org".to_string(),
//!     token: std::env::var("CODEBERG_TOKEN").unwrap_or_default(),
//!     owner: "alice".to_string(),
//!     private: true,
//!     prune: false,
//! })?;
//!
//! let none = HashSet::new();
//! let stats = push_mirrors(
//!     &client,
//!     &MirrorPlan {
//!         repos_dir: Path::new("/backup/octocat/git/repos"),
//!         description_prefix: "GitHub mirror of octocat/",
//!         public_repos: &none,
//!         failed_repos: &none,
//!     },
//! )
//! .await;
//!
//! println!("Pushed {} repos, {} errors", stats.pushed, stats.errored);
//! # Ok(())
//! # }
//! ```

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
#![warn(missing_debug_implementations)]

pub mod client;
pub mod config;
pub mod error;
pub mod gitlab_client;
mod http;
mod push;
pub mod runner;

pub use client::GiteaClient;
pub use error::MirrorError;
pub use gitlab_client::GitLabClient;
pub use runner::{MirrorDestination, MirrorPlan, MirrorStats};
