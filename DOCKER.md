# Docker Guide

`github-backup-rust` ships a multi-stage `Dockerfile` that produces a
minimal Alpine-based image (~15 MB) containing only `git`, the CA
bundle, `tini`, and the backup binary.

> Running on **Unraid**?  A Community Applications template is bundled
> at [`unraid/github-backup.xml`](unraid/README.md).  It points at the
> same multi-arch GHCR image; the rest of this guide still applies if
> you want to drop down to the WebUI or `docker run` directly.

Multi-architecture images for `linux/amd64` and `linux/arm64` are
published to GHCR on every tagged release:

    ghcr.io/tomtom215/github-backup-rust:latest      # most recent stable
    ghcr.io/tomtom215/github-backup-rust:0.3.2       # exact version
    ghcr.io/tomtom215/github-backup-rust:0.3         # latest patch in 0.3.x

## Quick Start

```sh
# Pull the image
docker pull ghcr.io/tomtom215/github-backup-rust:latest

# Verify the host environment before running for real (no API calls billed):
docker run --rm \
  -e GITHUB_TOKEN=$GITHUB_TOKEN \
  ghcr.io/tomtom215/github-backup-rust:latest \
  octocat --doctor

# Run a backup
docker run --rm \
  -v "$PWD/backups:/backup" \
  -e GITHUB_TOKEN=$GITHUB_TOKEN \
  ghcr.io/tomtom215/github-backup-rust:latest \
  octocat --output /backup --all
```

To build the image locally instead:

```sh
docker build -t github-backup .
docker run --rm -e GITHUB_TOKEN=$GITHUB_TOKEN github-backup octocat --doctor
```

## Docker Compose

The `docker-compose.yml` file provides pre-configured services for
every common scenario.  Activate one with `--profile <name>`.

Copy the env template once:

```sh
cp compose.example.env .env
chmod 600 .env
$EDITOR .env            # at minimum: GITHUB_TOKEN
```

### Profile matrix

| Profile      | Service          | What it does |
|--------------|------------------|--------------|
| _default_    | `backup`         | Local backup, output under `./backups/` |
| `doctor`     | `doctor`         | Run `--doctor` pre-flight checks only |
| `tui`        | `tui`            | Launch the full-screen interactive TUI |
| `verify`     | `verify`         | Verify an existing backup's SHA-256 manifest |
| `s3`         | `backup-s3`      | Backup + sync to AWS S3 |
| `b2`         | `backup-b2`      | Backup + sync to Backblaze B2 |
| `minio`      | `backup-minio`   | Backup + sync to bundled MinIO side-service |
| `codeberg`   | `backup-codeberg`| Backup + mirror push to Codeberg / Forgejo / Gitea |
| `gitlab`     | `backup-gitlab`  | Backup + mirror push to GitLab.com or self-hosted GitLab |

### Pre-flight check

Before the first real run, confirm your token, network, and host
filesystem are all good:

```sh
docker compose --profile doctor run --rm doctor octocat
```

Output is a colour-coded pass/fail report; exit code `0` means
everything checked out.

### Local filesystem backup

```sh
docker compose run --rm backup octocat --all
```

Files land in `./backups/`.

To use a TOML config file, mount it at runtime:

```sh
docker compose run --rm \
  -v "$PWD/config.toml:/etc/github-backup/config.toml:ro" \
  backup --config /etc/github-backup/config.toml
```

### Interactive TUI

```sh
docker compose --profile tui run --rm tui octocat
```

The Compose service already enables `tty: true` and `stdin_open: true`
so ratatui can render and read keystrokes.  Quit with `q` or `Ctrl+C`.

### Verify a previous backup

```sh
docker compose --profile verify run --rm verify octocat
```

Reads `./backups/<owner>/json/backup_manifest.json` and checks every
file's SHA-256.  Exits non-zero if anything is missing, tampered, or
unexpected.

### AWS S3 sync

```sh
docker compose --profile s3 run --rm backup-s3 octocat --all
```

### Backblaze B2

```sh
docker compose --profile b2 run --rm backup-b2 octocat --all
```

### Self-hosted MinIO

The `minio` profile starts a sidecar MinIO container automatically.

```sh
docker compose --profile minio up -d minio
docker compose --profile minio run --rm backup-minio octocat --all
```

MinIO console: <http://localhost:9001> (default `minioadmin` / `minioadmin`,
override in `.env`).

### Mirror to Codeberg / Forgejo / Gitea

```sh
docker compose --profile codeberg run --rm backup-codeberg octocat --all
```

### Mirror to GitLab

```sh
docker compose --profile gitlab run --rm backup-gitlab octocat --all
```

## Environment Variables

Every variable listed in `compose.example.env` is also accepted by a
plain `docker run`:

