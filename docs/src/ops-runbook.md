# Operations Runbook

Day-to-day procedures for a production `github-backup` deployment: health
checks, what to do when a run fails or is interrupted, key rotation,
verification and upgrades.  How the signals are produced is described in
[Monitoring & Reporting](monitoring.md).

---

## Daily Health Check

After each scheduled run, confirm:

1. **Exit status.**  `0` means complete.  `3` means the run finished but is
   **incomplete** (some items failed).  `1` means it could not be carried out.
   `130` / `143` mean it was interrupted.  Anything but `0` needs a look.

2. **Failures** (from the report, if `--report` is configured):

   ```bash
   jq '{success, failure_count, repos_errored}' /var/log/github-backup/report.json
   jq -r '.failures[] | "\(.scope): \(.step): \(.message)"' /var/log/github-backup/report.json
   ```

   The same list is printed at the end of the run in the summary banner.

3. **Prometheus metrics** (if `--prometheus-metrics` is configured):

   ```bash
   grep -E 'github_backup_(success|failures|last_success)' /var/lib/node_exporter/textfile_collector/github_backup.prom
   # github_backup_success{owner="..."} 1
   ```

4. **History** (always written): the last runs, newest last:

   ```bash
   jq '.entries[-3:]' /var/backup/github/octocat/json/backup_history.json
   ```

5. **S3** (if configured): a failed upload or deletion is a recorded failure
   (`step: "s3 sync"`) and makes the run exit `3`; the log line
   `S3 sync complete uploaded=N skipped=M errored=0 deleted=0` shows the counts.

---

## Backup Interrupted

### `SIGINT`, `SIGTERM` (exit 130 / 143)

`Ctrl+C`, `systemctl stop`, `docker stop` and a Kubernetes eviction send these.
The tool stops running `git` (the whole process group is killed), abandons
in-flight API requests, releases the lock and exits within about a second or
a few; nothing is left half-written under its real name.  What to expect:

1. `<output>/<owner>/json/backup_checkpoint.json` lists the repositories that
   were finished.
2. Re-run the same command.  Within 6 hours of the interruption the run resumes
   and skips those repositories; after that the checkpoint is ignored and every
   repository is refreshed.  Either way the git clones are only updated, not
   cloned again, and a repository that was in the middle of its **first** clone
   starts that clone again (the partial one is in a hidden staging directory,
   removed automatically).
3. `backup_state.json` is not advanced by an interrupted run, so nothing is
   skipped wrongly next time.

### `SIGKILL`, out-of-memory kill, power loss

The operating-system lock is released by the kernel, so **no stale lock
exists** and nothing has to be deleted before the next run (the lock files
stay on disk; they are only markers).  A `git` process may keep running for a
while after its parent was killed (it is not told to stop); it ends on its own,
and the next run removes leftover staging directories.  Start the next run
normally.

---

## Investigating Failures

Exit status `3` or `success: false` means one or more steps failed while the
rest completed.  Each entry of `failures` names a **scope** (a repository, the
owner or `post-processing`), the **step** and the error text.

