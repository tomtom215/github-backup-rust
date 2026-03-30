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
   Non-zero `errored` means some files were not uploaded.

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

Inspect the logs for lines matching `failed to upload file to S3`.  Common
causes:
- Invalid or expired AWS credentials
- Bucket name or region mismatch
- IAM policy missing `s3:PutObject` / `s3:HeadObject` permissions

Validate credentials manually:
```bash
aws s3 ls s3://your-bucket/your-prefix/ --region us-east-1
```

---

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

**Warning:** this permanently deletes objects from S3 that are no longer in the
local backup.  Review your local retention policy before enabling.

---

## Encryption Key Rotation

To rotate the AES-256-GCM at-rest encryption key:

1. Generate a new key:
   ```bash
   openssl rand -hex 32
   ```

2. Download and decrypt all objects from S3 using the **old** key:
   ```bash
   # Example: decrypt a single file
   github-backup \
     --encrypt-key "$OLD_KEY" \
     --decrypt \
     --decrypt-input issues.json.enc \
     --decrypt-output issues.json
   ```

3. Re-encrypt and upload with the **new** key by running a full backup:
   ```bash
   export BACKUP_ENCRYPT_KEY="$NEW_KEY"
   github-backup octocat --all --s3-bucket my-backups
   ```

4. Delete the old `.enc` objects from S3:
   ```bash
   aws s3 rm s3://my-backups/ --recursive --exclude "*" --include "*.enc"
   ```

   Or use `--s3-delete-stale` on the next backup run to remove stale objects
   automatically.

5. Update the key stored in your secrets manager and revoke the old key.

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
