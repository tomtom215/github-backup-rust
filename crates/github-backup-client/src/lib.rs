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
//! | Authentication | Personal access token (classic & fine-grained); a GitHub App (JWT signed with `aws-lc-rs`, installation tokens renewed five minutes before they expire, single-flight); the OAuth device flow ([`oauth`]) on github.com or GHES |
//! | API version | `Accept: application/vnd.github+json` and `X-GitHub-Api-Version: 2022-11-28` on every request; `Deprecation` / `Sunset` headers are logged once per endpoint |
//! | Pagination | Automatic via `Link` response header (including the `{"total_count", "<list>"}` wrappers of workflows, runs and environments); a next link to another origin or to a page already fetched is an error |
//! | Lossless lists | Lists are decoded element by element into a [`Page`](github_backup_types::Page): the original JSON of every element is kept, and an element that does not fit its model is isolated and logged instead of failing the list |
//! | Redirects | Up to 3 within the origin, with the credential (renamed or transferred repositories); non-GET requests follow only 307/308 |
//! | Rate limiting | A 403/429 with `Retry-After`, `X-RateLimit-Remaining: 0` or a rate-limit message is waited out (at least a minute and doubling when GitHub gives no hint; one wait at most about an hour, all waits of a request about two), pausing every clone of the client, and ends as [`ClientError::RateLimitExceeded`], never as a plain 403 |
//! | Retries | GETs (and GraphQL queries): up to 3 retries on 500/502/503/504, connection failures, timeouts and stalled bodies, with exponential back-off + jitter; 4xx fail fast |
//! | Body cap | 16 MiB cap on API responses ([`ClientError::BodyTooLarge`]) to protect against runaway streams |
//! | GraphQL | [`GitHubClient::graphql`] runs a query with the same handling, including GraphQL's own rate-limit reports |
//! | Release assets | Streamed chunk by chunk to a caller-supplied sink; `Authorization` is never forwarded to another origin and HTTPS→HTTP redirects are refused |
//! | TLS | rustls with platform CA bundle |
//! | Proxy | `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY` / `NO_PROXY` (either case): HTTP `CONNECT` tunnelling for HTTPS, absolute-form for HTTP; [`ProxyClient`] is shared with the webhook notifier and the device flow |
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

mod api_client;
mod app_auth;
mod client;
mod decode;
mod error;
pub mod oauth;
mod pagination;
pub mod proxy;
mod rate_limit;
mod token;

pub use api_client::{AssetSink, BackupClient, BoxFuture};
pub use client::{GitHubClient, Installation};
pub use error::ClientError;
pub use pagination::parse_next_link;
pub use proxy::{ProxyClient, ProxySettings};
pub use rate_limit::RateLimitInfo;
pub use token::{GitToken, TokenSource};

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub use app_auth::test_keys;
