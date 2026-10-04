# Monitoring & Reporting

A backup is only as good as your knowledge that it ran.  `github-backup`
reports every run in six ways, all derived from **one** list of failures so
that they cannot disagree:

| Signal | Where | Enabled by |
|--------|-------|-----------|
| Exit status | the process | always |
| Summary banner and log | stderr / stdout | always (`-q` hides the banner) |
| JSON report | a file | `--report FILE` |
| Prometheus metrics | a file for node_exporter | `--prometheus-metrics FILE` |
| Webhook | an HTTP POST | `--notify-webhook URL` |
| Run history | `<output>/<owner>/json/backup_history.json` | always |

A `--dry-run` produces none of them except the log and the banner.

---

## Exit Status

| Code | Meaning |
|------|---------|
| `0` | Everything succeeded. |
| `1` | The run could not be carried out (rejected token, repository list not available, lock held, full disk, bad configuration). |
| `2` | Usage error. |
| `3` | The run finished but is **incomplete**: at least one failure was recorded. |
| `130` / `143` | Interrupted by `SIGINT` / `SIGTERM`. |

Any non-zero status means "do not trust this run".  `3` is the interesting one:
the other repositories were backed up, and re-running retries only what failed.
Failures are isolated per repository and per category, so a repository whose
issue list cannot be fetched is still cloned, and one broken repository never
stops the others.  Full table: [CLI Reference](configuration/cli-reference.md#exit-codes).

---

## JSON Summary Report

`--report FILE` writes a JSON file at the end of **every** run, including a run
that failed early (so a monitor never keeps reading the previous run's
"success").  The file is replaced atomically.

```bash
github-backup octocat --output /var/backup/github --all \
  --report /var/log/github-backup/report.json
```

### Schema

```json
{
  "tool_version": "X.Y.Z",
  "schema_version": 1,
  "owner": "octocat",
  "started_at": "2026-01-15T04:00:00Z",
  "finished_at": "2026-01-15T04:02:23Z",
  "duration_secs": 142.7,
  "repos_discovered": 42,
  "repos_backed_up": 41,
  "repos_skipped": 0,
  "repos_errored": 1,
  "gists_backed_up": 5,
  "issues_fetched": 1204,
  "prs_fetched": 387,
  "workflows_fetched": 18,
  "failure_count": 1,
  "failures": [
    {
      "scope": "octocat/big-repo",
      "step": "repository",
      "message": "git clone ... failed (exit 128): ..."
    }
  ],
  "success": false
}
```

(`tool_version` is the version of the binary that wrote the file; `X.Y.Z` stands for it here.)

| Field | Type | Description |
|-------|------|-------------|
| `tool_version` | string | Version of `github-backup` |
| `schema_version` | integer | Version of this schema (currently `1`) |
| `owner` | string | User or organisation backed up |
| `started_at`, `finished_at` | string | ISO 8601 UTC |
| `duration_secs` | float | Wall-clock seconds |
| `repos_discovered` | integer | Repositories GitHub listed for the owner |
| `repos_backed_up` | integer | Repositories for which every step succeeded |
| `repos_skipped` | integer | Repositories not processed in this run: excluded by the filters, or already finished in a resumed run |
| `repos_errored` | integer | Repositories with at least one failed step |
| `gists_backed_up` | integer | Gists cloned |
| `issues_fetched`, `prs_fetched`, `workflows_fetched` | integer | Items fetched |
| `failure_count` | integer | Number of recorded failures |
| `failures` | array | One entry per failure: `scope` (a repository such as `octocat/big-repo`, the owner, or `post-processing`), `step` (what was being done) and `message` |
| `success` | bool | `true` exactly when `failure_count` is `0` |

`failures` includes things that are not repositories: an S3 upload that failed
(`scope: "post-processing"`, `step: "s3 sync"`), a mirror push that was refused
(`"mirror push"`), a manifest that could not be written, a starred repository
that could not be cloned.  `message` text is scrubbed of the token.

### Shell wrapper

```bash
#!/usr/bin/env bash
set -uo pipefail
REPORT=/var/log/github-backup/report.json

github-backup octocat --output /backup --all --report "$REPORT"
status=$?

if [ "$status" -ne 0 ]; then
  mail -s "github-backup exit $status on $(hostname)" ops@example.com < "$REPORT"
fi
exit "$status"
```

Testing the exit status is enough; the report adds the reason.  With `jq`:

```bash
jq -r '.failures[] | "\(.scope): \(.step)"' /var/log/github-backup/report.json
```

---

## Webhook

`--notify-webhook URL` (or `BACKUP_NOTIFY_WEBHOOK`) sends one JSON `POST` after
the run, also when the run failed:

```json
{
  "status": "partial",
  "owner": "octocat",
  "timestamp": "2026-01-15T04:02:23Z",
  "repos_backed_up": 41,
  "repos_errored": 1,
  "failure_count": 1,
  "failed": [ { "scope": "octocat/big-repo", "step": "repository" } ]
}
```

| `status` | Meaning | Exit status |
|----------|---------|-------------|
| `success` | nothing failed | `0` |
| `partial` | finished with failures | `3` |
| `failure` | the run could not be completed; `error` holds the reason (and `failed` the failure) | `1` |

`failed` lists at most the first 20 failures and never includes messages (they
can be long and the receiver is an external service); `failure_count` is the
full number.  A webhook that cannot be reached, or answers an error, is logged
as a warning and **does not change the exit status**.  The notification is not
sent for a dry run, nor when the tool stops before the run starts (a bad
argument, an unreadable config file, a lock held by another run, a failed
device-flow login): treat the exit status as the primary signal and the
webhook as a convenience.

Use an `https://` URL: the payload names the owner and a plain-`http://` URL
produces a warning.  The log shows only the scheme and host of the URL, never
its path or query.  The request honours `HTTPS_PROXY`.

---

## Prometheus Metrics

`--prometheus-metrics FILE` writes metrics in the Prometheus text format for
node_exporter's textfile collector.  The file is replaced atomically after
every run, including failed ones.

```bash
github-backup octocat --output /backup --all \
  --prometheus-metrics /var/lib/node_exporter/textfile_collector/github_backup.prom
```

All metrics carry an `owner` label.  The file holds one owner: when you back up
several owners, give each its own file.

| Metric | Meaning |
|--------|---------|
| `github_backup_success` | `1` if the last run had no failures, else `0` |
| `github_backup_failures` | failures recorded in the last run |
| `github_backup_repos_backed_up` | repositories backed up completely |
| `github_backup_repos_discovered` | repositories GitHub listed |
| `github_backup_repos_errored` | repositories with a failed step |
| `github_backup_issues_fetched`, `github_backup_prs_fetched` | items fetched |
| `github_backup_duration_seconds` | duration of the last run |
| `github_backup_last_run_timestamp_seconds` | when the last run started |
| `github_backup_last_success_timestamp_seconds` | when the last run **without failures** started |

`github_backup_last_success_timestamp_seconds` is carried over from the
previous file when a run fails, so it keeps pointing at the last good run.  If
no run has ever succeeded the metric is absent.

Alert rules:

```yaml
groups:
  - name: github-backup
    rules:
      - alert: GitHubBackupFailed
        expr: github_backup_success == 0
        for: 5m
        labels: { severity: critical }
        annotations:
          summary: "github-backup for {{ $labels.owner }} finished with failures"
      - alert: GitHubBackupStale
        expr: time() - github_backup_last_success_timestamp_seconds > 2 * 24 * 3600
        labels: { severity: critical }
        annotations:
          summary: "No complete github-backup for {{ $labels.owner }} in two days"
      - alert: GitHubBackupNeverSucceeded
        expr: absent(github_backup_last_success_timestamp_seconds)
        for: 1d
        labels: { severity: warning }
```

If the tool does not run at all (timer stopped, host down) no new file appears;
`GitHubBackupStale` catches that, `GitHubBackupFailed` does not.  Without
node_exporter, push the same values from the JSON report to a Pushgateway.

---

## Alerting With systemd

A failed run exits non-zero, so systemd marks the unit failed and `OnFailure=`
fires for exit status `1` **and** `3`:

```ini
# /etc/systemd/system/github-backup.service
[Unit]
Description=GitHub Backup
OnFailure=notify-github-backup-failure@%n.service

[Service]
Type=oneshot
EnvironmentFile=/etc/github-backup/secrets.env
ExecStart=/usr/local/bin/github-backup \
  --config /etc/github-backup/config.toml \
  --report /var/log/github-backup/report.json
```

```ini
# /etc/systemd/system/notify-github-backup-failure@.service
[Unit]
Description=Notify on failure of %i

[Service]
Type=oneshot
ExecStart=/usr/bin/mail -s "%i failed on %H" ops@example.com
```

See [Systemd Timer](deployment/systemd.md) for the full setup.  With cron, set
`MAILTO` (cron mails any output) or wrap the command as in the shell wrapper
above.

---

## Logs

Logs go to stderr, one event per line (`-v` adds debug detail, `-q` shows
errors only).  Do not build alerts on matching `WARN` or `ERROR` text: both
levels occur during a run that ends normally.  What the lines look like:

```
INFO repository processed repo=octocat/hello progress="2/3"
WARN step failed, continuing with the rest scope=octocat/big-repo step=repository error=git clone ...
INFO backup finished owner="octocat" stats=repos: 2/3 backed up, 0 skipped, 1 errored; gists: 0 backed up; issues: 5 fetched; PRs: 2 fetched; workflows: 1 fetched; 1 failure(s) (14.3s elapsed)
ERROR backup is incomplete: 1 item(s) could not be backed up (exit status 3) failures=1
```

`step failed, continuing` (WARN) is one recorded failure; `fatal error: stopping
the run` (ERROR) ends the run early (rejected credentials, exhausted rate-limit
budget, full disk); the final `backup is incomplete` / `backup failed` line
states the outcome.  The summary banner printed at the end lists each failure
with its reason.

---

## Incremental Runs and the State File

Run `github-backup` as often as you like: the second run is faster, and it
**never loses data** that the first one captured.

What happens each run:

* The repository list, **`issues.json` and `pulls.json` are always fetched in
  full** and merged into the stored files (an item that GitHub no longer lists
  stays in the backup).  The files can only grow or change; a run never shrinks
  them.
* The expensive part, the per-item requests (`issue_comments`, `issue_events`,
  `pull_comments`, `pull_commits`, `pull_reviews`), is skipped for an issue or
  pull request that has not been updated since the repository's **watermark**
  and whose files are already on disk.  A missing file is always fetched.
* Git clones are updated with `git fetch`, never cloned again.

The watermarks live in `<output>/<owner>/json/backup_state.json`:

* One watermark **per repository**, set to 15 minutes before the run started (a
  margin for clock differences and replication lag).
* A repository's watermark only advances when **every** step for it succeeded.
  A repository that failed keeps its old watermark, so the next run fetches
  what the failed run missed.
* A watermark covers the categories that were enabled when it was recorded.
  Turning on another per-item category later makes the first run fetch
  everything for it.
* `last_successful_run` is informational; it only moves after a run without
  failures.
* A missing or unreadable file means "fetch everything once".

Controls:

| | |
|--|--|
| `--full` | Ignore every watermark and fetch all per-item files again.  Use it for a periodic complete refresh. |
| `--since DATE` | Expert override: treat everything updated before DATE as already backed up (for every repository).  `DATE` is `2026-01-01` or an RFC 3339 timestamp.  Never written to the state file, so a mistyped date cannot affect later runs.  Cannot be combined with `--full`. |
| delete `backup_state.json` | Same effect as `--full` for the next run. |
| `--dry-run` | Does not read or write the state. |

> **Limitation.** "Unchanged" is judged by the issue's or pull request's
> `updated_at` timestamp.  GitHub does not guarantee that every edit to a
> comment, review or event bumps that timestamp, so an in-place edit of an old
> comment can be missed by an incremental run.  Run with `--full` now and then
> (for example weekly) if exact copies matter.

---

## Further Reading

- [Systemd Timer](deployment/systemd.md) and [Cron](deployment/cron.md): scheduling
- [CLI Reference](configuration/cli-reference.md): every flag and the exit codes
- [Operations Runbook](ops-runbook.md): what to do when an alert fires
