# Monitoring & Reporting

`github-backup` can write a machine-readable JSON summary after every run.
Combine it with standard monitoring tooling (cron, systemd, Prometheus,
Alertmanager, Grafana, DataDog, etc.) to get alerted when a backup fails or
drifts.

---

## JSON Summary Report

Pass `--report <FILE>` to write a structured JSON file at the end of each run:

```bash
github-backup octocat \
  --token "$GITHUB_TOKEN" \
  --output /var/backup/github \
  --all \
  --report /var/log/github-backup/report.json
```

### Report Schema

```json
{
  "tool_version": "0.3.0",
  "owner": "octocat",
  "started_at": "2026-01-15T04:00:00Z",
  "duration_secs": 142.7,
  "repos_discovered": 42,
  "repos_backed_up": 40,
  "repos_skipped": 2,
  "repos_errored": 0,
  "gists_backed_up": 5,
  "issues_fetched": 1204,
  "prs_fetched": 387,
  "workflows_fetched": 18,
  "success": true
}
```

| Field | Type | Description |
|-------|------|-------------|
| `tool_version` | string | `github-backup` semver |
| `owner` | string | GitHub username or org backed up |
| `started_at` | string | ISO 8601 UTC timestamp |
| `duration_secs` | float | Wall-clock seconds elapsed |
| `repos_discovered` | integer | Repos returned by the GitHub API |
| `repos_backed_up` | integer | Repos successfully backed up |
| `repos_skipped` | integer | Repos skipped by filters or dry-run |
| `repos_errored` | integer | Repos with non-fatal errors |
| `gists_backed_up` | integer | Gists backed up |
| `issues_fetched` | integer | Total issues fetched across all repos |
| `prs_fetched` | integer | Total pull requests fetched across all repos |
| `workflows_fetched` | integer | Total GitHub Actions workflows fetched across all repos |
| `success` | bool | `true` when `repos_errored == 0` |

---

## Shell-based Alerting

A minimal wrapper script that sends an email on failure:

```bash
#!/usr/bin/env bash
set -euo pipefail

REPORT=/var/log/github-backup/report.json
github-backup octocat --token "$GITHUB_TOKEN" --output /backup --all \
  --report "$REPORT"

SUCCESS=$(python3 -c "import json,sys; d=json.load(open('$REPORT')); print(d['success'])")
if [ "$SUCCESS" != "True" ]; then
  mail -s "github-backup FAILED" ops@example.com < "$REPORT"
fi
```

---

## Prometheus / Pushgateway

Push metrics after each run with a small shell one-liner:

```bash
REPORT=/var/log/github-backup/report.json
github-backup octocat --token "$GITHUB_TOKEN" --output /backup --all \
  --report "$REPORT"

python3 - <<'EOF'
import json, time, urllib.request

r = json.load(open('/var/log/github-backup/report.json'))
metrics = f"""# HELP github_backup_repos_backed_up Repositories backed up
# TYPE github_backup_repos_backed_up gauge
github_backup_repos_backed_up{{owner="{r['owner']}"}} {r['repos_backed_up']}
# HELP github_backup_repos_errored Repositories with errors
# TYPE github_backup_repos_errored gauge
github_backup_repos_errored{{owner="{r['owner']}"}} {r['repos_errored']}
# HELP github_backup_duration_seconds Backup duration
# TYPE github_backup_duration_seconds gauge
github_backup_duration_seconds{{owner="{r['owner']}"}} {r['duration_secs']}
# HELP github_backup_success Last backup success (1=ok, 0=fail)
# TYPE github_backup_success gauge
github_backup_success{{owner="{r['owner']}"}} {1 if r['success'] else 0}
"""
req = urllib.request.Request(
    'http://pushgateway:9091/metrics/job/github-backup',
    data=metrics.encode(),
    method='PUT',
)
urllib.request.urlopen(req)
print("Metrics pushed.")
EOF
```

---

## Grafana Dashboard Panels

Suggested panels for a Grafana dashboard backed by Pushgateway or a scrape job:

| Panel | Query |
|-------|-------|
| Repos backed up | `github_backup_repos_backed_up` |
| Repos with errors | `github_backup_repos_errored` |
| Backup duration (s) | `github_backup_duration_seconds` |
| Last success | `github_backup_success` |
| Time since last success | `time() - github_backup_last_success_timestamp_seconds` |

---

## Systemd `OnSuccess=` / `OnFailure=` Hooks

On systemd systems, trigger a notification service when the backup fails:

```ini
# /etc/systemd/system/github-backup.service
[Unit]
Description=GitHub Backup
OnFailure=notify-github-backup-failure@%n.service

[Service]
Type=oneshot
EnvironmentFile=/etc/github-backup/env
ExecStart=/usr/local/bin/github-backup \
  --config /etc/github-backup/config.toml \
  --report /var/log/github-backup/report.json
```

```ini
# /etc/systemd/system/notify-github-backup-failure@.service
[Unit]
Description=Notify on github-backup failure

[Service]
Type=oneshot
ExecStart=/usr/bin/mail -s "github-backup FAILED on %H" ops@example.com
```

---

## Log-based Alerting

`github-backup` writes structured log lines to `stderr` using the
`tracing` framework.  The final summary line always matches:

```
INFO repos: N backed up, N skipped, N errored; gists: N backed up (N.Ns elapsed)
```

Any `ERROR` line indicates a fatal failure.  Pass logs to your preferred log
aggregator (Loki, Splunk, CloudWatch Logs, etc.) and alert on `level=ERROR`.

### Example: Loki alert rule

```yaml
groups:
  - name: github-backup
    rules:
      - alert: GitHubBackupFailed
        expr: |
          count_over_time({job="github-backup"} |= "ERROR" [1h]) > 0
        for: 0m
        labels:
          severity: critical
        annotations:
          summary: "github-backup encountered an error"
```

---

## Incremental Backups

Use `--since` to limit API calls to items updated after the last run:

```bash
# Full backup on first run
github-backup octocat --token "$GITHUB_TOKEN" --output /backup \
  --all --report /var/log/github-backup/report.json

# Subsequent runs: only fetch issues/PRs updated since the last run
LAST_RUN=$(python3 -c "import json; print(json.load(open('/var/log/github-backup/report.json'))['started_at'])")
github-backup octocat --token "$GITHUB_TOKEN" --output /backup \
  --all --since "$LAST_RUN" --report /var/log/github-backup/report.json
```

> **Note** — `--since` only affects issues and pull requests.  Repository git
> mirrors are always updated incrementally via `git remote update`.

---

## Further Reading

- [Systemd Timer](deployment/systemd.md) — scheduling with `systemd`
- [Cron](deployment/cron.md) — scheduling with `cron`
- [CLI Reference](configuration/cli-reference.md) — all flags including `--report` and `--since`
