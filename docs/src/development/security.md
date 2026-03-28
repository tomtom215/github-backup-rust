# Security

This page documents the security design of `github-backup`, the credential
handling model, and recommended practices for production deployments.

---

## Credential Handling

### Personal Access Tokens

Tokens are never logged, never embedded in git remote URLs, and never written
to disk outside of an explicit config file.

- **HTTPS clones**: the token is injected via a temporary `GIT_ASKPASS` shell
  script written to `$TMPDIR` with mode `0700`.  The script is automatically
  deleted by a RAII guard (`AskpassScript`) even if the process panics or is
  killed with `SIGTERM`.
- **Authorization headers**: every GitHub API request uses
  `Authorization: Bearer <token>` — never as a URL parameter.

### Environment Variables

The recommended way to supply the token is via the `GITHUB_TOKEN` environment
variable:

```bash
export GITHUB_TOKEN=ghp_your_token_here
github-backup octocat --output /backup --all
```

This keeps the token out of shell history and process listings.

### Config Files

The `token` key in a TOML config file is supported but less secure than
environment variables.  If you must store a token in a config file:

- Restrict the file to the backup user: `chmod 0600 /etc/github-backup/config.toml`
- Use a fine-grained token with the minimum required scopes (see below).

---

## Minimum Token Scopes

### Classic Tokens

| Backup category | Required scope |
|----------------|----------------|
| Public repos | *(none required)* |
| Private repos | `repo` |
| Gists | `gist` |
| Org repos | `read:org` |
| Webhooks | `admin:repo_hook` (or repo admin) |

A token with `repo gist read:org` covers all categories except webhooks.

### Fine-Grained Tokens (Recommended)

Fine-grained tokens are scoped to specific repositories and expire
automatically.  Recommended permissions:

| Permission | Level |
|-----------|-------|
| Contents | Read-only |
| Issues | Read-only |
| Pull requests | Read-only |
| Metadata | Read-only |
| Webhooks | Read-only (optional) |

Fine-grained tokens cannot access organisation data; use a classic token with
`read:org` for organisation backups.

---

## Network Security

- **TLS only**: `github-backup` uses `hyper-rustls` and refuses plain-HTTP
  connections.  The `hyper` connector is built with `.https_only()`.
- **System CA bundle**: TLS verification uses the platform's native CA
  certificate store (`rustls-native-certs`).  No bundled CAs.
- **No OpenSSL**: the dependency policy (`deny.toml`) bans `openssl` and
  `native-tls`.  The entire TLS stack is pure Rust (`rustls`).

---

## Dependency Policy

`cargo-deny` enforces the following at CI time:

| Policy | Rule |
|--------|------|
| Banned crates | `openssl`, `openssl-sys`, `reqwest`, `native-tls` |
| Allowed licenses | MIT, Apache-2.0, ISC, BSD-3-Clause, Unicode-3.0, CC0-1.0 |
| Security advisories | `cargo audit` blocks any known vulnerability |

---

## S3 Credential Security

S3 credentials are accepted via `--s3-access-key` / `--s3-secret-key` or
via the `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` environment variables.

- Prefer environment variables in automated environments.
- Use an IAM policy with the minimum required permissions:
  `s3:PutObject`, `s3:GetObject`, `s3:HeadObject` on the target bucket only.

---

## Output Directory Permissions

Backup artefacts may include sensitive data (webhook secrets, security
advisories, private repository code).  Restrict the output directory:

```bash
mkdir -p /var/backup/github
chown backup-user:backup-group /var/backup/github
chmod 700 /var/backup/github
```

For multi-user systems, consider encrypting the backup directory with LUKS or
a similar technology.

---

## Unsafe Code Policy

`github-backup` enforces `#![deny(unsafe_op_in_unsafe_fn)]` workspace-wide.
There is **zero unsafe code** in the workspace.

---

## Reporting Security Vulnerabilities

Please do **not** open a public GitHub issue for security vulnerabilities.
Instead, use GitHub's private security advisory feature:

1. Navigate to
   [github.com/tomtom215/github-backup-rust/security/advisories/new](https://github.com/tomtom215/github-backup-rust/security/advisories/new).
2. Describe the vulnerability and steps to reproduce.
3. We will acknowledge within 48 hours and aim to release a patch within 14 days.
