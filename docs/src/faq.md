# FAQ

## General

### Does it support GitHub Enterprise?

Yes.  Use `--api-url` (or `GITHUB_API_URL`) to point `github-backup` at your
GitHub Enterprise Server instance.  The API is typically at
`https://github.example.com/api/v3`:

```bash
github-backup myorg \
  --token "$GITHUB_TOKEN" \
  --api-url https://github.example.com/api/v3 \
  --output /backup --org --all
```

Or in the config file:

```toml
owner   = "myorg"
api_url = "https://github.example.com/api/v3"
output  = "/var/backup/github"
org     = true
all     = true
```

### Can I back up a GitHub organisation?

Yes.  Pass `--org` to use the organisation repository listing API:

```bash
github-backup my-org --token $GITHUB_TOKEN --output /backup --org --all
```

### Will it overwrite existing backups?

- **Git repositories** are updated in-place (`git remote update --prune`), not re-cloned from scratch.  This is fast and incremental.
- **JSON files** are overwritten on each run with the latest data from the API.
- **Release assets** are skipped if the file already exists.

### Is it safe to run while a backup is in progress?

Do not run two instances for the same owner simultaneously.  Running two instances concurrently risks corruption of git repositories (concurrent writes to the same `.git/` directory).

### Can I back up multiple users/orgs into the same output directory?

Yes.  Each owner gets its own subdirectory: `<output>/<owner>/`.

### How do I restore from a mirror clone?

```bash
# Clone from the local mirror
git clone /backup/octocat/git/repos/Hello-World.git ~/restored/Hello-World

# Or push to a new remote
git -C /backup/octocat/git/repos/Hello-World.git \
    push --mirror https://github.com/new-owner/Hello-World.git
```

---

## Authentication

### What token scopes do I need?

| To back up | Required scope |
|-----------|---------------|
| Public repos | None (no token needed for public repos only) |
| Private repos | `repo` |
| Gists | `gist` |
| Org repos | `read:org` |
| Webhooks | `admin:repo_hook` |

### My PAT expired.  What happens?

`github-backup` will fail with an authentication error on the first API call.  Rotate the token in GitHub settings and update your `GITHUB_TOKEN` environment variable or config file.

### Can I use a GitHub App token?

GitHub App installation tokens work as long as they have the required permissions.  Pass the token via `--token` or `GITHUB_TOKEN`.

---

## Performance

### How long does a full backup take?

It depends on the number of repositories, repository sizes, the volume of
issues and pull requests, the concurrency setting, network bandwidth, and
GitHub's API rate limits. The dominant cost is usually `git clone` for new
repositories. Once a repository has been cloned once, subsequent runs only
fetch incremental updates, which is significantly faster.

### How do I speed up the backup?

1. Increase `--concurrency` (e.g. `--concurrency 16`)
2. Disable categories you don't need (avoid `--all` if you only want repos)
3. Use `--clone-type shallow:10` to limit history depth

### I'm hitting rate limits.  What should I do?

`github-backup` automatically backs off when it receives a `403` or `429` with rate-limit headers.  If the backoff window is too long, you can:
- Use a fine-grained PAT with higher rate limits
- Use GitHub Enterprise or GitHub Enterprise Managed Users which have higher rate limits
- Reduce `--concurrency` to slow down API consumption

---

## Storage

### How much disk space do I need?

Highly variable.  For a rough estimate:
- Each repository: 1 MB (small) to several GB (large)
- JSON metadata: 1–50 MB per repository depending on issue/PR volume

Run `du -sh /backup/<owner>` after a trial backup to estimate.

### Can I use a network filesystem (NFS, CIFS)?

Bare git repositories require filesystem support for atomic renames and file locking.  NFS v4 and CIFS work in practice but may be slower.  S3 sync is a better option for remote storage.

### Does S3 sync compress the data?

No. Objects are uploaded as-is. JSON metadata files compress well, so if
storage cost is a concern, configure server-side compression at the bucket
or object-store layer (or pair S3 sync with at-rest encryption via
`--encrypt-key`, which still leaves bucket-level compression available).

---

## Errors

### `clone failed: repository not found`

The repository exists but the token does not have access to it.  For private repos, ensure the token has the `repo` scope.

### `hooks: skipping (no admin access)`

Webhook backup requires admin/owner access.  If you don't have admin access to the repository, skip `--hooks`.

### `security advisories: skipping (not available)`

Security advisories are only available for public repositories and repos where the token has sufficient permissions.

### `cannot write report: permission denied`

The `--report` path is not writable.  Ensure the directory exists and the process has write permissions.
