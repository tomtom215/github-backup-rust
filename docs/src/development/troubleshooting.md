# Troubleshooting

The most common problems and how to resolve them.  First look at the **exit
status** and the summary at the end of the run (it lists every failure with its
reason): `3` means the backup finished but something could not be backed up;
`1` means it could not run.  The same list is in the `--report` file.  See
[Exit Codes](../configuration/cli-reference.md#exit-codes) and
[Monitoring](../monitoring.md).

Quick checks:

```bash
github-backup octocat --doctor        # git, output directory, network, token accepted?
github-backup octocat --dry-run --all # what would be backed up (writes nothing)
```

---

## Authentication Errors

### `GitHub API error 401: Bad credentials`

The token is missing, revoked, expired or mistyped.  The run stops at once with
exit status `1` and prints a hint.

- `github-backup --doctor` reports `token ... rejected by GitHub (HTTP 401)`.
- Check the token the process really sees: `GITHUB_TOKEN` must be exported in the
  environment where `github-backup` runs (cron and systemd do not read your
  shell profile).  An empty `GITHUB_TOKEN=` is treated as unset.
- Classic tokens need the scopes of the categories you enabled; see
  [Authentication](../getting-started/authentication.md#what-each-category-needs).

### `GitHub API error 403` / `Resource not accessible`

- The token lacks the scope or permission, or an organisation with SAML single
  sign-on has not authorised it (authorise the token in your GitHub token
  settings).
- For hooks, deploy keys, collaborators, advisories and branch protection a 403
  or 404 on a repository is logged at `INFO` level (`skipping hooks (no admin
  access)`) and the file is simply not written; the run is **not** marked
  incomplete.  If a file you expect is missing, look for `skipping` in the log.

### The tool warns that the token "cannot read GET /user"

GitHub App installation tokens cannot call `GET /user`.  For a user target the
tool then lists public repositories and gists only.  Use a personal access token
to back up private data of your own account.

### OAuth device flow ends with "OAuth device code expired"

You did not enter the code before it expired.  Run the command again.  "OAuth
authorisation was denied" means you declined the request in the browser.
`--oauth-client-id` is only valid with `--device-auth`.

---

## Rate Limit Errors

### `rate limited; waiting Ns (attempt K)`

A `429` or a `403` with rate-limit information (`Retry-After`, no remaining
requests, or a "rate limit" message): the client waits as long as GitHub says
(at least a minute, doubling, when GitHub gives no time) and retries, up to 6
times and about an hour of waiting per request.  Nothing to do unless it takes
too long.

### `rate limit exceeded; retry after Ns`

The wait would exceed that budget (for example a primary limit that resets in
more than an hour).  The run stops with exit status `1` and keeps its
checkpoint; run it again after the window resets.  To reduce the pressure:

1. Lower concurrency: `--concurrency 1`.
2. Enable fewer categories, or split the work with `--include-repos` over
   several runs.
3. Leave the incremental state in place (`backup_state.json`): unchanged issues
   and pull requests then cost no per-item requests.  (`--since` is not needed
   for this; it is an expert override.)

---

## Network and TLS Errors

### `TLS error: ...`, or `system TLS roots` fails in `--doctor`

No usable certificate store.  On minimal systems install the CA package
(`apk add ca-certificates`, `apt-get install ca-certificates`); the container
image includes it.  For a private CA set `SSL_CERT_FILE` (tool) and
`GIT_SSL_CAINFO` (git).

### `HTTP transport error: client error (Connect)`

The API cannot be reached.  Behind a proxy set `HTTPS_PROXY` (an HTTP proxy;
SOCKS is not supported):

```bash
export HTTPS_PROXY=http://proxy.example.com:3128
github-backup octocat --output /backup --all
```

When a proxy variable is present the log says `HTTP proxy configured from the
environment`.  `git` reads the same variables itself.  S3 requests and the
Gitea/GitLab API calls of `--mirror-to` ignore proxy settings and connect
directly.

### `request timed out`

Each request has a 120 s limit (headers, and the silence between two chunks of
the body); transient timeouts and `5xx` answers are retried (GET only, 3 times).
There is no flag to change the limit.  A `git` command that prints nothing for
10 minutes is stopped separately.

### `invalid API URL`

`--api-url` must be an `https://` URL with a host, for example
`https://github.example.com/api/v3`.

---

## Git Errors

### `git clone ... failed (exit 128): ... Repository not found`

The token cannot see the repository (private repository without `repo` scope,
SAML not authorised) or it was deleted or renamed.  The run goes on with the
other repositories and ends with exit status `3`.  `--private` is needed to
include private repositories at all.

### `detected dubious ownership`

`git` refuses a repository owned by a different user.  The tool trusts the exact
path it works on, so this is rare; if it happens run as the owning user
(`docker run --user`, `User=` in the unit) or `chown -R` the output directory.

### `git ... made no progress for 600s and was stopped`

A stalled connection.  Re-run; check the network and proxy.  A slow transfer
that keeps printing progress is never stopped.

### `git lfs` not found

`--lfs` needs `git-lfs` on the `PATH` (the container image has it).

### `git remote update` / `git fetch` fails

The repository is logged as failed and the rest continue.  Causes: network
interruptions, a repository deleted or transferred since the last run, a
directory owned by another user.

### A branch I deleted on GitHub is still in the backup

By design: deleted branches and tags are kept.  Pass `--prune` to remove them on
the next run.  See
[Clones Follow GitHub](../configuration/output-layout.md#clones-follow-github).

---

## Storage Errors

### `cannot create output directory` / `Permission denied`

`--output` must exist or be creatable, and be writable by the user running
`github-backup`:

```bash
mkdir -p /var/backup/github
chown backup-user:backup-group /var/backup/github
chmod 750 /var/backup/github
```

In a container the volume must be writable by the container user (UID 1000 by
default); see [Docker](../docker.md#users-and-permissions).

### `another backup for '<owner>' is already running`

Another `github-backup` holds the lock on that output directory.  Wait for it.
The lock is an operating-system lock that disappears when its process ends, so
a crashed or killed run never leaves a stale lock; the `.backup.lock` and
`.github-backup.lock` files that remain are harmless markers.

### Disk full

The run stops (`fatal error: stopping the run`, exit status `1`) and keeps its
checkpoint.  The tool does not check free space beforehand.  Free space and
re-run.  A simple pre-check:

```bash
REQUIRED_GB=50
AVAIL_GB=$(df --output=avail -BG /var/backup/github | tail -1 | tr -d 'G ')
[ "$AVAIL_GB" -ge "$REQUIRED_GB" ] || { echo "Not enough disk space" >&2; exit 1; }
```

### "0 repositories" in the summary

The listing came back empty or everything was filtered out.  Check the OWNER
spelling, `--org` for organisations, `--forks` / `--private` (forks and private
repositories are excluded by default), `--include-repos` / `--exclude-repos`, and
whether a user's token belongs to that user (private repositories need that).

---

## S3 Sync Issues

A failed upload is a recorded failure (exit status `3`) and the message names
the S3 error code.

### `AccessDenied` / `403`

Check the keys, the endpoint, and the policy: it needs `s3:ListBucket` on the
bucket and `s3:GetObject` and `s3:PutObject` on the objects (plus
`s3:DeleteObject` for `--s3-delete-stale`).  Without `s3:ListBucket` a `HEAD`
of a missing object answers 403 and every file is uploaded on every run.  See
[S3 permissions](../storage/s3.md#required-permissions).

### Objects are not updating

An object is skipped only when its stored SHA-256 digest and size match the local
file.  Changing the encryption key re-uploads everything.  To force a full
re-upload delete the objects (or use a new `--s3-prefix`).

### Nothing happens in `--dry-run`

A dry run skips the S3 step entirely.

---

## Mirroring Issues

### `401` / `403` on the destination

Check `--mirror-token` / `MIRROR_TOKEN` and that the token may create repositories
(and push) at the destination.

### A repository is refused as "foreign"

The destination already has a repository of that name that the tool did not
create (its description is not `GitHub mirror of <owner>/<repo>`) and it is not
empty.  The tool never pushes into such a repository.  Use another
`--mirror-owner`, or empty it first.

### `422` from the Gitea API

The tool treats HTTP 422 on creation as "already exists" (a race between the
existence check and the creation) and goes on to push.  If the repository does
not really exist, for example because the name is not valid at the destination,
the push fails and is reported.

---

## Enabling Debug Logging

```bash
github-backup octocat --output /backup --all -v 2>&1 | tee /tmp/debug.log
```

`-v` is debug, `-vv` is trace (connection-level events of the HTTP client;
request and response headers are not logged, and no level prints the token).
`RUST_LOG` overrides both.

---

## Reporting Bugs

Please open an issue at
[github.com/tomtom215/github-backup-rust/issues](https://github.com/tomtom215/github-backup-rust/issues)
and include:

1. The command you ran (redact tokens).
2. The relevant log output (`-v`) and, if you used `--report`, its `failures`.
3. Your operating system, and `git --version`.
4. The `github-backup --version` output (a build from `main` and the last release
   print the same number: say which one you use).
