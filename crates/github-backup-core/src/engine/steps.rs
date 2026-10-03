// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Step isolation: one failing category must not cost the backup of the rest.
//!
//! Every unit of work — cloning a repository, backing up its issues, fetching
//! the owner's gists — is a *step*.  [`Steps::run`] executes a step and turns
//! its error into a recorded [`Failure`](crate::stats::Failure) instead of
//! propagating it, so the remaining steps and the remaining repositories still
//! run.  Only an error for which continuing is pointless
//! ([`CoreError::is_fatal`]) stops the run, through [`RunControl`].

use std::future::Future;
use std::sync::{Arc, Mutex};

use tracing::{error, warn};

use crate::{cancel::CancelFlag, error::CoreError, redact, stats::BackupStats};

/// Shared by every task of one run: the cancellation switch and the first
/// fatal error.
///
/// Once either is set the scheduler starts no new repository, running tasks
/// skip their remaining steps, and the engine reports the error after the tasks
/// have wound down.
#[derive(Debug, Clone, Default)]
pub(crate) struct RunControl {
    fatal: Arc<Mutex<Option<CoreError>>>,
    cancel: CancelFlag,
}

impl RunControl {
    pub(crate) fn new(cancel: CancelFlag) -> Self {
        Self {
            fatal: Arc::default(),
            cancel,
        }
    }

    /// Records `error` as the reason the run stops.  The first one wins.
    pub(crate) fn set_fatal(&self, error: CoreError) {
        let mut slot = self.fatal.lock().unwrap_or_else(|p| p.into_inner());
        if slot.is_none() {
            *slot = Some(error);
        }
    }

    /// `true` once nothing further should be started.
    pub(crate) fn should_stop(&self) -> bool {
        self.cancel.is_cancelled()
            || self
                .fatal
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_some()
    }

    /// Takes the error the run stops with, if it must stop: the first fatal
    /// error, or [`CoreError::Interrupted`] if it was cancelled.
    pub(crate) fn take_stop_reason(&self) -> Option<CoreError> {
        let fatal = self.fatal.lock().unwrap_or_else(|p| p.into_inner()).take();
        fatal.or_else(|| self.cancel.is_cancelled().then_some(CoreError::Interrupted))
    }
}

/// Runs the steps of one scope — a repository, or an owner-level category —
/// recording failures instead of propagating them.
#[derive(Debug)]
pub(crate) struct Steps<'a> {
    scope: String,
    stats: &'a BackupStats,
    control: &'a RunControl,
    secrets: &'a [String],
    clean: bool,
    first_failure: Option<String>,
}

impl<'a> Steps<'a> {
    /// `scope` names what is being backed up (`owner/repo`, or an owner-level
    /// label); `secrets` are exact values to scrub from recorded messages.
    pub(crate) fn new(
        scope: impl Into<String>,
        stats: &'a BackupStats,
        control: &'a RunControl,
        secrets: &'a [String],
    ) -> Self {
        Self {
            scope: scope.into(),
            stats,
            control,
            secrets,
            clean: true,
            first_failure: None,
        }
    }

    /// Runs one step.
    ///
    /// Returns its value on success.  On failure the error is recorded (or, if
    /// fatal, handed to the run control) and `None` is returned.  Nothing runs
    /// once the run has been told to stop.
    pub(crate) async fn run<T, F>(&mut self, step: &str, work: F) -> Option<T>
    where
        F: Future<Output = Result<T, CoreError>>,
    {
        if self.control.should_stop() {
            self.clean = false;
            return None;
        }
        match work.await {
            Ok(value) => Some(value),
            Err(e) => {
                self.fail(step, e);
                None
            }
        }
    }

    /// Records a failure of `step` that the caller detected itself (for
    /// example one gist out of many), without stopping anything.
    pub(crate) fn record(&mut self, step: &str, message: &str) {
        self.clean = false;
        let message = self.scrub(message);
        warn!(scope = %self.scope, step, error = %message, "step failed, continuing with the rest");
        self.stats
            .record_failure(self.scope.clone(), step, message.clone());
        self.note_first(step, &message);
    }

    fn fail(&mut self, step: &str, e: CoreError) {
        self.clean = false;
        let message = self.scrub(&e.to_string());
        if e.is_fatal() {
            error!(scope = %self.scope, step, error = %message, "fatal error: stopping the run");
            self.control.set_fatal(e);
        } else {
            warn!(scope = %self.scope, step, error = %message, "step failed, continuing with the rest");
            self.stats
                .record_failure(self.scope.clone(), step, message.clone());
        }
        self.note_first(step, &message);
    }

    fn scrub(&self, text: &str) -> String {
        let known: Vec<&str> = self.secrets.iter().map(String::as_str).collect();
        redact::secrets(text, &known)
    }

