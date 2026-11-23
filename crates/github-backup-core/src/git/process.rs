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
//! * **Cancellation stops the child promptly.**  When
//!   [`CloneOptions::cancel`] is triggered the child is stopped within one poll
//!   interval plus a short grace period.
//! * **The child leads its own process group** (Unix), so stopping it also
//!   reaches git's transport helpers (`git-remote-https`, `ssh`, …) instead of
//!   leaving them blocked on a dead socket.  The group first gets `SIGTERM`,
//!   on which git removes the `.lock` files it holds, and `SIGKILL` only after
//!   [`TERM_GRACE`].
//! * **git does not prompt.**  stdin is closed and `GIT_TERMINAL_PROMPT=0`, so
//!   a missing or rejected HTTPS credential fails fast instead of waiting on a
//!   terminal.  `ssh` reads its questions (an unknown host key, a key
//!   passphrase) from the controlling terminal, not from stdin; for SSH URLs
//!   `GIT_SSH_COMMAND` is therefore set to `ssh -o BatchMode=yes` unless the
//!   user chose an SSH command (`GIT_SSH_COMMAND`, `GIT_SSH` or
//!   `core.sshCommand`), in which case answering without a prompt is up to
//!   that command.
//! * **git-lfs reports progress** (`GIT_LFS_FORCE_PROGRESS=1`) although its
//!   output is not a terminal, so a long LFS transfer is not mistaken for a
//!   stall.
//!
//! [`ProcessGitRunner`]: super::ProcessGitRunner
//! [`CloneOptions::stall_timeout_secs`]: super::CloneOptions::stall_timeout_secs

use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStderr, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tracing::debug;

use super::credential;
use super::{CloneOptions, GitCommand};
use crate::error::CoreError;

/// How much of a child's stderr is retained for error messages.
const STDERR_TAIL_BYTES: usize = 16 * 1024;

/// How long to wait for the stderr reader to reach end-of-file after the child
/// has exited.  It can only be late if a grandchild inherited the pipe.
const READER_GRACE: Duration = Duration::from_secs(2);

/// How often the supervisor checks the child, the stall timer and the
/// cancel flag.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How many lines of the stderr tail are kept in an error message.
const ERROR_LINES: usize = 12;

/// How long a stopped child's process group has between `SIGTERM` and
/// `SIGKILL` to clean up (git removes its `.lock` files on `SIGTERM`).
#[cfg(unix)]
const TERM_GRACE: Duration = Duration::from_secs(2);

/// The SSH command used when the user configured none: it fails instead of
/// asking a question nobody can answer.
const BATCH_SSH_COMMAND: &str = "ssh -o BatchMode=yes";

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
        .filter(|line| {
            !line.is_empty() && !is_progress_line(line) && !line.starts_with("Cloning into")
        })
        .collect();
    let start = lines.len().saturating_sub(ERROR_LINES);
    lines[start..].join("\n")
}

/// Matches git's percentage progress lines, e.g. `Counting objects:  20% (1/5)`.
fn is_progress_line(line: &str) -> bool {
    line.contains("% (") && line.ends_with([')', ',']) || line.ends_with("), done.")
}

/// Appends an actionable hint to git errors whose cause is not obvious.
fn with_hints(mut stderr: String) -> String {
    if stderr.contains("dubious ownership") {
        stderr.push_str(
            "\nhint: the repository is owned by a different user than the one running \
             github-backup. Run it as the owning user (for example docker run --user), or \
             fix ownership (chown -R) of the backup directory.",
        );
    }
    if stderr.contains(".lock': File exists") {
        stderr.push_str(
            "\nhint: a git process holds (or a killed one left) this lock file. \
             github-backup removes lock files older than the stall limit before it \
             updates a repository, so the next run clears a stale one; delete it by \
             hand only if no git process is running in that repository.",
        );
    }
    stderr
}

/// Stops `child` and, on Unix, its whole process group, then reaps it.
///
/// The group gets `SIGTERM` first so git can remove its `.lock` files (a
/// `SIGKILL`ed git leaves them behind and every later update of that
/// repository fails), then `SIGKILL` after [`TERM_GRACE`] for anything still
/// running, such as a helper that ignores `SIGTERM`.
fn kill_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        use nix::sys::signal::{killpg, Signal};
        // The child was started as the leader of a new process group (see
        // `run_git`), so its pid is the group id.
        if let Ok(pid) = i32::try_from(child.id()) {
            let group = nix::unistd::Pid::from_raw(pid);
            let _ = killpg(group, Signal::SIGTERM);
            let deadline = Instant::now() + TERM_GRACE;
            while Instant::now() < deadline && matches!(child.try_wait(), Ok(None)) {
                thread::sleep(POLL_INTERVAL);
            }
            let _ = killpg(group, Signal::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The `safe.directory` value that names the canonical path `real` the way
/// git compares it.
///
/// On Windows `canonicalize` returns a verbatim path (`\\?\C:\backup\r.git`,
/// `\\?\UNC\server\share\r.git`), which git never matches; the prefix is
/// dropped and the separators become `/`, the form git uses internally.
fn safe_directory_value(real: &Path) -> String {
    let text = strip_verbatim_prefix(&real.display().to_string());
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text
    }
}

/// `\\?\C:\x` becomes `C:\x` and `\\?\UNC\server\share` becomes
/// `\\server\share`; any other text is returned unchanged.
fn strip_verbatim_prefix(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match path.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_owned(),
        _ => path.to_owned(),
    }
}

