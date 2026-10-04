# Environment Variables

`github-backup` reads several environment variables so that secrets stay out
of the command line, shell history and the process list.  A variable that is
**set but empty** is treated as unset (container launchers often pass every
optional variable as an empty string); surrounding whitespace, such as a
newline pasted with a token, is trimmed.

## Authentication

| Variable | Flag equivalent | Description |
|---------|----------------|-------------|
| `GITHUB_TOKEN` | `--token` | GitHub personal access token |
| `GITHUB_OAUTH_CLIENT_ID` | `--oauth-client-id` | OAuth App client ID; ignored unless `--device-auth` is given |

## GitHub Enterprise

| Variable | Flag equivalent | Description |
|---------|----------------|-------------|
| `GITHUB_API_URL` | `--api-url` | API base URL (for example `https://github.example.com/api/v3`) |
| `GITHUB_CLONE_HOST` | `--clone-host` | Hostname used in git clone URLs |

## S3 Storage and Encryption

| Variable | Flag equivalent | Description |
|---------|----------------|-------------|
| `AWS_ACCESS_KEY_ID` | `--s3-access-key` | S3 access key ID |
| `AWS_SECRET_ACCESS_KEY` | `--s3-secret-key` | S3 secret access key |
| `AWS_SESSION_TOKEN` | `--s3-session-token` | Session token for temporary credentials |
| `BACKUP_ENCRYPT_KEY` | `--encrypt-key` | 64 hexadecimal characters: AES-256-GCM key for the S3 upload (and for `--decrypt`) |

The three `AWS_*` variables are ignored unless `--s3-bucket` is given, so
credentials exported for other tools never affect a run that does not use S3.
`AWS_PROFILE`, `~/.aws/credentials` and instance metadata are not read.

## Mirror Push

| Variable | Flag equivalent | Description |
|---------|----------------|-------------|
| `MIRROR_TOKEN` | `--mirror-token` | API token of the Gitea / Codeberg / Forgejo / GitLab destination; ignored unless `--mirror-to` is given |

## Notification and Restore

| Variable | Flag equivalent | Description |
|---------|----------------|-------------|
| `BACKUP_NOTIFY_WEBHOOK` | `--notify-webhook` | URL that receives the JSON status after a run |
| `GITHUB_BACKUP_RESTORE_YES` | `--restore-yes` | The value `1` confirms `--restore` without the interactive prompt |

## Proxy

| Variable | Description |
|---------|-------------|
| `HTTPS_PROXY` / `https_proxy` | Proxy for `https://` targets |
| `HTTP_PROXY` / `http_proxy` | Proxy for `http://` targets |
| `ALL_PROXY` / `all_proxy` | Fallback for both when the specific variable is unset |
| `NO_PROXY` / `no_proxy` | Comma-separated exceptions: `*`, host names (also match subdomains), IP addresses, CIDR blocks such as `10.0.0.0/8`, each with an optional `:port` |

The lower-case spelling wins when both are set.  The proxy must be an
**HTTP proxy** (`http://[user:pass@]host[:port]`, port 3128 if omitted;
credentials are sent as `Proxy-Authorization: Basic`).  `https://` targets are
reached through an HTTP `CONNECT` tunnel.  A value that is not a usable HTTP
proxy URL (`socks5://...` for example) is ignored with a warning.  **SOCKS
proxies are not supported.**

What the variables apply to:

| Traffic | Uses the proxy settings? |
|---------|--------------------------|
| GitHub API calls, including release-asset downloads | yes |
| `--notify-webhook` | yes |
| `--doctor` / `--check` | yes (the same client as the backup) |
| `git clone`, `git fetch`, `git push` (the `git` binary) | by git's own rules: git reads the same variables (`https_proxy`, `no_proxy`, ...) or its `http.proxy` setting |
| S3 requests | **no**, they connect to the endpoint directly |
| Gitea / GitLab API calls of `--mirror-to` | **no**, they connect directly (the `git push` itself follows git's rules) |

```bash
export HTTPS_PROXY=http://proxy.corp.example.com:3128
export NO_PROXY=localhost,.corp.example.com
github-backup octocat --output /backup --all
```

## TLS

| Variable | Description |
|---------|-------------|
| `SSL_CERT_FILE`, `SSL_CERT_DIR` | Extra or alternative trust store for the tool's own TLS clients (GitHub API, S3, webhook, mirror API).  Without them the operating system's certificate store is used. |
| `GIT_SSL_CAINFO` | The same for the `git` subprocesses (git has its own setting) |

## Logging

| Variable | Description |
|---------|-------------|
| `RUST_LOG` | A [`tracing` filter](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html).  When set it replaces the level chosen by `-q`, `-v` and `-vv`. |
| `NO_COLOR` | Set and non-empty: no ANSI colour. |
| `CLICOLOR_FORCE` | `1`: colour even when stderr is not a terminal. |

Logs go to **stderr**; the banners, `--doctor`, `--list-scopes` and
`--print-config-template` print to **stdout**.

```bash
# Warnings and errors only
RUST_LOG=warn github-backup ...

# Debug output of the API client only
RUST_LOG=github_backup_client=debug github-backup ...
```

## Container Variables

The image's entrypoint additionally reads `GITHUB_OWNER`, `BACKUP_MODE`,
`BACKUP_FLAGS` and `UMASK` when it is started without arguments; see
[Docker](../docker.md).  They are not read by the `github-backup` binary.

## Setting Variables Securely

For interactive use, `read` keeps the token out of the shell history:

```bash
read -rs GITHUB_TOKEN && export GITHUB_TOKEN
```

For systemd services use `EnvironmentFile=` with a `0600` file; for Docker use
`--env-file` or `-e GITHUB_TOKEN` (no value, so the secret is not in the
`docker` command line); for Kubernetes use a `Secret`.

```ini
[Service]
EnvironmentFile=/etc/github-backup/secrets.env
```

```
# /etc/github-backup/secrets.env (mode 0600)
GITHUB_TOKEN=ghp_dummy_token_for_illustration
```