| Variable                    | Purpose |
|-----------------------------|---------|
| `GITHUB_TOKEN`              | GitHub personal access token (required) |
| `GITHUB_API_URL`            | Override base URL for GitHub Enterprise Server |
| `GITHUB_CLONE_HOST`         | Override the git clone hostname for split GHES setups |
| `GITHUB_OAUTH_CLIENT_ID`    | OAuth App client ID for `--device-auth` |
| `AWS_ACCESS_KEY_ID`         | AWS / B2 / MinIO access key |
| `AWS_SECRET_ACCESS_KEY`     | AWS / B2 / MinIO secret key |
| `MIRROR_TOKEN`              | Token for the mirror destination |
| `BACKUP_ENCRYPT_KEY`        | 32-byte hex key for AES-256-GCM at-rest encryption |
| `BACKUP_NOTIFY_WEBHOOK`     | URL receiving JSON success/failure POST |
| `GITHUB_BACKUP_RESTORE_YES` | `1` to authorise `--restore` non-interactively |
| `RUST_LOG`                  | Override the default `info` log level |
| `NO_COLOR`                  | Non-empty: disable ANSI colour codes |
| `CLICOLOR_FORCE`            | `1`: force ANSI colour even when not a TTY |
| `HTTPS_PROXY` / `https_proxy` | Route all GitHub traffic through this proxy |

## Image Security Notes

- **Non-root user**: the runtime image runs as `backup` (UID 1000, GID 1000).
  Bind-mounted host directories owned by the typical first user "just work".
- **No shell needed**: `apk` is removed from the runtime layer; `git`,
  `ca-certificates`, and `tini` are the only Alpine packages installed.
- **Tini as PID 1**: signals (`SIGTERM` from `docker stop`, Kubernetes pod
  eviction) propagate cleanly to the backup process, which writes a
  checkpoint and releases its lock before exiting with the conventional
  signal exit code (130 for SIGINT, 143 for SIGTERM).
- **Atomic outputs**: report JSON and Prometheus textfiles are written
  with a `*.tmp` + `rename` so a monitoring agent polling the mount
  never reads a half-written file.

## Scheduled Backups

### Plain cron

```sh
# /etc/cron.d/github-backup
0 2 * * * backup docker run --rm \
  -v /var/backup/github:/backup \
  -e GITHUB_TOKEN=ghp_xxx \
  ghcr.io/tomtom215/github-backup-rust:latest \
  octocat --output /backup --all \
  >> /var/log/github-backup.log 2>&1
```

### systemd timer

```ini
# /etc/systemd/system/github-backup.timer
[Unit]
Description=Daily GitHub backup

[Timer]
OnCalendar=*-*-* 02:00:00
Persistent=true
RandomizedDelaySec=15m

[Install]
WantedBy=timers.target
```

```ini
# /etc/systemd/system/github-backup.service
[Unit]
Description=GitHub backup
After=network-online.target docker.service
Requires=network-online.target

[Service]
Type=oneshot
EnvironmentFile=/etc/github-backup/env
ExecStart=/usr/bin/docker run --rm --pull always -v /var/backup/github:/backup -e GITHUB_TOKEN ghcr.io/tomtom215/github-backup-rust:latest octocat --output /backup --all
```

Use `EnvironmentFile=` so `GITHUB_TOKEN` lives in a `0600`-mode file
rather than the unit file itself.  systemd does not expand shell `\`
line continuations in `ExecStart=`, so keep that line whole.

### Kubernetes CronJob

```yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: github-backup
spec:
  schedule: "0 2 * * *"
  concurrencyPolicy: Forbid          # one backup at a time
  failedJobsHistoryLimit: 3
  successfulJobsHistoryLimit: 1
  jobTemplate:
    spec:
      template:
        spec:
          restartPolicy: OnFailure
          containers:
            - name: backup
              image: ghcr.io/tomtom215/github-backup-rust:0.3.2
              args:
                - "octocat"
                - "--output"
                - "/backup"
                - "--all"
              env:
                - name: GITHUB_TOKEN
                  valueFrom:
                    secretKeyRef:
                      name: github-backup-token
                      key: token
              volumeMounts:
                - name: backup
                  mountPath: /backup
              resources:
                requests:
                  cpu: "100m"
                  memory: "128Mi"
                limits:
                  cpu: "1"
                  memory: "512Mi"
          volumes:
            - name: backup
              persistentVolumeClaim:
                claimName: github-backup-pvc
```

## Troubleshooting

```sh
# Show what the doctor sees inside the container
docker compose --profile doctor run --rm doctor octocat

# Print the OAuth scopes the current flag set needs
docker run --rm \
  -e GITHUB_TOKEN=$GITHUB_TOKEN \
  ghcr.io/tomtom215/github-backup-rust:latest \
  octocat --org --all --list-scopes

# Validate a config file without running a backup
docker run --rm \
  -v "$PWD/config.toml:/etc/github-backup/config.toml:ro" \
  -e GITHUB_TOKEN=$GITHUB_TOKEN \
  ghcr.io/tomtom215/github-backup-rust:latest \
  --config /etc/github-backup/config.toml --check
```
