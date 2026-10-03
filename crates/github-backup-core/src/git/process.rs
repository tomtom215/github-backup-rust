// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Spawning and supervising a single `git` subprocess.
//!
//! [`run_git`] is the only place in the engine that executes git.  It is
//! deliberately free of any async code: [`ProcessGitRunner`] calls it from a
//! blocking task so a long clone never occupies an async worker thread.
//!
//! # Supervision rules
//!
//! * **Output is drained continuously.**  stderr is read by a helper thread
//!   that keeps only the last [`STDERR_TAIL_BYTES`]; a child that writes more
//!   than the OS pipe buffer (64 KiB on Linux) can therefore never block.
//! * **The limit is a *stall* limit, not a wall-clock limit.**  A child is
//!   stopped only when it has produced no output for
//!   [`CloneOptions::stall_timeout_secs`].  git is run with `--progress`, so a
//!   healthy multi-hour clone of a very large repository keeps emitting
//!   progress and is never interrupted, while a hung connection is.
//! * **A shutdown request stops the child promptly.**  See
//!   [`request_shutdown`].
//! * **The child leads its own process group** (Unix), so killing it also
//!   reaches git's transport helpers (`git-remote-https`, `ssh`, …) instead of
//!   leaving them blocked on a dead socket.
//! * **git can never prompt.**  stdin is closed and `GIT_TERMINAL_PROMPT=0`,
//!   so a missing or rejected credential fails fast instead of waiting on a
//!   terminal that does not exist (cron, Docker, systemd).
//!
//! [`ProcessGitRunner`]: super::ProcessGitRunner
//! [`CloneOptions::stall_timeout_secs`]: super::CloneOptions::stall_timeout_secs

use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tracing::debug;

use super::askpass::AskpassScript;
use super::CloneOptions;
use crate::error::CoreError;

/// How much of a child's stderr is retained for error messages.
const STDERR_TAIL_BYTES: usize = 16 * 1024;

/// How long to wait for the stderr reader to reach end-of-file after the child
/// has exited.  It can only be late if a grandchild inherited the pipe.
const READER_GRACE: Duration = Duration::from_secs(2);

