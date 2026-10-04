// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Cooperative cancellation of a backup run.

use std::sync::Arc;

use tokio::sync::watch;

/// A shared, one-way "stop now" switch.
///
/// Cloning gives another handle to the **same** switch.  The engine hands one
/// to everything it starts: a running `git` subprocess is killed (with its
/// process group) within one supervision poll interval, an in-flight step is
/// abandoned at its next `await`, new git work is refused with
/// [`CoreError::Interrupted`](crate::CoreError::Interrupted), and the scheduler
/// starts no further repository.
///
/// The switch belongs to one engine, not to the process, so a front end that
/// can run several backups in a row (the TUI) simply creates a fresh engine —
/// and with it a fresh switch — for each run.
#[derive(Debug, Clone)]
pub struct CancelFlag(Arc<watch::Sender<bool>>);

impl Default for CancelFlag {
    fn default() -> Self {
        Self(Arc::new(watch::channel(false).0))
    }
}

impl CancelFlag {
    /// Creates a switch that has not been triggered.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Triggers the switch.  Idempotent; it cannot be reset.
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }

    /// `true` once [`cancel`](Self::cancel) has been called on any clone.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }

    /// Completes when the switch is triggered (immediately if it already was).
    pub async fn cancelled(&self) {
        let mut rx = self.0.subscribe();
        // `wait_for` only errors when the sender is dropped, which cannot
        // happen while `self` holds it.
        let _ = rx.wait_for(|cancelled| *cancelled).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_untriggered_and_stays_triggered() {
        let flag = CancelFlag::new();
        assert!(!flag.is_cancelled());
        flag.cancel();
        assert!(flag.is_cancelled());
        flag.cancel();
        assert!(flag.is_cancelled(), "cancelling twice is harmless");
    }

    #[test]
    fn clones_share_one_switch_and_separate_flags_do_not() {
        let a = CancelFlag::new();
        let b = a.clone();
        let other = CancelFlag::new();
        b.cancel();
        assert!(a.is_cancelled(), "a clone must trigger the original");
        assert!(
            !other.is_cancelled(),
            "an unrelated flag must be unaffected"
        );
    }

    #[tokio::test]
    async fn cancelled_completes_when_triggered_and_immediately_if_already() {
        let flag = CancelFlag::new();
        let waiter = {
            let flag = flag.clone();
            tokio::spawn(async move { flag.cancelled().await })
        };
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished(), "must wait until cancelled");
        flag.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
            .await
            .expect("wakes promptly")
            .expect("no panic");
        // Already cancelled: returns without waiting.
        tokio::time::timeout(std::time::Duration::from_millis(200), flag.cancelled())
            .await
            .expect("immediate");
    }
}