| Typical message | Meaning and fix |
|-----------------|-----------------|
| `git clone ... failed (exit 128): ... Repository not found` | The token cannot see the repository (a private repository without a suitable token, or SAML single sign-on not authorised for the token), or it was deleted. |
| `... detected dubious ownership ...` | `git` refuses a repository owned by another user.  The tool normally trusts the path it works on; if you still see this, the directory was changed under it.  Run as the owning user (`docker run --user`) or `chown -R` the output. |
| `git ... made no progress for 600s and was stopped` | `git` printed nothing for 10 minutes (a stalled connection).  A slow but progressing clone is not interrupted.  Re-run; if it repeats, check the network or a proxy. |
| `GitHub API error 403 ... rate limit` / `rate limit exceeded` | See [Rate limit](#rate-limit). |
| `GitHub API error 404` on a category | Feature not available for that repository (no Actions, no environments); informational for most categories. |
| `no space left on device` | The disk is full: the run stops (`fatal error`, exit `1`). |
| `S3 ... AccessDenied` / `NoSuchBucket` / `SignatureDoesNotMatch` | See [S3 failures](#s3-failures). |

Re-running retries what failed and keeps everything that succeeded.

### Rate limit

The client waits out `429` responses and `403` responses that carry rate-limit
information (`Retry-After`, `X-RateLimit-Remaining: 0` or a "rate limit"
message) and retries (up to 6 times, at most about an hour of waiting per
request).  The log says `rate limited; waiting Ns`.  If the wait would exceed
the budget the run stops with a `rate limit exceeded` error (exit `1`) and the
next run starts from the checkpoint.  Check the quota:

```bash
curl -s -H "Authorization: Bearer $GITHUB_TOKEN" https://api.github.com/rate_limit | jq '.rate'
```

A token has 5 000 requests per hour (classic or fine-grained; GitHub App and
OAuth app tokens of an Enterprise Cloud organisation get more).  Reduce the
load with `--concurrency 1`, fewer categories, or `--include-repos` slices
spread over several runs.  Issue and pull request lists cost one request per 100
items every run; the per-item requests are what the watermarks save.

### S3 failures

A failed upload is a recorded failure; the message carries the HTTP status, the S3
error code and a hint.  Common causes:

- `SignatureDoesNotMatch` / `InvalidAccessKeyId`: wrong or expired credentials, or an
  endpoint belonging to another provider.
- `NoSuchBucket`: the bucket does not exist (it is never created) or the
  endpoint or region is wrong.
- `AccessDenied`: the policy lacks `s3:ListBucket`, `s3:GetObject` or
  `s3:PutObject` (see [required permissions](storage/s3.md#required-permissions)).
- `RequestTimeTooSkewed`: fix the system clock.
- "cannot reach the endpoint": network, DNS or a proxy (S3 ignores `HTTPS_PROXY`).

`--dry-run` does not test S3 (it skips the sync).

---

## Retention and Deleted Data

`--keep-last` and `--max-age-days` are **deprecated and ignored**: the tool
keeps one continuously updated backup per owner and never deletes snapshot
directories.  If you need point-in-time copies, snapshot the output directory
with restic, borg, ZFS or LVM, and apply the retention policy there.

Know what "update" means for deletions: by default a mirror update **keeps**
branches and tags that were deleted on GitHub, but it follows force-pushes (the
old commits of a force-pushed branch are not kept, and become unreachable until
`git gc` removes them).  Add `--prune` to delete refs that were deleted on
GitHub, and keep snapshots if you must be able to recover an earlier state.  `issues.json` and `pulls.json` keep items that
disappear from GitHub; other lists mirror the current state.

### S3 stale object cleanup

With `--s3-delete-stale` objects under `<prefix>/<owner>/json/` whose local file
is gone are deleted, but never when the run had failures, when the local tree
could not be fully read or is empty, or when an upload failed.  Deletion is
permanent unless the bucket is versioned.  There is no preview (a `--dry-run`
skips the S3 step).

---

## Encryption Key Rotation

Changing the key makes the next run upload every file again under the new
key (the stored content digest is keyed).  Objects whose local file is gone stay
under the old key.  The full procedure, including what to keep and when to
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

If `--manifest` was used, verify at any time (no network):

```bash
github-backup octocat --output /var/backup/github --verify
```

This compares the SHA-256 of every data file under `json/` (not the history, state,
checkpoint and lock files) with
`json/backup_manifest.json` and exits `1` if a file is missing, changed or
unexpected.  It does **not** examine the git clones: check those with
`git -C <repo>.git fsck`.  The manifest protects against accidental damage and
casual edits only: it is stored next to the files it describes and is not
signed, so someone who can write the directory can rewrite both.

---

## Restoring After Disaster

See the [Restore Guide](restore.md) for the full procedure and its limits.  Quick
reference:

```bash
# 1. Git data: create the empty repository first, then push branches and tags
git -C /backup/octocat/git/repos/my-repo.git push --prune \
    https://github.com/new-org/my-repo.git \
    '+refs/heads/*:refs/heads/*' '+refs/tags/*:refs/tags/*'

# 2. Labels, milestones and issues (from the local backup, no backup run first)
GITHUB_TOKEN=ghp_dummy_write_token \
github-backup octocat --output /backup --restore --restore-target-org new-org --restore-yes
```

---

## Upgrade Procedure

1. Read the [changelog](development/changelog.md), in particular the
   `Unreleased` / new version entry (breaking changes are listed there).
2. Stop the scheduled backup (systemd timer or cron job); wait for a running
   backup to finish.
3. Replace the binary (or pull the new image) and check
   `github-backup --version`.
4. Check the setup: `github-backup octocat --doctor`, and try the real
   configuration without writing anything: `github-backup --config ... --dry-run`.
5. Run one backup manually and check the exit status.
6. Resume the schedule.

The state, history and checkpoint files are read by newer versions; a file
from an older version only costs a full fetch.  An older binary does not
understand config keys added by a newer one (unknown keys are rejected).

---

## Log Levels

| Flag | Level | Output |
|------|-------|--------|
| (default) | `INFO` | Progress and summary |
| `-v` | `DEBUG` | Per-file decisions, git commands |
| `-vv` | `TRACE` | HTTP client and connection events |
| `-q` | `ERROR` | Errors only (no banners) |

Set `RUST_LOG=github_backup=debug` for fine-grained control; `RUST_LOG`
overrides `-v` and `-q`.  No level prints the token.
