// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Per-category backup helpers invoked by the [`crate::engine::BackupEngine`].
//!
//! Each submodule handles one data category (repositories, issues, …) and
//! exposes a single async function that writes artefacts via the
//! [`crate::storage::Storage`] and [`crate::git::GitRunner`] traits.
//!
//! # Failures inside a category
//!
//! A category is one engine *step*; an error it returns fails the step.  A
//! category made of many items (an issue's comments, a release's assets, a
//! package's versions) does not give up at the first item that fails:
//! `Outcome::item_error` records it and the loop carries on, and the engine
//! records every entry of [`Outcome::failed`] as a failure, so the run still
//! ends with exit status 3.  A sub-resource that answers 404 or 410 is gone
//! (deleted since it was listed) and is not a failure.  Fatal errors
//! ([`CoreError::is_fatal`]) are returned at once.

pub mod actions;
pub mod branches;
pub mod collaborators;
pub mod commit_comments;
pub mod deploy_keys;
pub mod discussion;
pub mod environments;
pub mod gist;
pub mod hooks;
pub mod issue;
pub mod labels;
pub(crate) mod merge;
pub mod milestones;
pub mod package;
pub mod project;
pub mod pull_request;
pub mod release;
pub mod repository;
pub mod security_advisories;
pub mod starred_repos;
pub mod topics;
pub mod user_data;
pub mod wiki;

use serde_json::Value;
use tracing::info;

use github_backup_client::ClientError;
use github_backup_types::{config::BackupOptions, Page};

use crate::error::CoreError;

/// What a category accomplished when it carried on past individual items.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Items backed up (what is counted depends on the category: issues,
    /// pull requests, workflows, gists, ...).
    pub count: u64,
    /// Items that could not be backed up completely, as `(what, error)`.
    /// The rest of the category was still processed.
    pub failed: Vec<(String, String)>,
}

impl Outcome {
    /// Records that `what` could not be backed up.
    pub(crate) fn fail(&mut self, what: impl Into<String>, message: impl Into<String>) {
        self.failed.push((what.into(), message.into()));
    }

    /// Deals with the error of one item: a fatal error is returned (the run
    /// stops); 404 or 410 means the item is gone and is only logged; anything
    /// else is recorded with [`fail`](Self::fail) and the caller carries on.
    ///
    /// # Errors
    ///
    /// Returns `error` if it is fatal.
    pub(crate) fn item_error(
        &mut self,
        what: impl Into<String>,
        error: impl Into<CoreError>,
    ) -> Result<(), CoreError> {
        let error = error.into();
        let what = what.into();
        if error.is_fatal() {
            return Err(error);
        }
        if is_gone(&error) {
            info!(item = %what, "no longer exists on GitHub; skipping it");
            return Ok(());
        }
        self.fail(what, error.to_string());
        Ok(())
    }
}

/// A category without items to count or to carry on past.
impl From<()> for Outcome {
    fn from((): ()) -> Self {
        Self::default()
    }
}

/// `true` for an API answer that says the resource does not exist (404) or
/// no longer exists (410): a category switched off, an item deleted since
/// it was listed.
#[must_use]
pub(crate) fn is_gone(error: &CoreError) -> bool {
    matches!(
        error,
        CoreError::Client(ClientError::ApiError {
            status: 404 | 410,
            ..
        })
    )
}

/// `true` for an API answer that refuses access (403).  Rate limits are not
/// among them: the client waits those out and reports an exhausted budget as
/// [`ClientError::RateLimitExceeded`].
#[must_use]
pub(crate) fn is_forbidden(error: &CoreError) -> bool {
    matches!(
        error,
        CoreError::Client(ClientError::ApiError { status: 403, .. })
    )
}

/// What a refusal (HTTP 403) of `category` means for the run; see
/// [`BackupOptions::tolerate_forbidden`].
///
/// With `tolerate_forbidden` (`--all`) the category is skipped: `Ok(())`, and
/// when `warn` is true one warning naming what the token lacks (callers pass
/// `false` when that warning was already given in this run).  Otherwise the
/// refusal is returned as an error that names what the token lacks, which
/// the engine records as a failure.
///
/// # Errors
///
/// The rewritten refusal, unless refusals are tolerated.
pub(crate) fn refused(
    opts: &BackupOptions,
    category: &str,
    needs: &str,
    error: CoreError,
    warn: bool,
) -> Result<(), CoreError> {
    let said = match &error {
        CoreError::Client(ClientError::ApiError { body, .. }) => body.clone(),
        other => other.to_string(),
    };
    if opts.tolerate_forbidden {
        if warn {
            tracing::warn!(
                category,
                needs,
                "the token is not permitted to read {category} (it needs {needs}); \
                 skipping {category} wherever GitHub refuses it in this run"
            );
        }
        return Ok(());
    }
    Err(CoreError::Client(ClientError::ApiError {
        status: 403,
        body: format!("the token is not permitted to read {category} (it needs {needs}): {said}"),
    }))
}

