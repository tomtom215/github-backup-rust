# Interactive TUI

`github-backup` ships a full-screen terminal user interface built with
[Ratatui](https://ratatui.rs) 0.30.  Pass `--tui` and the tool enters the TUI
instead of running non-interactively.

```bash
# Recommended first-run experience
export GITHUB_TOKEN=ghp_your_token_here
github-backup octocat --tui

# Flags pre-fill the Configure screen
github-backup octocat --tui --token "$GITHUB_TOKEN" --output /var/backup/github \
  --all --private --dry-run --org --concurrency 8 --manifest
```

The command line is resolved exactly as for a normal run, so `--all`,
`--private`, `--forks`, `--dry-run`, `--org`, `--full`, `--since`,
`--include-repos`, `--clone-type`, `--concurrency`, `--manifest` and the
individual category flags all arrive in the Configure screen as initial
values.  Review and adjust them, then start the backup.  Not carried over:
`--config` files, `--clone-host`, and every option listed under
[What the TUI cannot do](#what-the-tui-cannot-do).

The TUI needs an interactive terminal on stdout.  Without one (a pipe, cron,
`docker run` without `-t`) it prints a one-line explanation and exits with
status 2.

---

## Screen Overview

The title bar lists five screens.  Press the number key to switch (not while a
backup is running, and not while editing a field).  On narrow terminals the
title bar shrinks to digits.

```
 github-backup v0.3.2  [1]Dashboard  [2]Configure  [3]Run  [4]Verify  [5]Results
```

Terminals smaller than 30x8 show a "Terminal too small" notice instead of a
clipped screen.

### 1 — Dashboard

Owner, output directory, token status (never shown, only "configured") and the
last recorded run: time, repository count and result (`complete`, or
`INCOMPLETE (N failures)`).  The last run is read from
`<output>/<owner>/json/backup_history.json`, falling back to
`backup_state.json`, and refreshed whenever you return to the Dashboard.

### 2 — Configure

Six tabs:

| Tab | Contents |
|-----|----------|
| Auth | GitHub token (always masked), API URL for GitHub Enterprise (`https://` only) |
| Target | Owner, output directory, organisation mode, since date, **Full backup (ignore state)** |
| Categories | 34 backup-category toggles |
| Clone | Clone type (mirror/bare/full/shallow), forks, private, LFS, prefer-SSH, no-prune, concurrency (1–64) |
| Filter | include / exclude repository globs (comma-separated) |
| Output | Write SHA-256 manifest, dry run |

*Full backup* ignores the saved incremental state and re-fetches everything.
It cannot be combined with *Since*, as on the command line.  The Output tab
ends with a note: **Command line only: mirror push, S3 sync, JSON report,
Prometheus metrics, webhook notification and device-flow sign-in.**

The focused field's help text is shown under the list when there is room.
Starting a backup validates the form (owner, token, `https://` API URL, date
format, concurrency range) and shows what to fix in a dialog.

### 3 — Run

Live view of an active backup:

- **Progress gauge** — repositories finished out of discovered, with a failure
  count.  As soon as one repository fails the title reads
  `INCOMPLETE: N failed` and the bar turns red.
- **Repo list** — `>>` running, `ok` done, `!!` failed (with the reason),
  `--` skipped.  Failed repositories are pinned to the top so they never
  scroll away.
- **Log panel** — one row per entry (long lines are cut, never wrapped), so the
  newest entry is always visible while following.  Only this tool's own INFO
  and above are shown by default; set `RUST_LOG` to change that.
- **Counters** — repos finished/total, failed, elapsed (stops when the run
  ends), and key hints.

With no backup running this screen just says so; `Esc` or `q` leave it.

### 4 — Verify

Checks the JSON files listed in `backup_manifest.json` against the disk.  It
does **not** cover git mirrors.  The manifest exists only if you ran a backup
with *Write SHA-256 Manifest* (or `--manifest`); otherwise Verify says so.
`v` starts it.

### 5 — Results

The verdict follows the engine, never the repo counters alone:

| Headline | Meaning |
|----------|---------|
| `BACKUP COMPLETE` | ran to the end, nothing failed |
| `BACKUP INCOMPLETE - N failures` | ran to the end but something failed; **not a complete backup** |
| `DRY RUN COMPLETE - nothing was written` | dry run, no failures |
| `BACKUP CANCELLED` | stopped with Ctrl+C |
| `BACKUP FAILED` | stopped by a fatal error; the message is shown |

Below the headline: counters (repositories discovered / backed up / skipped /
failed, gists, issues, pull requests, workflows, discussions) and, when
anything failed, a scrollable **Failures** list with the scope (usually
`owner/repo`), the step (`clone`, `wiki`, `issues`, …) and the message.  The
selected failure's full text is shown underneath when there is room.

---

## Key Reference

### Global

| Key | Action |
|-----|--------|
| `1`–`5` | Switch screens (disabled while a backup runs or a field is being edited) |
| `Ctrl+C` | Cancel the running backup; anywhere else, quit |
| any key | Dismiss an error dialog |

### Dashboard

| Key | Action |
|-----|--------|
| `j` / `k` or `↓` / `↑` | Select action |
| `Enter` | Run the selected action |
| `r` / `c` / `v` / `q` | Run backup / Configure / Verify / Quit |

### Configure

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Next / previous tab |
| `j` / `k` or `↓` / `↑` | Next / previous field |
| `Space` | Toggle a checkbox |
| `Enter` | Edit a text field (toggles a checkbox) |
| `←` / `→` | Change the Clone type |
| `A` | Categories tab: select all, or none when all are on |
| `s` / `F5` | Start the backup |
| `Esc` | Back to the Dashboard |

While editing a text field: `Enter` **saves**, `Esc` **discards** the change,
`Backspace` deletes.  Pasting inserts a single clean line; a paste never
triggers shortcuts.  The Concurrency field accepts digits only.  There is no
cursor movement inside a field.

### Run

| Key | Action |
|-----|--------|
| `Ctrl+C` | Cancel (stops git and releases the lock; Results then shows `CANCELLED`) |
| `j` / `k` | Scroll the repo list |
| `g` / `G` | Log: oldest / follow newest |
| `PgUp` / `PgDn` | Scroll the log |
| `Esc` / `q` | Leave the screen (only when no backup is running) |

### Verify

| Key | Action |
|-----|--------|
| `v` | Start verification |
| `j` / `k`, `PgUp` / `PgDn` | Scroll |
| `Esc` / `d` | Dashboard |
| `q` | Quit |

### Results

| Key | Action |
|-----|--------|
| `j` / `k`, `PgUp` / `PgDn` | Select failure |
| `g` / `G` | First / last failure |
| `r` | Run again (starts a new backup immediately) |
| `d` / `Esc` | Dashboard |
| `c` | Configure |
| `q` | Quit |

---

## What the TUI cannot do

These exist only on the command line, and the TUI does not offer them:

- mirror push (Gitea/GitLab), S3 sync, JSON report, Prometheus metrics, webhook
  notification;
- device-flow sign-in (a token is required);
- `--config` files and `--clone-host`;
- the CLI-level output lock.

What the TUI does after a run, like the CLI: the engine writes the incremental
state, the TUI appends a run-history entry (shown on the Dashboard, including
the failure count) and, if enabled and not a dry run, writes the manifest.  A
manifest that cannot be written counts as a failure of the run.

## Terminal safety

Raw mode, the alternate screen and bracketed paste are always restored on
exit, on a panic (the message is printed after the terminal is restored), and
on `SIGINT`, `SIGTERM`, `SIGHUP` and `SIGQUIT`.  Such a signal cancels a
running backup first (git is killed, the lock released) and exits with
128 + the signal number.  A second signal exits immediately.  `SIGKILL` cannot
be handled; run `reset` if you ever use it.  Closing the terminal window ends
the process.

## Architecture Notes

The TUI lives in the `github-backup-tui` crate and drives the same
`BackupEngine` as the CLI.  Per-repository progress and outcome arrive over the
engine's event channel; the final verdict and failure list come from
`BackupStats`.  A tracing layer forwards log events to the Run screen's log
panel.  Terminal input is read on a dedicated thread; the async loop redraws
only when something changed.

See [Architecture](development/architecture.md) for the crate dependency graph.
