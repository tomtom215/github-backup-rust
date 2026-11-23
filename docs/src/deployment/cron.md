# Cron

Schedule `github-backup` with cron for periodic backups.  (A
[systemd timer](systemd.md) gives better logging and failure handling when it is
available, and in a container or anywhere without a scheduler the
[built-in schedule](#built-in-schedule---every) needs neither.)  The examples
run `/usr/local/bin/github-backup`; `sudo sh install.sh --dir /usr/local/bin`
installs the release binary there (see
[Installation](../getting-started/installation.md#install-script-linux-macos)).

## Built-in Schedule (`--every`)

`--every INTERVAL` keeps the process running: it backs up at once, then again
every INTERVAL (a whole number with a unit: `30m`, `6h`, `24h`, `7d`; at least
`1m`).

```bash
github-backup --output /backup --every 24h
```

- The interval is measured from the start of each run.  A run that takes longer
  than the interval is followed at once by the next one.
- Every run is a fresh run: configuration, client and engine are built anew, and
  each run writes its own history entry, `--report`, metrics and notification.
  The output lock is held only while a run is in progress.
- A run that fails (exit status `1`) or is incomplete (`3`) is logged and the
  loop carries on: the next run retries it.  Watch the `--report` file,
  `--prometheus-metrics` or `--notify-webhook` (for example
  `--notify-format healthchecks`) to hear about failures.
- After each run the log says when the next one is due
  (`next backup in 23h 58m 12s next_run=...`).
- `SIGINT` / `SIGTERM` (`docker stop`, `systemctl stop`) while waiting ends the
  process with status `0`; during a run they interrupt it as usual (status
  `130` / `143`).
- The token is read once at startup (from `GITHUB_TOKEN`, `GH_TOKEN`, the
  config file or the GitHub CLI); a token that expires later makes the later
  runs fail until the process is restarted with a new one.

## Keep the Token Out of the Crontab

Put the token in a file readable only by the backup user and load it in a small
wrapper:

```bash
sudo install -d -m 750 -o github-backup -g github-backup /etc/github-backup
sudo -u github-backup sh -c 'umask 077; echo "GITHUB_TOKEN=ghp_your_token_here" > /etc/github-backup/secrets.env'
```

```bash
#!/bin/bash
# /etc/github-backup/run.sh   (chmod 750)
set -uo pipefail
set -a; . /etc/github-backup/secrets.env; set +a
LOG=/var/log/github-backup.log

/usr/local/bin/github-backup --config /etc/github-backup/config.toml \
    --report /var/lib/github-backup/report.json >> "$LOG" 2>&1
status=$?

# 0 = complete, 3 = finished but incomplete, 1 = could not run, 130/143 = stopped.
# Whatever this script prints is mailed by cron, so print only on failure.
if [ "$status" -ne 0 ]; then
  echo "github-backup exited with status $status on $(hostname); last log lines:" >&2
  tail -n 20 "$LOG" >&2
fi
exit "$status"
```

Crontab for the `github-backup` user:

```cron
# Daily at 02:00.  The wrapper logs everything to /var/log/github-backup.log and
# prints (so cron mails) only when the backup failed or is incomplete.
MAILTO=ops@example.com
0 2 * * * /etc/github-backup/run.sh
```

Cron mails whatever a job prints, and `github-backup` writes its log to stderr, so
a job without the wrapper's redirect would mail the whole log every day.  Test the
wrapper by hand (`sudo -u github-backup /etc/github-backup/run.sh`) before relying
on it: cron runs with a minimal environment, so use absolute paths.  The log
file and the report directory must be writable by the backup user
(`install -d -o github-backup /var/lib/github-backup`, and create
`/var/log/github-backup.log` with `touch` and `chown` once).

## Failure Detection

The exit status is the signal: `0` complete, `3` finished but incomplete, `1` could
not run.  Do not assume "no error mail" means success.  For stronger monitoring
use `--prometheus-metrics`, `--notify-webhook` or an external check of the report
file's age; see [Monitoring](../monitoring.md).

## Log Rotation

```
/var/log/github-backup.log {
    daily
    rotate 30
    compress
    delaycompress
    missingok
    notifempty
    create 640 github-backup adm
}
```

Save it as `/etc/logrotate.d/github-backup`.

## Notes

- Do not run two backups for the same owner at once: the second one stops with
  "another backup ... is already running" (exit `1`).  A run that outlasts the
  schedule interval is therefore harmless, but it shows up as a failed run.
- The user running cron needs write access to `--output` and to the `--report`
  directory.
- Prefer `systemd` timers where available, or `--every` where there is no
  scheduler at all.
