// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Orderly shutdown on SIGINT / SIGTERM.

use github_backup_core::CancelFlag;

/// Listens for the process shutdown signals for as long as it lives.
///
/// Create it once, early, and keep it: a signal that arrives while nothing is
/// waiting on [`Shutdown::recv`] is remembered and returned by the next call,
/// so no Ctrl+C is lost between two phases of a run (or between two runs of
/// `--every`).
///
/// - `SIGINT` (Ctrl+C) → exit code 130  (128 + 2)
/// - `SIGTERM` (`docker stop`, `systemctl stop`, Kubernetes) → exit code 143  (128 + 15)
///
/// On Windows only `Ctrl+C` is handled (exit code 130); there is no SIGTERM.
pub(crate) struct Shutdown {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(not(unix))]
    ctrl_c: tokio::signal::windows::CtrlC,
}

impl Shutdown {
    /// Installs the signal handlers.
    ///
    /// # Panics
    ///
    /// If the operating system refuses to install a signal handler, which
    /// does not happen on a normal system.
    pub(crate) fn install() -> Self {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            Self {
                interrupt: signal(SignalKind::interrupt())
                    .expect("failed to install SIGINT handler"),
                terminate: signal(SignalKind::terminate())
                    .expect("failed to install SIGTERM handler"),
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                ctrl_c: tokio::signal::windows::ctrl_c().expect("failed to install Ctrl+C handler"),
            }
        }
    }

    /// Waits for the next shutdown signal and returns the conventional exit code.
    pub(crate) async fn recv(&mut self) -> u8 {
        #[cfg(unix)]
        {
            tokio::select! {
                _ = self.interrupt.recv() => 130,
                _ = self.terminate.recv() => 143,
            }
        }
        #[cfg(not(unix))]
        {
            self.ctrl_c.recv().await;
            130
        }
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
/// it: the signal handlers swallow further Ctrl+C presses, so without it a
/// stuck task could not be interrupted except with SIGKILL.
pub(crate) fn begin_shutdown(cancel: &CancelFlag, exit_code: u8) {
    cancel.cancel();
    std::thread::spawn(move || {
        std::thread::sleep(SHUTDOWN_GRACE);
        eprintln!("shutdown did not finish within {SHUTDOWN_GRACE:?}; forcing exit");
        std::process::exit(i32::from(exit_code));
    });
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A signal raised while nobody is waiting is not lost: the next `recv`
    /// returns it.  (The old per-call `ctrl_c()` listener missed it, so a
    /// Ctrl+C during post-processing went unnoticed.)
    #[tokio::test]
    async fn a_signal_raised_before_recv_is_delivered() {
        let mut shutdown = Shutdown::install();
        let status = std::process::Command::new("kill")
            .args(["-TERM", &std::process::id().to_string()])
            .status()
            .expect("kill");
        assert!(status.success());
        let code = tokio::time::timeout(std::time::Duration::from_secs(5), shutdown.recv())
            .await
            .expect("signal delivered");
        assert_eq!(code, 143);
    }
}
