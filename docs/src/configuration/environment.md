# Environment Variables

`github-backup` reads several environment variables so that secrets stay out of the command line and shell history.

## Authentication

| Variable | Flag equivalent | Description |
|---------|----------------|-------------|
| `GITHUB_TOKEN` | `--token` | GitHub personal access token |
| `GITHUB_OAUTH_CLIENT_ID` | `--oauth-client-id` | OAuth App client ID (device flow) |

## S3 Storage

| Variable | Flag equivalent | Description |
|---------|----------------|-------------|
| `AWS_ACCESS_KEY_ID` | `--s3-access-key` | S3 access key ID |
| `AWS_SECRET_ACCESS_KEY` | `--s3-secret-key` | S3 secret access key |

## Mirror Push

| Variable | Flag equivalent | Description |
|---------|----------------|-------------|
| `MIRROR_TOKEN` | `--mirror-token` | API token for Gitea/Codeberg mirror destination |

## Logging

| Variable | Description |
|---------|-------------|
| `RUST_LOG` | Overrides the computed log level. Accepts [`tracing` filter directives](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html). |

Examples:
```bash
# Show only warnings and errors from all crates
RUST_LOG=warn github-backup ...

# Show debug from github-backup-client only
RUST_LOG=github_backup_client=debug github-backup ...

# Show trace for everything
RUST_LOG=trace github-backup ...
```

## Setting Variables Securely

For interactive use, use `read` to avoid token appearing in history:

```bash
read -rs GITHUB_TOKEN && export GITHUB_TOKEN
```

For Docker / Kubernetes, use secrets management:

```yaml
# Docker Compose
environment:
  GITHUB_TOKEN: "${GITHUB_TOKEN}"
  AWS_ACCESS_KEY_ID: "${AWS_ACCESS_KEY_ID}"
  AWS_SECRET_ACCESS_KEY: "${AWS_SECRET_ACCESS_KEY}"
```

For systemd services, use `EnvironmentFile`:

```ini
[Service]
EnvironmentFile=/etc/github-backup/secrets.env
```

Where `secrets.env` has `0600` permissions:

```
GITHUB_TOKEN=ghp_xxx
```
