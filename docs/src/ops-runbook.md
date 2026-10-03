# Operations Runbook

This runbook covers day-to-day operational procedures for a production
`github-backup` deployment: health checks, failure response, retention, key
rotation, and common troubleshooting steps.

---

## Daily Health Check

After each backup run, confirm the following:

1. **Exit code** — the process exits `0` on success.  Exit code `1` indicates
   a fatal error; exit code `130` means the backup was interrupted by SIGINT.

2. **Log summary** — look for the `backup complete` log line:
   ```
   INFO backup complete repos_backed_up=42 repos_skipped=0 repos_errored=0 ...
   ```
   A non-zero `repos_errored` warrants investigation.

3. **JSON report** (if `--report` is configured):
   ```bash
   jq '.repos_backed_up, .repos_errored' /var/backup/github/report.json
   ```

4. **Prometheus metrics** (if `--prometheus-metrics` is configured):
   ```bash
   grep 'github_backup_success' /var/lib/prometheus/github_backup.prom
   # Should be: github_backup_success{owner="..."} 1
   ```

5. **S3 sync** (if configured):
   ```
   INFO S3 sync complete uploaded=N skipped=M errored=0 deleted=0
   ```
   If any upload or deletion failed the run exits non-zero and the error
   lists the failed keys with the S3 status, code and a hint.

---

## Backup Interrupted (SIGINT / Exit 130)

If the backup process received SIGINT (e.g. the timer was stopped):

1. Check for partial JSON files in `<output>/<owner>/json/repos/`.  These are
   overwritten on the next successful run.
2. Temporary `GIT_ASKPASS` scripts in `$TMPDIR` are cleaned up by RAII guards
   at process exit; check `/tmp/github-backup-askpass-*` if the process was
   killed with SIGKILL instead.
3. Re-run the backup — it resumes from the beginning (incremental git fetches
   avoid re-downloading all history).

---

## Investigating Backup Failures

### Single repository error

If `repos_errored > 0`, the error is logged at the `WARN` or `ERROR` level
with the repository name:

```
WARN backup_one_repo error="..." owner="octocat" repo="my-repo"
```

Common causes:
- **Rate limit** — the client retries automatically; a persistent failure may
  indicate an unusually large repository or a token with insufficient quota.
- **Token scope** — ensure the token has `repo` scope for private repositories.
- **Git clone failure** — check network connectivity and that `git` is on `$PATH`.

### GitHub API rate limit

The client backs off automatically when rate-limited.  To check the current
limit:

```bash
curl -s -H "Authorization: Bearer $GITHUB_TOKEN" \
  https://api.github.com/rate_limit | jq '.rate'
```

Increase the token quota by using a dedicated service-account token, or
schedule the backup in an off-peak window.

### S3 upload failures

Look for `S3 ... failed with HTTP <status> <Code>` in the log; it ends with a
hint. Common causes:
- `SignatureDoesNotMatch` / `InvalidAccessKeyId`: wrong or expired credentials,
  or an endpoint that belongs to another provider
- `NoSuchBucket`: the bucket does not exist (it is never created) or the
  endpoint/region is wrong
- `AccessDenied`: the policy lacks `s3:ListBucket` / `s3:GetObject` /
  `s3:PutObject` (see the [S3 guide](storage/s3.md#required-permissions))
- `RequestTimeTooSkewed`: fix the system clock
- "cannot reach the endpoint": network, DNS, or a proxy (S3 ignores `HTTPS_PROXY`)

Use `--dry-run` to test access without writing anything.

## Retention Management

Limit disk growth using the retention flags:

```bash
# Keep only the 7 most recent dated snapshot directories
github-backup octocat --output /var/backup/github --keep-last 7

# Delete snapshots older than 30 days
github-backup octocat --output /var/backup/github --max-age-days 30

# Combine both: keep at least 3 and delete anything older than 14 days
github-backup octocat --output /var/backup/github --keep-last 3 --max-age-days 14
```

Snapshots are directories matching `YYYY-MM-DD*` directly under `--output`.
Non-snapshot directories (e.g. `config`, `keys`) are never touched.

### S3 stale object cleanup

When backups change (repositories archived/deleted), enable stale deletion to
keep S3 in sync with local state:

```bash
github-backup octocat \
  --s3-bucket my-backups \
  --s3-delete-stale \
  ...
```

Only objects under `<prefix>/<owner>/json/` are considered, and nothing is
deleted when the backup run had failures, the local tree could not be fully
read or is empty, or an upload failed. Try it with `--dry-run` first; it lists
what would be removed. Deletion is permanent unless bucket versioning is on.

---

## Encryption Key Rotation

Changing the key makes the next run upload every file again under the new
key (the stored content digest is keyed). Objects whose local file is gone stay
under the old key. The full procedure, including what to keep and when to
retire the old key, is in the [encryption guide](storage/encryption.md#rotating-the-key).

Short form:

1. Generate a new key: `openssl rand -hex 32`.
2. Run a backup with `BACKUP_ENCRYPT_KEY=<new key>`.
3. Decrypt one object with the new key to verify it.
4. Optionally run once with `--s3-delete-stale` to drop objects still under the
   old key, then retire the old key.

Do not run `aws s3 rm ... --include "*.enc"`: it deletes every encrypted
object, new ones included.

---

## Verifying Backup Integrity

If `--manifest` was used during the backup, verify integrity at any time:

```bash
github-backup octocat \
  --output /var/backup/github \
  --verify
```

This checks the SHA-256 digest of every JSON file against
`json/backup_manifest.json`.  Exits non-zero if any file is missing, changed,
or unexpected.

---

## Restoring After Disaster

See the [Restore Guide](restore.md) for full procedures.  Quick reference:

```bash
# 1. Restore git data to a new org
git -C /backup/octocat/git/repos/my-repo.git push --mirror \
    https://github.com/new-org/my-repo.git

# 2. Restore labels, milestones, and issues
github-backup octocat \
  --token ghp_write_token \
  --output /var/backup/github \
  --restore \
  --restore-target-org new-org \
  --restore-yes
```

---

## Upgrade Procedure

1. Stop the scheduled backup (systemd timer or cron job).
2. Download the new binary and replace the old one.
3. Verify the version: `github-backup --version`
4. Run a manual backup once to confirm there are no regressions:
   ```bash
   github-backup octocat --all --output /tmp/test-backup --dry-run
   ```
5. Resume the scheduled backup.

---

## Log Levels

| Flag | Level | Output |
|------|-------|--------|
| (default) | `INFO` | Backup progress, statistics |
| `-v` | `DEBUG` | Per-file upload decisions, git commands |
| `-vv` | `TRACE` | HTTP request/response details |
| `-q` | `ERROR` | Errors only |

Set `RUST_LOG=github_backup=debug` for fine-grained filter control.
