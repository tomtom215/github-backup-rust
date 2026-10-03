// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Cooperative cancellation of a backup run.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A shared, one-way "stop now" switch.
///
/// Cloning gives another handle to the **same** switch.  The engine hands one
/// to everything it starts: a running `git` subprocess is killed (with its
/// process group) within one supervision poll interval, new git work is refused
/// with [`CoreError::Interrupted`](crate::CoreError::Interrupted), and the
/// scheduler starts no further repository.
///
/// The switch belongs to one engine, not to the process, so a front end that
/// can run several backups in a row (the TUI) simply creates a fresh engine —
/// and with it a fresh switch — for each run.
#[derive(Debug, Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    /// Creates a switch that has not been triggered.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Triggers the switch.  Idempotent; it cannot be reset.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// `true` once [`cancel`](Self::cancel) has been called on any clone.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
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
}
