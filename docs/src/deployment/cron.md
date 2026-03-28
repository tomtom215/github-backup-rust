# Cron

Schedule `github-backup` with cron for periodic backups.

## Basic Cron Entry

Edit the crontab for a dedicated user:

```bash
sudo -u github-backup crontab -e
```

Add an entry to run daily at 02:00:

```cron
# Backup GitHub daily at 02:00
0 2 * * * GITHUB_TOKEN=ghp_xxx /usr/local/bin/github-backup \
    --config /etc/github-backup/config.toml \
    >> /var/log/github-backup.log 2>&1
```

## Using an Environment File

Avoid embedding the token in crontab:

```bash
# /etc/github-backup/run.sh
#!/bin/bash
set -euo pipefail
source /etc/github-backup/secrets.env
exec /usr/local/bin/github-backup --config /etc/github-backup/config.toml
```

```bash
chmod 750 /etc/github-backup/run.sh
```

Crontab:

```cron
0 2 * * * /etc/github-backup/run.sh >> /var/log/github-backup.log 2>&1
```

## Log Rotation

Add a logrotate config to prevent logs from growing unbounded:

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

Save to `/etc/logrotate.d/github-backup`.

## Notes

- Prefer **systemd timers** over cron when available — they provide better logging, dependency handling, and `OnCalendar` expressions.
- Set `MAILTO=""` in the crontab to suppress emails on stderr output (which is normal for info-level logs).
- Ensure the user running cron has write access to `--output`.
