// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Orderly shutdown on SIGINT / SIGTERM.

use github_backup_core::CancelFlag;

/// Waits for a process shutdown signal and returns the conventional exit code.
///
/// Handles:
/// - `SIGINT` (Ctrl+C) → exit code 130  (128 + 2)
/// - `SIGTERM` (`docker stop`, `systemctl stop`, Kubernetes) → exit code 143  (128 + 15)
///
/// On Windows only `Ctrl+C` is handled (exit code 130); there is no SIGTERM.
pub(crate) async fn wait_for_shutdown_signal() -> u8 {
    // Ctrl+C / SIGINT is cross-platform via tokio.
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
        130u8
    };

    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            code = ctrl_c => code,
            _ = sigterm.recv() => 143u8,
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await
    }
}

/// How long the process may take to wind down after a shutdown signal.
///
/// `docker stop` waits 10 s before SIGKILL; staying under that lets the
/// process leave on its own terms (lock released, askpass scripts removed).
pub(crate) const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(8);

/// Starts an orderly shutdown after SIGINT/SIGTERM.
///
/// Cancelling the engine's [`CancelFlag`] stops running `git` subprocesses (and
/// their transport helpers) and refuses new ones, so the runtime can drop
/// promptly.  A watchdog thread then forces the exit if anything still blocks
/// it: the tokio signal handler swallows further Ctrl+C presses, so without it
/// a stuck task could not be interrupted except with SIGKILL.
pub(crate) fn begin_shutdown(cancel: &CancelFlag, exit_code: u8) {
    cancel.cancel();
    std::thread::spawn(move || {
        std::thread::sleep(SHUTDOWN_GRACE);
        eprintln!("shutdown did not finish within {SHUTDOWN_GRACE:?}; forcing exit");
        std::process::exit(i32::from(exit_code));
    });
}