    fn note_first(&mut self, step: &str, message: &str) {
        if self.first_failure.is_none() {
            self.first_failure = Some(format!("{step}: {message}"));
        }
    }

    /// `true` if every step so far succeeded and none was skipped.
    pub(crate) fn is_clean(&self) -> bool {
        self.clean
    }

    /// `step: message` of the first failure, for a one-line event or log.
    pub(crate) fn first_failure(&self) -> Option<&str> {
        self.first_failure.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use github_backup_client::ClientError;

    fn api(status: u16) -> CoreError {
        CoreError::Client(ClientError::ApiError {
            status,
            body: "nope".into(),
        })
    }

    #[tokio::test]
    async fn a_failing_step_is_recorded_and_the_next_step_still_runs() {
        let stats = BackupStats::new();
        let control = RunControl::default();
        let mut steps = Steps::new("o/r", &stats, &control, &[]);

        let first: Option<u32> = steps.run("issues", async { Err(api(500)) }).await;
        let second = steps.run("labels", async { Ok(7u32) }).await;

        assert_eq!(first, None);
        assert_eq!(second, Some(7), "isolation: the next step must run");
        assert!(!steps.is_clean());
        assert_eq!(stats.failure_count(), 1);
        let failure = &stats.failures()[0];
        assert_eq!(
            (failure.scope.as_str(), failure.step.as_str()),
            ("o/r", "issues")
        );
        assert!(failure.message.contains("500"), "{}", failure.message);
        assert!(steps
            .first_failure()
            .expect("first")
            .starts_with("issues: "));
        assert!(
            !control.should_stop(),
            "a plain failure must not stop the run"
        );
    }

    #[tokio::test]
    async fn a_fatal_error_stops_everything_and_is_not_listed_as_a_failure() {
        let stats = BackupStats::new();
        let control = RunControl::default();
        let mut steps = Steps::new("o/r", &stats, &control, &[]);

        steps.run("issues", async { Err::<(), _>(api(401)) }).await;
        assert!(control.should_stop());

        let mut ran = false;
        steps
            .run("labels", async {
                ran = true;
                Ok(())
            })
            .await;
        assert!(!ran, "no step may start once the run must stop");
        assert!(!steps.is_clean());
        assert_eq!(
            stats.failure_count(),
            0,
            "the fatal error is the run's result"
        );
        assert!(matches!(
            control.take_stop_reason(),
            Some(CoreError::Client(ClientError::ApiError { status: 401, .. }))
        ));
    }

    #[tokio::test]
    async fn the_first_fatal_error_wins() {
        let control = RunControl::default();
        control.set_fatal(api(401));
        control.set_fatal(CoreError::Interrupted);
        assert!(matches!(
            control.take_stop_reason(),
            Some(CoreError::Client(ClientError::ApiError { status: 401, .. }))
        ));
    }

    #[tokio::test]
    async fn cancelling_stops_new_steps_and_reports_interrupted() {
        let cancel = CancelFlag::new();
        let control = RunControl::new(cancel.clone());
        let stats = BackupStats::new();
        let mut steps = Steps::new("o/r", &stats, &control, &[]);

        assert_eq!(steps.run("a", async { Ok(1) }).await, Some(1));
        cancel.cancel();
        assert_eq!(steps.run("b", async { Ok(2) }).await, None);
        assert!(matches!(
            control.take_stop_reason(),
            Some(CoreError::Interrupted)
        ));
    }

    #[tokio::test]
    async fn recorded_messages_never_contain_the_token() {
        let stats = BackupStats::new();
        let control = RunControl::default();
        let secrets = vec!["tok-0123456789".to_string()];
        let mut steps = Steps::new("o/r", &stats, &control, &secrets);

        steps
            .run("clone", async {
                Err::<(), _>(CoreError::GitFailed {
                    args: "clone".into(),
                    code: 128,
                    stderr: "fatal: could not use tok-0123456789 or ghp_AbCdEf123".into(),
                })
            })
            .await;
        steps.record("gist", "also leaked tok-0123456789 here");

        for f in stats.failures() {
            assert!(!f.message.contains("tok-0123456789"), "{}", f.message);
            assert!(!f.message.contains("AbCdEf123"), "{}", f.message);
        }
        assert!(!steps
            .first_failure()
            .expect("first")
            .contains("tok-0123456789"));
    }

    #[tokio::test]
    async fn a_scope_with_no_failures_is_clean() {
        let stats = BackupStats::new();
        let control = RunControl::default();
        let mut steps = Steps::new("o/r", &stats, &control, &[]);
        steps.run("a", async { Ok(()) }).await;
        assert!(steps.is_clean());
        assert!(steps.first_failure().is_none());
        assert_eq!(stats.failure_count(), 0);
    }
}
