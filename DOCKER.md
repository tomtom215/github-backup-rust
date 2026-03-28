# Docker Guide

`github-backup-rust` ships a multi-stage `Dockerfile` that produces a minimal
Alpine-based image (~15 MB) with only `git`, CA certificates, and the backup
binary.

## Quick Start

```sh
# Build the image
docker build -t github-backup .

# Basic backup (local filesystem)
docker run --rm \
  -v /var/backup/github:/backup \
  -e GITHUB_TOKEN=ghp_xxxxxxxxxxxx \
  github-backup octocat --output /backup --all
```

## Docker Compose Profiles

The `docker-compose.yml` file provides pre-configured services for common
scenarios.  Activate a profile with `--profile <name>`.

### Local backup (default)

```sh
GITHUB_TOKEN=ghp_xxx docker compose run --rm backup octocat --all
```

Backup files are written to `./data/`.

### AWS S3 sync

```sh
export GITHUB_TOKEN=ghp_xxx
export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
export AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
export S3_BUCKET=my-github-backups
export S3_REGION=us-east-1

docker compose --profile s3 run --rm backup-s3 octocat --all
```

### Backblaze B2

```sh
export GITHUB_TOKEN=ghp_xxx
export B2_KEY_ID=your_b2_key_id
export B2_APP_KEY=your_b2_app_key
export B2_BUCKET=my-b2-bucket
export B2_REGION=us-west-004

docker compose --profile b2 run --rm backup-b2 octocat --all
```

### Self-hosted MinIO

Start MinIO and run the backup in one command:

```sh
export GITHUB_TOKEN=ghp_xxx
export MINIO_BUCKET=github-backup

docker compose --profile minio up -d minio
docker compose --profile minio run --rm backup-minio octocat --all
```

MinIO console is available at `http://localhost:9001` (admin/admin by default).

### Mirror to Codeberg

```sh
export GITHUB_TOKEN=ghp_xxx
export MIRROR_TOKEN=your_codeberg_token
export MIRROR_OWNER=your_codeberg_username

docker compose --profile codeberg run --rm backup-codeberg octocat \
  --repositories --all
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `GITHUB_TOKEN` | GitHub personal access token |
| `AWS_ACCESS_KEY_ID` | AWS / B2 / MinIO access key |
| `AWS_SECRET_ACCESS_KEY` | AWS / B2 / MinIO secret key |
| `MIRROR_TOKEN` | API token for the mirror destination |

## Building for Production

The multi-stage build produces a statically-linked musl binary:

```sh
docker build --target runtime -t github-backup:latest .
docker image ls github-backup
```

## Scheduled Backups with Cron

Run daily backups via cron or any scheduler:

```sh
# /etc/cron.d/github-backup
0 2 * * * backup docker run --rm \
  -v /var/backup/github:/backup \
  -e GITHUB_TOKEN=ghp_xxx \
  github-backup octocat --output /backup --all \
  >> /var/log/github-backup.log 2>&1
```

Or with systemd timers:

```ini
# /etc/systemd/system/github-backup.timer
[Unit]
Description=Daily GitHub backup

[Timer]
OnCalendar=*-*-* 02:00:00
Persistent=true

[Install]
WantedBy=timers.target
```

```ini
# /etc/systemd/system/github-backup.service
[Unit]
Description=GitHub backup

[Service]
Type=oneshot
ExecStart=docker run --rm \
  -v /var/backup/github:/backup \
  -e GITHUB_TOKEN=ghp_xxx \
  github-backup octocat --output /backup --all
```
