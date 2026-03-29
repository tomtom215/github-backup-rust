# GitHub Enterprise Server (GHES)

`github-backup` supports GitHub Enterprise Server by overriding the API base URL and the clone hostname.

---

## Configuration

Set the `--api-url` flag (or `api_url` in `config.toml`) to your GHES REST API endpoint:

```bash
github-backup octocat \
  --token $GITHUB_TOKEN \
  --output /backup \
  --api-url https://github.example.com/api/v3 \
  --all
```

The tool constructs all API requests relative to this base URL, so the standard `https://api.github.com` root is replaced throughout.

### Config File

```toml
# config.toml
token = "ghp_xxxxxxxxxxxx"
output = "/backup"
api_url = "https://github.example.com/api/v3"
all = true
```

---

## TLS / Self-Signed Certificates

If your GHES instance uses a self-signed or internal CA certificate, add the CA bundle to the system trust store before running `github-backup`.

On Debian/Ubuntu:

```bash
cp my-ca.crt /usr/local/share/ca-certificates/
update-ca-certificates
```

On RHEL/Fedora:

```bash
cp my-ca.crt /etc/pki/ca-trust/source/anchors/
update-ca-trust
```

`github-backup` uses the system certificate store via `rustls-native-certs`; no additional flags are needed once the CA is trusted system-wide.

---

## Clone URLs

By default, repository clone URLs are taken directly from the API response (`clone_url` and `ssh_url` fields).  For GHES these already point to your instance hostname, so no extra configuration is needed.

If your GHES clone hostname differs from the API hostname (for example, when using a separate load balancer), use `--clone-host` to override:

```bash
github-backup octocat \
  --token $GITHUB_TOKEN \
  --output /backup \
  --api-url https://github-api.example.com/api/v3 \
  --clone-host github-git.example.com \
  --repositories
```

---

## Authentication

GHES supports the same personal access token flow as GitHub.com.  Create a token at:

```
https://<your-ghes-host>/settings/tokens
```

For organisation backups, the token must belong to an organisation owner or must have explicit repository access granted.

### Required Scopes

| Category | Required scope |
|----------|---------------|
| Public repos | `public_repo` |
| Private repos | `repo` |
| Hooks / deploy keys / collaborators | `repo` (admin access) |
| Gists | `gist` |
| Org members / teams | `read:org` |

---

## GitHub Enterprise Cloud (GHEC)

GitHub Enterprise Cloud uses `https://api.github.com` — the same endpoint as GitHub.com.  No `--api-url` override is needed.  Token scopes and the `--org` flag work identically to GitHub.com.

---

## Proxy Support

If your GHES instance (or GitHub.com) is reached through a corporate HTTP proxy, set `HTTPS_PROXY` before running the tool:

```bash
export HTTPS_PROXY=http://proxy.example.com:3128
github-backup octocat --token $GITHUB_TOKEN --output /backup --all
```

`github-backup` reads `HTTPS_PROXY` (or the lowercase `https_proxy`) at startup and routes all GitHub API calls through the proxy via HTTP `CONNECT` tunnelling.  Credentials embedded in the URL (`http://user:pass@host:port`) are forwarded automatically as a `Proxy-Authorization` header.

> **Note**: git clone operations are performed by the system `git` binary, which honours `HTTPS_PROXY` / `GIT_PROXY_COMMAND` from the environment separately.  Set them together for consistent behaviour.

See [Environment Variables → Proxy](environment.md#proxy) for the full variable reference.

---

## Rate Limits

GHES rate limits are configurable by your site administrator and may differ from GitHub.com defaults.  If you encounter `429 Too Many Requests` responses, reduce concurrency:

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup --all --concurrency 2
```

Or in `config.toml`:

```toml
concurrency = 2
```
