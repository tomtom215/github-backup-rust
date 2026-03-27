// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Async GitHub REST API v3 client.
//!
//! Built on [hyper] and [rustls] — no OpenSSL, no reqwest.
//!
//! # Architecture
//!
//! ```text
//! You → GitHubClient → hyper (HTTP/1.1) → rustls (TLS) → api.github.com
//! ```
//!
//! ## Features
//!
//! | Capability | Details |
//! |-----------|---------|
//! | Authentication | Personal access token (classic & fine-grained) |
//! | Pagination | Automatic via `Link` response header |
//! | Rate limiting | Automatic back-off when `X-RateLimit-Remaining == 0` |
//! | Retries | Configurable retry on transient 5xx responses |
//! | TLS | rustls with platform CA bundle |
//!
//! # Example
//!
//! ```no_run
//! use github_backup_client::GitHubClient;
//! use github_backup_types::config::Credential;
//!
//! # async fn example() -> Result<(), github_backup_client::ClientError> {
//! let cred = Credential::Token("ghp_xxxx".to_string());
//! let client = GitHubClient::new(cred)?;
//! let repos = client.list_user_repos("octocat").await?;
//! println!("Found {} repos", repos.len());
//! # Ok(())
//! # }
//! ```

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
#![warn(missing_debug_implementations)]

mod client;
mod error;
mod pagination;
mod rate_limit;

pub use client::GitHubClient;
pub use error::ClientError;
pub use pagination::parse_next_link;
pub use rate_limit::RateLimitInfo;
