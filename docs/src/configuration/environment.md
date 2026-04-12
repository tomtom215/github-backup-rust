# Environment Variables

`github-backup` reads several environment variables so that secrets stay out of the command line and shell history.

## Authentication

| Variable | Flag equivalent | Description |
|---------|----------------|-------------|
| `GITHUB_TOKEN` | `--token` | GitHub personal access token |
| `GITHUB_OAUTH_CLIENT_ID` | `--oauth-client-id` | OAuth App client ID (device flow) |

## GitHub Enterprise

| Variable | Flag equivalent | Description |
|---------|----------------|-------------|
| `GITHUB_API_URL` | `--api-url` | Override the GitHub API base URL (e.g. `https://github.example.com/api/v3`) |
| `GITHUB_CLONE_HOST` | `--clone-host` | Override the hostname used in git clone URLs |

## S3 Storage and Encryption

| Variable | Flag equivalent | Description |
|---------|----------------|-------------|
| `AWS_ACCESS_KEY_ID` | `--s3-access-key` | S3 access key ID |
| `AWS_SECRET_ACCESS_KEY` | `--s3-secret-key` | S3 secret access key |
| `BACKUP_ENCRYPT_KEY` | `--encrypt-key` | 64-hex-char AES-256-GCM key for at-rest encryption |

## Mirror Push

| Variable | Flag equivalent | Description |
|---------|----------------|-------------|
| `MIRROR_TOKEN` | `--mirror-token` | API token for the Gitea / GitLab mirror destination |

## Notification

| Variable | Flag equivalent | Description |
|---------|----------------|-------------|
| `BACKUP_NOTIFY_WEBHOOK` | `--notify-webhook` | URL that receives a JSON status payload after the backup |

## Proxy

| Variable | Description |
|---------|-------------|
| `HTTPS_PROXY` | Route HTTPS API calls through this proxy (e.g. `http://user:pass@proxy.example.com:3128`). Also read as `https_proxy` (lowercase). |

`github-backup` reads `HTTPS_PROXY` (or `https_proxy`) at startup and routes all GitHub API calls through the configured proxy using HTTP `CONNECT` tunnelling.  Credentials embedded in the URL (`http://user:pass@host:port`) are forwarded as a `Proxy-Authorization` header.

> **Note:** `NO_PROXY` is not honoured by the API client; the proxy, when set, is used for every API request.  `git clone` and `git push` are performed by the system `git` binary, which honours its own proxy environment variables independently.

```bash
export HTTPS_PROXY=http://proxy.corp.example.com:3128
github-backup octocat --output /backup --all
```

With authentication:
```bash
export HTTPS_PROXY=http://proxyuser:secret@proxy.corp.example.com:3128
github-backup octocat --output /backup --all
```

---

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
