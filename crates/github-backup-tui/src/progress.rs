// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Progress-reporting channel used to bridge the backup engine to the TUI.
//!
//! The backup engine runs on a background tokio task.  It emits [`BackupEvent`]
//! messages through an unbounded channel that the TUI event loop drains each
//! render tick.

use tokio::sync::mpsc;

use crate::event::BackupEvent;

/// Sender side — kept by the backup task / tracing layer.
pub type ProgressTx = mpsc::UnboundedSender<BackupEvent>;

/// Receiver side — kept by the TUI event loop.
pub type ProgressRx = mpsc::UnboundedReceiver<BackupEvent>;

/// Creates a new (sender, receiver) pair for backup progress events.
pub fn channel() -> (ProgressTx, ProgressRx) {
    mpsc::unbounded_channel()
}
