// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! S3-compatible storage backend for `github-backup-rust`.
//!
//! Uploads backup artefacts (JSON metadata and optional binary assets) to any
//! S3-compatible object store using a pure-Rust implementation of
//! [AWS Signature Version 4][sigv4].
//!
//! # Supported Services
//!
//! | Service | Notes |
//! |---------|-------|
//! | **AWS S3** | Standard virtual-hosted URLs |
//! | **Backblaze B2** | Set `endpoint` to your B2 S3-compatible URL |
//! | **Cloudflare R2** | Set `endpoint` to `https://<account>.r2.cloudflarestorage.com` |
//! | **MinIO** | Set `endpoint` to your MinIO URL (HTTP or HTTPS) |
//! | **DigitalOcean Spaces** | Set `endpoint` to `https://<region>.digitaloceanspaces.com` |
//! | **Wasabi** | Set `endpoint` to `https://s3.<region>.wasabisys.com` |
//!
//! # Design
//!
//! - **No AWS SDK**: avoids large transitive dependencies.
//! - **No reqwest / OpenSSL**: uses `hyper` + `rustls` from the workspace.
//! - **SigV4 from scratch**: implemented in [`signing`] using `sha2` + `hmac`.
//! - **Incremental**: already-existing objects are skipped via `HeadObject`.
//!
//! # Usage
//!
//! ```no_run
//! use github_backup_s3::{S3Client, config::S3Config, sync::sync_to_s3};
//! use std::path::Path;
//!
//! # async fn example() -> Result<(), github_backup_s3::S3Error> {
//! let config = S3Config {
//!     bucket: "my-github-backups".to_string(),
//!     region: "us-east-1".to_string(),
//!     prefix: "github/".to_string(),
//!     endpoint: None,
//!     access_key_id: std::env::var("AWS_ACCESS_KEY_ID").unwrap(),
//!     secret_access_key: std::env::var("AWS_SECRET_ACCESS_KEY").unwrap(),
//! };
//!
//! let client = S3Client::new(config.clone())?;
//! let stats = sync_to_s3(
//!     &client,
//!     &config,
//!     Path::new("/backup/octocat"),
//!     false,  // skip binary release assets
//!     None,   // no at-rest encryption
//!     false,  // keep stale S3 objects
//! ).await?;
//!
//! println!("Uploaded {} files", stats.uploaded);
//! # Ok(())
//! # }
//! ```
//!
//! [sigv4]: https://docs.aws.amazon.com/general/latest/gr/sigv4_signing.html

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
#![warn(missing_debug_implementations)]

pub mod client;
pub mod config;
pub mod encrypt;
pub mod error;
pub mod signing;
pub mod sync;

pub use client::S3Client;
pub use error::S3Error;
pub use sync::SyncStats;
