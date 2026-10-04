# Systemd Timer

Run `github-backup` automatically on a schedule using a systemd service + timer pair.

## Setup

### 1. Create a dedicated user

```bash
sudo useradd -r -m -d /var/backup/github -s /sbin/nologin github-backup
sudo mkdir -p /var/backup/github
sudo chown github-backup:github-backup /var/backup/github
```

### 2. Store the token securely

```bash
sudo mkdir -p /etc/github-backup
sudo tee /etc/github-backup/secrets.env > /dev/null <<'EOF'
GITHUB_TOKEN=ghp_your_token_here
EOF
sudo chmod 600 /etc/github-backup/secrets.env
sudo chown root:github-backup /etc/github-backup/secrets.env
```

### 3. Create a config file

```bash
sudo tee /etc/github-backup/config.toml > /dev/null <<'EOF'
owner = "octocat"
output = "/var/backup/github"
concurrency = 8
repositories = true
issues = true
pulls = true
releases = true
wikis = true
gists = true
EOF
sudo chmod 644 /etc/github-backup/config.toml
```

### 4. Create the service unit

```bash
sudo tee /etc/systemd/system/github-backup.service > /dev/null <<'EOF'
[Unit]
Description=GitHub Backup
After=network-online.target
Wants=network-online.target
OnFailure=notify-failure@%n.service

[Service]
Type=oneshot
User=github-backup
Group=github-backup
EnvironmentFile=/etc/github-backup/secrets.env
ExecStart=/usr/local/bin/github-backup --config /etc/github-backup/config.toml --report /var/lib/github-backup/report.json
StateDirectory=github-backup
StandardOutput=journal
StandardError=journal
SyslogIdentifier=github-backup

# Hardening
ProtectSystem=strict
ReadWritePaths=/var/backup/github
PrivateTmp=true
NoNewPrivileges=true
EOF
```

### 5. Create the timer unit

```bash
sudo tee /etc/systemd/system/github-backup.timer > /dev/null <<'EOF'
[Unit]
Description=Run GitHub Backup daily at 02:00
Requires=github-backup.service

[Timer]
OnCalendar=*-*-* 02:00:00
RandomizedDelaySec=1800
Persistent=true

[Install]
WantedBy=timers.target
EOF
```

### 6. Enable and start

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now github-backup.timer
```

### 7. Verify

```bash
# Check timer status
systemctl status github-backup.timer

# Run immediately to test
sudo systemctl start github-backup.service

# Follow logs
journalctl -u github-backup.service -f
```

## Exit Status and Failure Handling

The unit fails whenever the exit status is not `0`: `1` (the run could not
run), `3` (the run finished but something could not be backed up) and `130` /
`143` (stopped).  `OnFailure=` therefore fires for an incomplete backup too, and
`systemctl status github-backup.service` shows the status.  Do **not** add
`SuccessExitStatus=3`: that would hide incomplete backups.  `journalctl` shows
the summary banner with the list of failures.  See
[Monitoring](../monitoring.md) for the report file, metrics and webhook.

A failure unit that sends mail:

```ini
# /etc/systemd/system/notify-failure@.service
[Unit]
Description=Notify on failure of %i

[Service]
Type=oneshot
ExecStart=/usr/bin/mail -s "%i failed on %H" ops@example.com
```

`StateDirectory=github-backup` creates `/var/lib/github-backup` (owned by the
service user) for the `--report` file; with `ProtectSystem=strict` the report
path must be writable, which `StateDirectory=` provides.

## Stopping and Timeouts

`systemctl stop` sends `SIGTERM`.  The backup stops its `git` processes, releases
its lock and exits `143` within a few seconds; the next run resumes (within 6
hours) from the checkpoint.  No cleanup of lock files is ever needed.

## Multiple Owners

Create separate service/timer pairs per owner, or one script that does not stop
at the first failure:

```bash
#!/bin/bash
# /usr/local/bin/github-backup-all.sh
status=0
for owner in octocat myorg another-org; do
  github-backup "$owner" --config /etc/github-backup/config.toml \
    --output /var/backup/github || status=$?
done
exit "$status"
```

(Each owner needs its own `--report` and `--prometheus-metrics` file if you use
them: both hold one run.  If the config file has `org = true` it applies to every
owner; keep organisations and users in separate configs.)  The last non-zero
status is the one systemd sees.

## Monitoring

```bash
systemctl list-timers github-backup.timer
journalctl -u github-backup.service --since "yesterday"
```
