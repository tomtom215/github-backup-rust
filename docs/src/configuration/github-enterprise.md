# GitHub Enterprise Server (GHES)

`github-backup` can back up a GitHub Enterprise Server instance by overriding
the API base URL and, when it differs, the clone hostname.

> **Verification status.** The project's tests run against mock clients and
> local fake servers.  No real GHES instance was available, so GHES-specific
> behaviour (older API versions, missing endpoints) is not verified.  An
> endpoint that a GHES version lacks answers 404 and is skipped or reported as
> a failure like any other missing resource.

---

## Configuration

Set `--api-url` (or `GITHUB_API_URL`, or `api_url` in the config file) to your
instance's REST API root.  It must be an `https://` URL:

```bash
github-backup myorg \
  --output /backup \
  --api-url https://github.example.com/api/v3 \
  --org --all
```

(Pass the token in `GITHUB_TOKEN`.)  Every API request is made relative to this
base, and git clones use the `clone_url` the API returns.

### Config File

```toml
# config.toml  (token comes from GITHUB_TOKEN)
owner = "my-org"
org = true
output = "/backup"
api_url = "https://github.example.com/api/v3"
all = true
```

---

## TLS and Private Certificate Authorities

If the instance uses an internal CA, make both the tool and `git` trust it.

System-wide (works for both):

```bash
# Debian / Ubuntu
cp my-ca.crt /usr/local/share/ca-certificates/ && update-ca-certificates
# RHEL / Fedora
cp my-ca.crt /etc/pki/ca-trust/source/anchors/ && update-ca-trust
```

Or per process: `SSL_CERT_FILE=/path/to/ca.pem` for `github-backup` and
`GIT_SSL_CAINFO=/path/to/ca.pem` for the `git` it starts.  The tool uses
`rustls` with the operating system's certificate store; it has no flag to
disable certificate verification.

---

## Clone URLs

Clone URLs come from the API response (`clone_url`, or `ssh_url` with
`--prefer-ssh`).  If your instance advertises a hostname that is not reachable
from the backup host, or the git endpoint is behind a different load balancer
than the API, replace the hostname:

```bash
github-backup myorg \
  --output /backup \
  --api-url https://github-api.example.com/api/v3 \
  --clone-host github-git.example.com \
  --org --repositories
```

---

## Authentication

GHES accepts the same personal access tokens as github.com; create one at
`https://<your-ghes-host>/settings/tokens`.  The scopes per category are in
[Authentication](../getting-started/authentication.md#what-each-category-needs).
For organisation backups the token owner must be able to see the repositories
(organisation owner, or explicit access).

---

## GitHub Enterprise Cloud (GHEC)

GHEC uses `https://api.github.com`, like github.com: no `--api-url` is needed.
If the organisation enforces SAML single sign-on, authorise the token for it
in GitHub's token settings, otherwise the API answers 403.

---

## Proxies

`HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY` and `NO_PROXY` apply to the API calls;
`git` follows its own proxy settings.  S3 and mirror-destination API calls do
not use a proxy.  Details and the SOCKS limitation are in
[Environment Variables](environment.md#proxy).

---

## Rate Limits

GHES rate limits are configured by the site administrator.  The tool waits out
`429` responses and `403` responses that carry rate-limit information
(`Retry-After`, `X-RateLimit-Remaining: 0` or a "rate limit" message) and
retries; see [Troubleshooting](../development/troubleshooting.md#rate-limit-errors).
To reduce the pressure, lower the concurrency:

```bash
github-backup myorg --output /backup --org --all --concurrency 2
```
