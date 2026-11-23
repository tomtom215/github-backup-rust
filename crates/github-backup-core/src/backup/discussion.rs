// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub Discussions: requested, but not backed up yet.
//!
//! GitHub serves Discussions through its GraphQL API only; the REST API (see
//! GitHub's OpenAPI description, `github/rest-api-description`) has no route
//! for repository discussions, and the backup does not speak GraphQL yet.
//! So `--discussions` backs up nothing and writes nothing.  Rather than let
//! the run look as if it had succeeded at it, the engine calls
//! [`warn_if_requested`] once per run, which says so in one warning.

use tracing::warn;

use github_backup_types::config::BackupOptions;

/// Logs one warning that discussions are not backed up when
/// `opts.discussions` is set; returns whether it did.
///
/// The engine calls this once at the start of each run.
pub fn warn_if_requested(opts: &BackupOptions) -> bool {
    if opts.discussions {
        warn!(
            "--discussions was requested, but GitHub serves Discussions through its \
             GraphQL API only and this version does not back them up: no discussion \
             data is saved"
        );
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_only_when_requested() {
        assert!(!warn_if_requested(&BackupOptions::default()));
        let requested = BackupOptions {
            discussions: true,
            ..Default::default()
        };
        assert!(warn_if_requested(&requested));
    }
}
