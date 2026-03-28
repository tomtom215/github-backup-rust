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

[Service]
Type=oneshot
User=github-backup
Group=github-backup
EnvironmentFile=/etc/github-backup/secrets.env
ExecStart=/usr/local/bin/github-backup --config /etc/github-backup/config.toml
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

## Multiple Owners

Create separate service/timer pairs for each owner, or use a wrapper script:

```bash
# /usr/local/bin/github-backup-all.sh
#!/bin/bash
set -euo pipefail
for owner in octocat myorg another-org; do
  github-backup "$owner" \
    --config /etc/github-backup/config.toml \
    --output /var/backup/github
done
```

## Monitoring

Check the last run time and status:

```bash
systemctl list-timers github-backup.timer
journalctl -u github-backup.service --since "yesterday"
```

Send a notification on failure using `OnFailure`:

```ini
[Unit]
OnFailure=notify-failure@%n.service
```
