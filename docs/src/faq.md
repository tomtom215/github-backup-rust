# FAQ

## General

### Does it support GitHub Enterprise?

Yes, GitHub Enterprise Server through `--api-url` (or `GITHUB_API_URL`) and,
when the clone host differs, `--clone-host`.  GHES behaviour has not been
verified against a real instance; see
[GitHub Enterprise Server](configuration/github-enterprise.md).  Enterprise
Cloud uses `https://api.github.com` and needs nothing special.

### Can I back up a GitHub organisation?

Yes.  Name it; the tool recognises an organisation and backs it up as one (you
may still pass `--org`):

```bash
github-backup my-org --output /backup
```

### What does a run back up when I give no category flags?

Everything, as with `--all` (the plan banner says
`everything (no category selected ...)`).  Name categories to back up only
those (`--repositories --issues`), or leave some out of everything with
`--skip`, for example `--skip release-assets,packages`.

### Which account is backed up when I give no OWNER?

The account the token belongs to; the log says `no OWNER given: backing up
<login>`.  Give OWNER (or `owner` in the config file) for any other user or an
organisation.  An anonymous run (no token anywhere) needs OWNER.

### Can I run it as a long-lived service instead of with cron?

Yes: `--every 24h` (or `6h`, `30m`, ...) backs up at once and then at that
interval until it is stopped; see
[Built-in schedule](deployment/cron.md#built-in-schedule---every).

### What is *not* backed up?

See [What Is Not Restored, or Not Backed Up](restore.md#what-is-not-restored-or-not-backed-up).
In short: Actions secrets and variables, code-scanning and Dependabot alerts, issue
attachments unless `--attachments` (or `--all`), LFS objects unless `--lfs`, and
the contents of starred gists.

### Does a second run overwrite the first?

It updates it, and does not lose what the first run captured:

- **Git clones** are updated in place with `git fetch` (skipped when nothing
  was pushed since the last clean backup, at most for 7 days).  A branch or tag
  deleted on GitHub stays in the clone (unless `--prune`), while a force-pushed
  branch is overwritten.
- **`issues.json` and `pulls.json`** are **merged** into the stored file; an
  item that disappears from GitHub stays in the backup.  After the first run
  only the items updated since are listed.
- **Other JSON lists** are rewritten with the current response.
- **Per-item files** (comments, events, commits, reviews) are re-fetched only for
  items that changed since the repository's watermark.
- **Release assets** are kept when complete (size and checksum match).

See [Incremental runs](monitoring.md#incremental-runs-and-the-state-file).

### Is it safe to run two instances at once?

Not for the same owner, and the tool stops you: an operating-system lock in the
output directory makes the second run exit with status `1` ("another backup ...
is already running").  The lock is released automatically when a process ends,
however it ends.  Different owners (or different `--output` directories) can run
at the same time.

### Can I back up several users or organisations into the same directory?

Yes.  Each owner gets its own `<output>/<owner>/` directory, and S3 keys include
the owner as well.

### How do I read my backup without GitHub?

Render it as a website and open it in a browser:

```bash
github-backup octocat --output /backup --render-html
# then open /backup/octocat/site/index.html
```

Issues, pull requests with their reviews, releases, discussions, gists and
projects become pages that work offline.  See [Browsing a Backup](viewer.md).

### How do I restore a repository from the backup?

```bash
# Clone from the local mirror
git clone /backup/octocat/git/repos/Hello-World.git ~/restored/Hello-World

# Push to a new, empty GitHub repository: branches and tags, not --mirror
git -C /backup/octocat/git/repos/Hello-World.git push \
    https://github.com/new-owner/Hello-World.git \
    '+refs/heads/*:refs/heads/*' '+refs/tags/*:refs/tags/*'
```

`git push --mirror` fails against GitHub because a mirror contains
`refs/pull/*`.  The [Restore Guide](restore.md) has the details.

### What does the exit status mean?

`0` complete, `3` finished but incomplete (something failed; the summary says
what), `1` could not run, `2` usage error, `130`/`143` interrupted.  Failures
never stop the other repositories.  See
[Exit Codes](configuration/cli-reference.md#exit-codes).

---

## Authentication

### What token scopes do I need?

It depends on the categories; see
[What each category needs](getting-started/authentication.md#what-each-category-needs).
For a complete backup (the default) a classic token needs `repo`, `gist`,
`read:org`, `read:packages`, `read:project` and `admin:repo_hook`; without one of
them that category is skipped with a warning.  The GitHub CLI's login
(`gh auth login`, used automatically) has `repo`, `gist` and `read:org`;
`gh auth refresh -s read:packages,read:project` adds two more.
`github-backup --list-scopes` prints what your selection needs.  Public data
needs no token.

### My token expired.  What happens?

GitHub answers 401; the run stops at once (exit status `1`), reports the reason
and a hint, and writes the report, metrics and a `failure` webhook so monitoring
sees it.  Create a new token and update `GITHUB_TOKEN` (or the config file, or
log in again with `gh auth login`).
`github-backup --doctor` tells you whether the token is accepted.

### Can I run it as a GitHub App?

Yes: give `--app-id`, `--app-key` (the private key file, or the key itself in
`GITHUB_APP_PRIVATE_KEY`) and optionally `--app-installation-id`.  The tool
mints installation tokens and renews them before they expire, for API requests
and git alike, and lists every repository the installation can see, private
ones included.  See
[GitHub App](getting-started/authentication.md#github-app) for the permissions
to grant.

---

## Performance

### How long does a full backup take?

It depends on the number and size of the repositories, issue and pull-request
volume, `--concurrency`, bandwidth and GitHub's rate limits.  The first
`git clone` of each repository usually dominates; later runs only fetch changes.
A repository with many issues and pull requests costs one list request per 100
items on every run, plus the per-item requests for the items that changed.

### How do I speed it up?

1. Raise `--concurrency` (for example 8), as far as the rate limit allows.
2. Back up only the categories you need (name them, or `--skip` the others)
   instead of everything.
3. Use `--clone-type shallow:10` to limit history (at the cost of a complete
   backup).
4. Leave the incremental state alone: do not use `--full` on every run.

### I'm hitting rate limits.  What should I do?

The tool waits out rate-limit responses (`429`, and `403` with rate-limit
headers) and retries, up to about an hour per request; see
[Rate limit](ops-runbook.md#rate-limit).  A token (classic or fine-grained) has
5 000 requests per hour; tokens of GitHub Apps and OAuth apps owned or approved
by an Enterprise Cloud organisation get more.  To reduce the load:

- lower `--concurrency` (this slows the consumption, it does not reduce it),
- enable fewer categories, or split the work with `--include-repos` over several runs,
- keep the incremental state so the issue and pull request lists are asked
  only for changes and per-item requests are skipped.

---

## Storage

### How much disk space do I need?

It varies a lot.  Roughly: 1 MB to several GB per repository, and several
MB per 1 000 issues (the JSON holds GitHub's complete responses).  A mirror
keeps `refs/pull/*` too.  Measure with `du -sh /backup/<owner>` after a trial
run; the tool does not check free space.

### Can I use a network filesystem (NFS, CIFS)?

The tool relies on atomic renames and on operating-system file locks
(`flock` / `LockFileEx`).  A network filesystem that does not provide them can
make runs fail or locks ineffective; this has not been tested.  A local disk is
the safe choice; for an off-site copy use `--mirror-to` or a file-level tool on
top of the local backup.

### Does S3 sync compress the data?

No.  Objects are uploaded as they are.  JSON compresses well, so use the
provider's compression or lifecycle features if cost matters.  Only the JSON
metadata is uploaded, never the git clones.

---

## Errors

### `git clone ... failed (exit 128): ... Repository not found`

The token cannot see the repository (a private repository needs the `repo`
scope; an organisation with SAML single sign-on needs the token authorised for
it) or the repository was deleted.  The run continues with the others and ends
with exit status `3`.

### `skipping hooks (no admin access)`

`--hooks` (like `--deploy-keys`, `--collaborators` and the protection rules of
`--branches`) needs admin access.  When GitHub refuses (403), a category you
selected with its own flag is a failure naming the missing permission (exit
status `3`); under the default selection or `--all` it is one warning per
category per run.  A category that does not exist for a repository (404/410) is
skipped without a failure.

### `skipping security advisories (not available)`

Advisories exist only where the repository has them enabled and the token may
read them; the same rule applies.

### `failed to write report: ...`

The `--report` path is not writable (directory missing, permissions).  The
backup itself is unaffected and the exit status does not change; fix the path.