/// The original JSON of every element of `page`: the typed ones, then the
/// ones the typed model could not read.  Loops that fetch per-item data walk
/// this, so an element the model cannot read still gets its sub-resources
/// whenever it has the property they are addressed by.
pub(crate) fn all_json<T>(page: &Page<T>) -> impl Iterator<Item = &Value> {
    page.iter().map(|item| item.json()).chain(page.unparsed())
}

/// The number at the end of an API URL such as
/// `https://api.github.com/repos/o/r/issues/12` (`segment` = `"issues"`).
pub(crate) fn number_after(url: &str, segment: &str) -> Option<u64> {
    let (_, tail) = url.rsplit_once(&format!("/{segment}/"))?;
    tail.parse().ok()
}

/// The value of `key` in an API object as a non-negative integer.
pub(crate) fn json_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

/// The value of `key` in an API object as a string.
pub(crate) fn json_str<'v>(value: &'v Value, key: &str) -> Option<&'v str> {
    value.get(key).and_then(Value::as_str)
}

/// Every element of a list the typed model could not read, described for a
/// failure message: its position among the unparsed ones and its `id` /
/// `number` when it has them.
pub(crate) fn describe_unparsed(value: &Value) -> String {
    match (value.get("number"), value.get("id")) {
        (Some(n), _) => format!("number {n}"),
        (None, Some(id)) => format!("id {id}"),
        (None, None) => "without id or number".to_string(),
    }
}

#[cfg(test)]
pub(crate) mod mock_client;

#[cfg(test)]
mod integration_tests;

#[cfg(test)]
mod lossless_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn api(status: u16) -> CoreError {
        CoreError::Client(ClientError::ApiError {
            status,
            body: "x".into(),
        })
    }

    #[test]
    fn item_errors_are_recorded_gone_items_skipped_fatal_ones_returned() {
        let mut outcome = Outcome::default();
        outcome.item_error("a", api(500)).expect("recorded");
        outcome.item_error("b", api(404)).expect("gone");
        outcome.item_error("c", api(410)).expect("gone");
        outcome.item_error("d", api(403)).expect("recorded");
        assert!(outcome.item_error("e", api(401)).is_err(), "fatal");
        let whats: Vec<&str> = outcome.failed.iter().map(|(w, _)| w.as_str()).collect();
        assert_eq!(whats, ["a", "d"]);
    }

    #[test]
    fn a_refusal_is_a_named_failure_unless_tolerated() {
        let strict = BackupOptions::default();
        let err = refused(&strict, "webhooks", "admin access", api(403), true)
            .expect_err("explicitly requested: a failure");
        let text = err.to_string();
        assert!(
            text.contains("webhooks") && text.contains("admin access"),
            "{text}"
        );
        assert!(is_forbidden(&err));

        let all = BackupOptions {
            tolerate_forbidden: true,
            ..Default::default()
        };
        refused(&all, "webhooks", "admin access", api(403), true).expect("--all: skipped");
    }

    #[test]
    fn numbers_are_read_from_the_end_of_api_urls() {
        let url = "https://api.github.com/repos/o/r/issues/12";
        assert_eq!(number_after(url, "issues"), Some(12));
        assert_eq!(
            number_after("https://x/repos/o/r/pulls/7", "pulls"),
            Some(7)
        );
        assert_eq!(number_after(url, "pulls"), None);
        assert_eq!(number_after("https://x/issues/12/comments", "issues"), None);
    }

    #[test]
    fn unparsed_elements_are_described_by_number_or_id() {
        assert_eq!(
            describe_unparsed(&serde_json::json!({"number": 7})),
            "number 7"
        );
        assert_eq!(describe_unparsed(&serde_json::json!({"id": 9})), "id 9");
        assert_eq!(describe_unparsed(&Value::Null), "without id or number");
    }
}
