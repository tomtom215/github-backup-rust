// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Push-mirror backup to Gitea, Codeberg, Forgejo, and other self-hosted Git
//! services.
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
//! # Supported Destinations
//!
//! Any host that implements the [Gitea REST API v1] is supported, including:
//!
//! - [Codeberg] — free, ad-free, non-profit hosting powered by Forgejo.
//! - [Gitea] — self-hostable, lightweight Git service.
//! - [Forgejo] — community-driven fork of Gitea.
//! - Self-hosted instances of any of the above.
//!
//! [Gitea REST API v1]: https://gitea.io/api/swagger
//! [Codeberg]: https://codeberg.org
//! [Gitea]: https://gitea.io
//! [Forgejo]: https://forgejo.org
//!
//! # Usage
//!
//! ```no_run
//! use github_backup_mirror::{GiteaClient, config::GiteaConfig, runner::push_mirrors};
//! use std::path::Path;
//!
//! # async fn example() -> Result<(), github_backup_mirror::MirrorError> {
//! let config = GiteaConfig {
//!     base_url: "https://codeberg.org".to_string(),
//!     token: std::env::var("CODEBERG_TOKEN").unwrap_or_default(),
//!     owner: "alice".to_string(),
//!     private: true,
//! };
//!
//! let client = GiteaClient::new(config.clone())?;
//! let stats = push_mirrors(
//!     &client,
//!     &config,
//!     Path::new("/backup/octocat/git/repos"),
//!     "Mirror of ",
//! ).await?;
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
pub mod gitlab_runner;
pub mod runner;

pub use client::GiteaClient;
pub use error::MirrorError;
pub use gitlab_client::GitLabClient;
pub use runner::MirrorStats;
