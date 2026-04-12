# Interactive TUI

`github-backup` ships a full-screen terminal user interface built with
[Ratatui](https://ratatui.rs) 0.30.  Pass `--tui` to any normal invocation
and the tool enters the TUI instead of running non-interactively.

```bash
# Recommended first-run experience
export GITHUB_TOKEN=ghp_your_token_here
github-backup octocat --tui

# Pre-seed common settings via flags; the TUI picks them up
github-backup octocat \
  --token "$GITHUB_TOKEN" \
  --output /var/backup/github \
  --tui
```

Any flags passed alongside `--tui` are loaded into the Configure screen as
initial values.  You can review and adjust every setting before starting
a backup run.

---

## Screen Overview

The title bar lists five screens.  Press the corresponding number key to
switch at any time.

```
 github-backup v0.3.2  [1]Dashboard  [2]Configure  [3]Run  [4]Verify  [5]Results
```

### 1 — Dashboard

The home screen.  Shows the owner, output directory, token status, and the
last successful run (date, repo count).  Use `j` / `k` to select an action
and `Enter` to activate it.

```
  Owner          octocat
  Output dir     /var/backup/github
  Token          ghp_****...****   (set)
  Last run       2026-03-29 08:14 UTC  (312 repos)

  > Start backup
    Verify integrity
    Configure
```

### 2 — Configure

All 50+ backup settings in a single screen, organised across eight tabs:

| Tab | Contents |
|-----|----------|
| Auth | Token, OAuth client ID / scopes, device-auth toggle |
| Target | Owner, output dir, org mode, concurrency, dry-run |
| Categories | 34 backup-category toggles (repos, issues, PRs, gists, …) |
| Clone | Clone type (mirror/bare/full/shallow), LFS, no-prune, prefer-SSH |
| Filter | include-repos, exclude-repos glob patterns, since date |
| Mirror | Mirror-to URL, mirror token, owner, type (Gitea/Gitlab), private |
| S3 | Bucket, region, prefix, endpoint, access key, secret key, include-assets |
| Output | Config file path, report path, API URL, clone host |

Navigate tabs with `h` / `l` (or `←` / `→`).  Move between fields with
`j` / `k`.  Press `Enter` to begin editing a text field; `Esc` to commit.
Toggle booleans with `Enter`.  Cycle select fields with `<` / `>`.

On the Categories tab, press `A` to select all or deselect all at once.

### 3 — Run

Live view of an active backup:

- **Progress gauge** — filled as repos complete; labelled with counts and
  the current phase name
- **Repo list** (35 % width) — all discovered repositories with status icons:
  ` .` pending, `>>` running, `ok` done, `!!` error, `--` skipped; auto-scrolls
  to keep the active repo visible
- **Log panel** (65 % width) — structured log lines from `tracing` (timestamp,
  level, message), scrollable with `g` / `G`
- **Stats bar** — repos done / total, error count, elapsed time, key hints

Scroll the repo list with `j` / `k`.  Scroll the log with `g` (top) / `G`
(bottom).  Cancel the running backup with `Ctrl+C`.

### 4 — Verify

Offline integrity check against the JSON manifest stored in the output directory.
Press `Enter` to start a verification run; the results appear in-place:

```
  Path: /var/backup/github/octocat/json
  Status: CLEAN

  Files OK: 4,821
  Tampered:  0
  Missing:   0
  Unexpected: 0
```

Scroll through tampered / missing / unexpected file lists with `j` / `k`.

### 5 — Results

Post-run statistics after each backup completes (or fails):

| Counter | Value |
|---------|-------|
| Repos discovered | 317 |
| Repos backed up | 312 |
| Repos skipped | 3 |
| Repos errored | 2 |
| Gists backed up | 14 |
| Issues fetched | 8,042 |
| PRs fetched | 1,203 |
| Workflows fetched | 289 |
| Elapsed | 4m 32s |

---

## Key Reference

### Global

| Key | Action |
|-----|--------|
| `1`–`5` | Switch screens |
| `q` | Quit (prompts to cancel if backup is running) |
| `Ctrl+C` | Quit / cancel backup |
| `Esc` | Dismiss error modal |

### Dashboard (`1`)

| Key | Action |
|-----|--------|
| `j` / `k` | Move selection down / up |
| `Enter` | Activate selected action |

### Configure (`2`)

| Key | Action |
|-----|--------|
| `h` / `l` or `←` / `→` | Previous / next tab |
| `j` / `k` or `↑` / `↓` | Move field cursor |
| `Enter` | Begin editing text field; toggle boolean |
| `Esc` | Commit field edit (text fields) |
| `< >` | Cycle select field options |
| `A` | Select-all / deselect-all (Categories tab only) |
| `Backspace` | Delete last character (text field edit mode) |

### Run (`3`)

| Key | Action |
|-----|--------|
| `j` / `k` | Scroll repo list down / up |
| `g` | Scroll log to top |
| `G` | Scroll log to bottom |
| `Ctrl+C` | Cancel running backup |

### Verify (`4`)

| Key | Action |
|-----|--------|
| `Enter` | Start verification |
| `j` / `k` | Scroll results list |

### Results (`5`)

| Key | Action |
|-----|--------|
| `r` | Return to Dashboard |

---

## Architecture Notes

The TUI lives in the `github-backup-tui` crate.  It does not embed its own
backup logic; it drives the same `BackupEngine` used by the CLI.  A
`tokio::sync::mpsc::UnboundedSender<BackupEvent>` is passed to the backup and
verify tasks; the event loop drains it every 16 ms and applies events to the
`App` state struct.

A custom `tracing_subscriber::Layer` (`TuiTracingLayer`) intercepts all
`tracing` events and forwards them as `BackupEvent::LogLine` so structured
log output appears in the Run screen's log panel instead of stderr.

The backup task is cancelled via a `tokio::sync::oneshot::Sender<()>` stored
on `App`; `Ctrl+C` sends the signal and the task exits cleanly within one
poll cycle.

See [Architecture](development/architecture.md) for the full crate dependency
graph and data flow diagram.