/// `true` for the URL forms git hands to `ssh`: `ssh://`, `git+ssh://`,
/// `ssh+git://` and the scp-like `[user@]host:path`.
fn is_ssh_url(url: &str) -> bool {
    if let Some((scheme, _)) = url.split_once("://") {
        return matches!(
            scheme.to_ascii_lowercase().as_str(),
            "ssh" | "git+ssh" | "ssh+git"
        );
    }
    // scp-like syntax: a colon before any slash.  A single letter before the
    // colon is a Windows drive (`C:\repo`), which git treats as a path.
    match url.split_once(':') {
        Some((host, _)) => !host.contains('/') && host.len() > 1,
        None => false,
    }
}

/// `true` if the user chose how git runs `ssh`: the `GIT_SSH_COMMAND` or
/// `GIT_SSH` environment variable, or `core.sshCommand` in the configuration
/// that applies in `cwd`.
fn ssh_command_configured(git: &GitCommand, cwd: &Path) -> bool {
    if std::env::var_os("GIT_SSH_COMMAND").is_some() || std::env::var_os("GIT_SSH").is_some() {
        return true;
    }
    git.command()
        .args(["config", "--get", "core.sshCommand"])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Runs `git` with `args` in `cwd` and waits for it, applying the
/// supervision rules described in the [module documentation](self).
///
/// If `token` is `Some` and `url` is an HTTP(S) URL, the token is supplied to
/// git in an environment variable together with a credential helper scoped to
/// that URL's origin (see [`credential`]): no file is created and the token is
/// never part of an argument list.  `url` is the remote the command talks to.
pub(super) fn run_git(
    git: &GitCommand,
    args: &[&str],
    cwd: &Path,
    url: &str,
    token: Option<&str>,
    opts: &CloneOptions,
) -> Result<(), CoreError> {
    if opts.cancel.is_cancelled() {
        return Err(CoreError::Interrupted);
    }
    debug!(args = ?args, cwd = %cwd.display(), "running git");

    // Global options go before the subcommand.
    let mut global: Vec<String> = Vec::new();
    // We manage this repository, whoever owns it on disk (Unraid "New
    // Permissions", a restore as another uid): exactly this path is trusted,
    // never `*`.  Command-line config counts as protected configuration.
    if cwd != Path::new(".") {
        if let Ok(real) = cwd.canonicalize() {
            global.push("-c".into());
            global.push(format!("safe.directory={}", safe_directory_value(&real)));
        }
    }
    if token.is_some() {
        global.extend(credential::config_args(url));
    }

    let mut cmd = git.command();
    cmd.args(&global)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        // Never prompt: a missing/rejected credential must fail, not hang.
        .env("GIT_TERMINAL_PROMPT", "0")
        // Stable, English messages: error classification matches on them.
        .env("LC_ALL", "C")
        .env("LANGUAGE", "C")
        // Without a terminal git-lfs is silent, and silence is a stall.
        .env("GIT_LFS_FORCE_PROGRESS", "1");
    // ssh asks its questions on the terminal, where nobody may be watching.
    if is_ssh_url(url) && !ssh_command_configured(git, cwd) {
        cmd.env("GIT_SSH_COMMAND", BATCH_SSH_COMMAND);
    }
    if let Some(tok) = token {
        if credential::origin(url).is_some() {
            cmd.env(credential::TOKEN_ENV, tok);
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // New process group so a stall/shutdown kill reaches git's helpers.
        cmd.process_group(0);
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
                    stderr: with_hints(tail),
                });
            }
            None => {
                if opts.cancel.is_cancelled() {
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
        assert!(
            !out.contains("Cloning into"),
            "git's banner only buries the reason: {out}"
        );
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

    /// Regression (Windows): `canonicalize` returns `\\?\` paths, which git's
    /// `safe.directory` never matches, so the trust it was meant to give was
    /// not given.
    #[test]
    fn verbatim_windows_prefixes_are_dropped() {
        assert_eq!(
            strip_verbatim_prefix(r"\\?\C:\backup\o\git\r.git"),
            r"C:\backup\o\git\r.git"
        );
        assert_eq!(
            strip_verbatim_prefix(r"\\?\UNC\nas\share\r.git"),
            r"\\nas\share\r.git"
        );
        // A verbatim path that is not a disk or UNC path has no other form.
        assert_eq!(
            strip_verbatim_prefix(r"\\?\Volume{x}\r.git"),
            r"\\?\Volume{x}\r.git"
        );
        assert_eq!(strip_verbatim_prefix("/backup/r.git"), "/backup/r.git");
        assert_eq!(
            safe_directory_value(Path::new("/backup/o/r.git")),
            "/backup/o/r.git"
        );
    }

    #[test]
    fn ssh_urls_are_recognised() {
        for url in [
            "git@github.com:o/r.git",
            "github.com:o/r.git",
            "ssh://git@github.com/o/r.git",
            "SSH://github.com:22/o/r.git",
            "git+ssh://github.com/o/r.git",
        ] {
            assert!(is_ssh_url(url), "{url}");
        }
        for url in [
            "https://github.com/o/r.git",
            "http://127.0.0.1:9/o/r.git",
            "file:///srv/r.git",
            "/srv/repos/r.git",
            "./relative:odd/r.git",
            "C:\\repos\\r.git",
        ] {
            assert!(!is_ssh_url(url), "{url}");
        }
    }

    #[test]
    fn a_lock_file_error_gets_a_hint() {
        let text = with_hints(
            "error: cannot lock ref 'refs/heads/main': Unable to create '/b/r.git/refs/heads/main.lock': File exists.".into(),
        );
        assert!(text.contains("hint: a git process holds"), "{text}");
        assert_eq!(with_hints("fatal: other".into()), "fatal: other");
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
