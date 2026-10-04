# Cron

Schedule `github-backup` with cron for periodic backups.  (A
[systemd timer](systemd.md) gives better logging and failure handling when it is
available.)

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
- Prefer `systemd` timers where available.