/// How often the supervisor checks the child, the stall timer and the
/// shutdown flag.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How many lines of the stderr tail are kept in an error message.
const ERROR_LINES: usize = 12;

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Asks every running and future git subprocess to stop.
///
/// Called by the binary when it receives SIGINT/SIGTERM.  Running children are
/// killed within one poll interval and new ones are refused with
/// [`CoreError::Interrupted`], so the process can exit promptly (for example
/// inside Docker's 10-second `docker stop` grace period) without leaving git
/// processes behind.  The flag is process-wide and one-way.
pub fn request_shutdown() {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

/// Returns `true` once [`request_shutdown`] has been called.
#[must_use]
pub fn shutdown_requested() -> bool {
    SHUTDOWN.load(Ordering::SeqCst)
}

/// Retains the tail of a child's stderr and the time of its last output.
struct StderrCapture {
    tail: Arc<Mutex<Vec<u8>>>,
    /// Milliseconds since `epoch` at which output was last seen.
    last_output_ms: Arc<AtomicU64>,
    epoch: Instant,
    finished: mpsc::Receiver<()>,
}

fn elapsed_ms(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

impl StderrCapture {
    fn start(stream: ChildStderr) -> Self {
        let tail = Arc::new(Mutex::new(Vec::new()));
        let last_output_ms = Arc::new(AtomicU64::new(0));
        let epoch = Instant::now();
        let (done_tx, finished) = mpsc::channel();

        let reader_tail = Arc::clone(&tail);
        let reader_activity = Arc::clone(&last_output_ms);
        thread::spawn(move || {
            let mut stream = stream;
            let mut chunk = [0u8; 8192];
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        reader_activity.store(elapsed_ms(epoch), Ordering::Relaxed);
                        let mut buf = reader_tail.lock().unwrap_or_else(|p| p.into_inner());
                        buf.extend_from_slice(&chunk[..n]);
                        if buf.len() > 2 * STDERR_TAIL_BYTES {
                            let excess = buf.len() - STDERR_TAIL_BYTES;
                            buf.drain(..excess);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            let _ = done_tx.send(());
        });

        Self {
            tail,
            last_output_ms,
            epoch,
            finished,
        }
    }

    /// Time since the child last wrote anything (or since it was spawned).
    fn idle_for(&self) -> Duration {
        let now = elapsed_ms(self.epoch);
        Duration::from_millis(now.saturating_sub(self.last_output_ms.load(Ordering::Relaxed)))
    }

    /// Waits briefly for end-of-file and returns the readable tail.
    fn finish(self) -> String {
        let _ = self.finished.recv_timeout(READER_GRACE);
        let bytes = self.tail.lock().unwrap_or_else(|p| p.into_inner()).clone();
        summarise_stderr(&bytes)
    }
}

/// Reduces raw stderr to the few lines worth showing in an error message.
///
/// git redraws progress with carriage returns (`Receiving objects:  45%
/// (45/100)`), which would otherwise bury the one `fatal:` line that matters.
fn summarise_stderr(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text
        .split(['\n', '\r'])
        .map(str::trim)
        .filter(|line| !line.is_empty() && !is_progress_line(line))
        .collect();
    let start = lines.len().saturating_sub(ERROR_LINES);
    lines[start..].join("\n")
}

/// Matches git's percentage progress lines, e.g. `Counting objects:  20% (1/5)`.
fn is_progress_line(line: &str) -> bool {
    line.contains("% (") && line.ends_with([')', ',']) || line.ends_with("), done.")
}

/// Kills `child` and, on Unix, its whole process group, then reaps it.
fn kill_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        // The child was started as the leader of a new process group (see
        // `run_git`), so its pid is the group id.
        if let Ok(pid) = i32::try_from(child.id()) {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Runs `program` with `args` in `cwd` and waits for it, applying the
/// supervision rules described in the [module documentation](self).
///
/// If `token` is `Some`, it is supplied through a temporary `GIT_ASKPASS`
/// script (removed when this function returns) so the credential never appears
/// on the command line or in the process list.
pub(super) fn run_git(
    program: &Path,
    args: &[&str],
    cwd: &Path,
    token: Option<&str>,
    opts: &CloneOptions,
) -> Result<(), CoreError> {
    if shutdown_requested() {
        return Err(CoreError::Interrupted);
    }
    debug!(args = ?args, cwd = %cwd.display(), "running git");

    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        // Never prompt: a missing/rejected credential must fail, not hang.
        .env("GIT_TERMINAL_PROMPT", "0")
        // Stable, English messages: error classification matches on them.
        .env("LC_ALL", "C")
        .env("LANGUAGE", "C");

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // New process group so a stall/shutdown kill reaches git's helpers.
        cmd.process_group(0);
    }

    // Keep the guard alive until the process has exited.
    let _askpass_guard;
    if let Some(tok) = token {
        _askpass_guard = AskpassScript::create(tok);
        if let Some(ref script) = _askpass_guard {
            cmd.env("GIT_ASKPASS", script.path());
            cmd.env("GIT_USERNAME", "x-access-token");
        }
    } else {
        _askpass_guard = None;
    }

    let mut child = cmd.spawn().map_err(CoreError::GitSpawn)?;
    let stderr = StderrCapture::start(child.stderr.take().expect("stderr was configured as piped"));
    let stall_limit = Duration::from_secs(opts.stall_timeout_secs);

    loop {
        match child.try_wait().map_err(CoreError::GitSpawn)? {
            Some(status) => {
                let tail = stderr.finish();
                if status.success() {
                    return Ok(());
                }
                return Err(CoreError::GitFailed {
                    args: args.join(" "),
                    code: status.code().unwrap_or(-1),
                    stderr: tail,
                });
            }
            None => {
                if shutdown_requested() {
                    kill_tree(&mut child);
                    return Err(CoreError::Interrupted);
                }
                if stderr.idle_for() >= stall_limit {
                    kill_tree(&mut child);
                    return Err(CoreError::GitTimeout {
                        args: args.join(" "),
                        timeout_secs: opts.stall_timeout_secs,
                    });
                }
                thread::sleep(POLL_INTERVAL);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarise_keeps_the_fatal_line_and_drops_progress() {
        let raw = b"Cloning into bare repository 'x.git'...\r\
            remote: Counting objects:  20% (1/5)\rremote: Counting objects: 100% (5/5), done.\n\
            Receiving objects:  45% (45/100)\rReceiving objects: 100% (100/100), 1.2 MiB | 3 MiB/s, done.\n\
            fatal: unable to access 'https://example.invalid/r.git/': Could not resolve host\n";
        let out = summarise_stderr(raw);
        assert!(out.contains("fatal: unable to access"), "{out}");
        assert!(out.contains("Cloning into bare repository"), "{out}");
        assert!(!out.contains("45%"), "progress must be dropped: {out}");
    }

    #[test]
    fn summarise_is_bounded_to_the_last_lines() {
        let raw: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let out = summarise_stderr(raw.as_bytes());
        assert_eq!(out.lines().count(), ERROR_LINES);
        assert!(out.ends_with("line 99"));
        assert!(!out.contains("line 0\n"));
    }

    #[test]
    fn summarise_tolerates_invalid_utf8() {
        let out = summarise_stderr(b"fatal: bad \xff\xfe bytes\n");
        assert!(out.starts_with("fatal: bad"));
    }

    #[test]
    fn progress_lines_are_recognised() {
        assert!(is_progress_line("Receiving objects:  45% (45/100)"));
        assert!(is_progress_line("Resolving deltas: 100% (3/3), done."));
        assert!(is_progress_line(
            "remote: Counting objects: 100% (5/5), done."
        ));
        assert!(!is_progress_line("fatal: repository not found"));
        assert!(!is_progress_line("From https://github.com/o/r"));
    }
}
