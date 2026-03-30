// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Real-time progress events emitted by the [`BackupEngine`].
//!
//! Callers that need live progress (e.g. the TUI) construct an unbounded
//! channel with [`tokio::sync::mpsc::unbounded_channel`], pass the sender to
//! [`BackupEngine::with_event_channel`], and poll the receiver from their own
//! task.
//!
//! The CLI does not use this channel; the events are optional and the engine
//! operates correctly if no channel is attached.
//!
//! [`BackupEngine`]: crate::engine::BackupEngine

use tokio::sync::mpsc::UnboundedSender;

/// An event emitted by the backup engine during a run.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// The total number of repositories discovered (after listing from the API).
    ///
    /// Emitted once, before any `RepoStarted` events.
    ReposDiscovered {
        /// Total number of repositories that will be processed.
        total: u64,
    },

    /// A repository worker task has started.
    RepoStarted {
        /// Full repository name (`owner/repo`).
        name: String,
    },

    /// A repository worker task has finished.
    RepoCompleted {
        /// Full repository name (`owner/repo`).
        name: String,
        /// `true` if the repository was successfully backed up.
        success: bool,
        /// Error description, present only when `success` is `false`.
        error: Option<String>,
    },
}

/// Sender half of the engine event channel.
///
/// Clone-able; each clone sends to the same channel.  Use
/// [`tokio::sync::mpsc::unbounded_channel`] to create a matched pair.
pub type EngineEventTx = UnboundedSender<EngineEvent>;
