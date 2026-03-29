# Troubleshooting

This page covers the most common problems encountered when running
`github-backup` and how to resolve them.

---

## Authentication Errors

### `401 Unauthorized`

Your token is missing or invalid.

- Verify the token with `curl -H "Authorization: Bearer $GITHUB_TOKEN" https://api.github.com/user`.
- Ensure `GITHUB_TOKEN` is exported in the environment where `github-backup` runs.
- Classic tokens: ensure the `repo` and `gist` scopes are enabled.
- Fine-grained tokens: ensure the token has read access to the repositories you want to back up.

### `403 Forbidden` on hooks or security advisories

Backing up webhooks (`--hooks`) requires admin permission on the repository.
`github-backup` automatically skips these with an `INFO`-level log message when
access is denied — this is expected and not an error.

### OAuth Device Flow timeout

If you start `--device-auth` but don't enter the code in time, the flow will
time out with a `slow_down` or `access_denied` error. Simply re-run the
command.

---

## Rate Limit Errors

### `rate limit hit, sleeping`

`github-backup` automatically waits until the rate-limit window resets and
retries.  For large accounts this can mean waiting up to 60 minutes.

To reduce rate-limit pressure:

1. Lower concurrency: `--concurrency 1`.
2. Use `--since <DATETIME>` to only fetch recently-updated issues and PRs.
3. Break up the backup into smaller runs (e.g. `--include-repos "a*"` then `--include-repos "b*"`).

### `RateLimitExceeded` error after 3 retries

GitHub's secondary rate limits apply to certain write-heavy operations.
Reduce `--concurrency` and re-run.

---

## Network and TLS Errors

### `Tls: no CA certificates found`

On minimal Linux installations (Alpine, distroless) the system CA bundle may
be absent.

- Install `ca-certificates`: `apk add ca-certificates` / `apt-get install ca-certificates`.
- When using Docker, use the provided `Dockerfile` which already includes `ca-certificates`.

### `Connect` or `Connection refused` errors behind a proxy

If `github-backup` cannot reach `api.github.com` in a network where outbound HTTPS is only permitted through a proxy:

```
ERROR backup failed: GitHub API error: HTTP transport error: client error (Connect)
```

Set `HTTPS_PROXY`:

```bash
export HTTPS_PROXY=http://proxy.example.com:3128
github-backup octocat --output /backup --all
```

With credentials:

```bash
export HTTPS_PROXY=http://user:secret@proxy.example.com:3128
github-backup octocat --output /backup --all
```

At startup you will see:
```
INFO  routing GitHub API calls through HTTPS proxy proxy=http://proxy.example.com:3128
```

> **git clone vs API calls**: `github-backup` routes API calls through the proxy automatically. For git clone operations the system `git` binary reads `HTTPS_PROXY` / `GIT_PROXY_COMMAND` independently — set those environment variables too if git clones are also failing.

### `Timeout` errors

GitHub can be slow for very large repositories or under high load.  The
default request timeout is 120 seconds.  There is currently no CLI flag to
increase it, but you can increase it by setting `RUST_LOG=debug` and checking
whether requests are consistently timing out on the same endpoint.

---

## Git Errors

### `git clone` fails with `fatal: repository not found`

- Confirm the repository is accessible with your token.
- For private repositories, ensure `--private` is set and your token has the `repo` scope.
- For SSH clones (`--prefer-ssh`), ensure your SSH key is available in the environment.

### `git remote update` fails with exit code ≠ 0

`github-backup` logs the error and continues with the next repository.
Check for:

- Network interruptions during large repository fetches.
- Repositories that have been deleted or transferred between backup runs.

---

## Storage Errors

### `cannot create directory: Permission denied`

Ensure the backup output directory (`--output`) exists and is writable by the
user running `github-backup`:

```bash
mkdir -p /var/backup/github
chown backup-user:backup-group /var/backup/github
chmod 750 /var/backup/github
```

### Disk full during backup

`github-backup` does not pre-check available disk space.  If the disk fills
up mid-run you will see write errors.  Per-repository errors are non-fatal —
the run continues and errored repos are counted in the summary report.

Monitor disk usage with a pre-backup check:

```bash
REQUIRED_GB=50
AVAIL_GB=$(df --output=avail -BG /var/backup/github | tail -1 | tr -d 'G ')
if [ "$AVAIL_GB" -lt "$REQUIRED_GB" ]; then
  echo "Not enough disk space" >&2
  exit 1
fi
```

---

## S3 Sync Issues

### `403 Forbidden` from S3

- Verify the access key and secret key.
- Ensure the IAM policy allows `s3:PutObject`, `s3:GetObject` (for HEAD checks), and `s3:ListBucket`.
- For non-AWS providers (B2, R2, MinIO) verify the `--s3-endpoint` URL.

### Objects not updating

`github-backup` performs a `HeadObject` check before uploading.  If the
remote object's ETag matches the local SHA-256, the upload is skipped.  This
is the intended incremental behaviour.  To force a full re-upload, delete the
objects in the bucket first.

---

## Mirroring Issues

### `401 Unauthorized` on Gitea/Codeberg push

- Verify the mirror token (`--mirror-token` / `MIRROR_TOKEN`).
- Ensure the token has repository-creation permissions at the destination.

### Mirror push creates duplicate repositories

`github-backup` attempts to create the repository before pushing.  If the
repository already exists, it uses the existing one.  A `409 Conflict` from
the Gitea API is silently ignored.

---

## Enabling Debug Logging

For any issue not listed here, enable detailed logs:

```bash
RUST_LOG=debug github-backup octocat --token "$GITHUB_TOKEN" --output /backup --all 2>&1 | tee /tmp/debug.log
```

Or increase verbosity via flags:

```bash
github-backup octocat -vv --token "$GITHUB_TOKEN" --output /backup --all
```

`-v` = `debug`, `-vv` = `trace` (very verbose; includes every HTTP request
and response header).

---

## Reporting Bugs

Please open an issue at
[github.com/tomtom215/github-backup-rust/issues](https://github.com/tomtom215/github-backup-rust/issues)
and include:

1. The command you ran (redact tokens).
2. The relevant log output (with `RUST_LOG=debug`).
3. Your OS and Rust version (`rustc --version`).
4. The `github-backup --version` output.
