# Docker

`github-backup` ships a multi-stage Alpine Docker image of approximately 15 MB.

## Quick Start

```bash
docker run --rm \
  -e GITHUB_TOKEN=ghp_xxx \
  -v /var/backup/github:/backup \
  ghcr.io/tomtom215/github-backup-rust:latest \
  octocat --output /backup --all
```

## Building the Image

```bash
git clone https://github.com/tomtom215/github-backup-rust
cd github-backup-rust
docker build -t github-backup .
```

## Docker Compose

### Basic backup

```yaml
# docker-compose.yml
services:
  backup:
    image: ghcr.io/tomtom215/github-backup-rust:latest
    environment:
      GITHUB_TOKEN: "${GITHUB_TOKEN}"
    command: >
      octocat
      --output /backup
      --repositories
      --issues
      --pulls
      --releases
      --concurrency 8
    volumes:
      - ./backup:/backup
```

### With Codeberg mirror

```yaml
services:
  backup:
    image: ghcr.io/tomtom215/github-backup-rust:latest
    environment:
      GITHUB_TOKEN: "${GITHUB_TOKEN}"
      MIRROR_TOKEN: "${CODEBERG_TOKEN}"
    command: >
      octocat
      --output /backup
      --repositories
      --mirror-to https://codeberg.org
      --mirror-owner your_codeberg_username
    volumes:
      - ./backup:/backup
```

### With S3 sync (AWS)

```yaml
services:
  backup:
    image: ghcr.io/tomtom215/github-backup-rust:latest
    environment:
      GITHUB_TOKEN: "${GITHUB_TOKEN}"
      AWS_ACCESS_KEY_ID: "${AWS_ACCESS_KEY_ID}"
      AWS_SECRET_ACCESS_KEY: "${AWS_SECRET_ACCESS_KEY}"
    command: >
      octocat
      --output /backup
      --all
      --s3-bucket my-backup-bucket
      --s3-region us-east-1
    volumes:
      - ./backup:/backup
```

### With Backblaze B2

```yaml
services:
  backup:
    image: ghcr.io/tomtom215/github-backup-rust:latest
    environment:
      GITHUB_TOKEN: "${GITHUB_TOKEN}"
      AWS_ACCESS_KEY_ID: "${B2_KEY_ID}"
      AWS_SECRET_ACCESS_KEY: "${B2_APP_KEY}"
    command: >
      octocat
      --output /backup
      --all
      --s3-bucket my-b2-bucket
      --s3-region us-west-004
      --s3-endpoint https://s3.us-west-004.backblazeb2.com
    volumes:
      - ./backup:/backup
```

### With MinIO (self-hosted S3)

```yaml
services:
  minio:
    image: minio/minio:latest
    command: server /data --console-address ":9001"
    environment:
      MINIO_ROOT_USER: minioadmin
      MINIO_ROOT_PASSWORD: minioadmin
    ports:
      - "9000:9000"
      - "9001:9001"
    volumes:
      - minio_data:/data

  backup:
    image: ghcr.io/tomtom215/github-backup-rust:latest
    depends_on:
      - minio
    environment:
      GITHUB_TOKEN: "${GITHUB_TOKEN}"
      AWS_ACCESS_KEY_ID: minioadmin
      AWS_SECRET_ACCESS_KEY: minioadmin
    command: >
      octocat
      --output /backup
      --all
      --s3-bucket github-backup
      --s3-region us-east-1
      --s3-endpoint http://minio:9000
    volumes:
      - ./backup:/backup

volumes:
  minio_data:
```

## Scheduled Backups

### Using a cron container

```yaml
services:
  backup-cron:
    image: ghcr.io/tomtom215/github-backup-rust:latest
    environment:
      GITHUB_TOKEN: "${GITHUB_TOKEN}"
    # Use a shell wrapper to run on schedule
    entrypoint: /bin/sh
    command: |
      -c "while true; do
        github-backup octocat --output /backup --all
        sleep 86400
      done"
    volumes:
      - ./backup:/backup
```

### Using Ofelia (Docker cron scheduler)

```yaml
services:
  ofelia:
    image: mcuadros/ofelia:latest
    depends_on:
      - backup
    command: daemon --docker
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro

  backup:
    image: ghcr.io/tomtom215/github-backup-rust:latest
    labels:
      ofelia.enabled: "true"
      ofelia.job-exec.backup.schedule: "@daily"
      ofelia.job-exec.backup.command: >
        github-backup octocat --output /backup --all
    environment:
      GITHUB_TOKEN: "${GITHUB_TOKEN}"
    volumes:
      - ./backup:/backup
```

## Using a Config File in Docker

Mount a config file instead of long command lines:

```yaml
services:
  backup:
    image: ghcr.io/tomtom215/github-backup-rust:latest
    environment:
      GITHUB_TOKEN: "${GITHUB_TOKEN}"
    command: --config /config/backup.toml
    volumes:
      - ./backup.toml:/config/backup.toml:ro
      - ./backup:/backup
```

`backup.toml`:
```toml
owner = "octocat"
output = "/backup"
concurrency = 8
repositories = true
issues = true
pulls = true
releases = true
wikis = true
```

## Image Security Notes

- The image runs as a non-root user (`nobody`) by default.
- No shell is present in the final image (scratch + musl libc).
- Secrets should be provided via environment variables, not baked into the image.
