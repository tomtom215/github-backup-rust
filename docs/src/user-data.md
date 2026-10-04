# User & Organisation Data

These categories back up list-style metadata that is associated with the owner rather than individual repositories.

## Starred Repositories (JSON list)

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup --starred
```

Saves a JSON array of every repository the owner has starred:

```
json/starred.json
```

Each entry is the repository object exactly as GitHub returns it (name, description, owner, visibility, star and fork counts, topics, license, clone URLs and everything else GitHub includes).

---

## Clone Starred Repositories

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup --clone-starred
```

Clones (or updates) every starred repository as a bare mirror.  This is
independent from `--starred` — you can use either or both.

Repositories are cloned into:

```
git/starred/<upstream-owner>/<repo-name>.git
```

### Durable Queue and Refresh

`--clone-starred` uses a durable JSON queue stored at:

```
json/starred_clone_queue.json
```

The queue is written atomically after **every** repository, and processed one
repository at a time.  Two situations:

* **A pass is in progress** (some item is still `pending`, for example because
  the run was interrupted by Ctrl+C, a power cut or a network loss): re-running
  the same command **resumes** and leaves the finished items alone.
* **The previous pass is complete** (nothing is `pending`): the next run starts
  a **new pass**: every item, including earlier failures, goes back to `pending`
  with a fresh retry budget, so each run refreshes every starred mirror
  (`git fetch`) and retries what failed.  A large starred list therefore costs a
  full pass on every run.

```bash
# First run: clones whatever it can, writes progress to the queue
github-backup octocat --output /backup --clone-starred

# Interrupted?  Re-run the same command: it resumes where it left off
github-backup octocat --output /backup --clone-starred
```

Newly starred repositories are merged into the queue on every run.

### Retry & Backoff

Failed clones are retried up to **4 total attempts** with exponential backoff:

| Attempt | Delay before next retry |
|---------|------------------------|
| 1 (initial) | — |
| 2 | 5 s |
| 3 | 30 s |
| 4 (last) | 2 min |

After 4 failures the item is marked `"failed"` in the queue and counted as a
failure of the run (exit status `3`, `step: "starred clone <owner>/<repo>"` in
the report).  It is retried automatically in the next pass; there is nothing to
edit by hand.

### Progress Logging

After each clone the tool emits a structured log line:

```
INFO starred repo cloned repo="rust-lang/rust" done=42 pending=1505 failed=0 total=1547 rate_per_min="8.3" eta_secs=10880
```

### Clone Type

The clone mode follows the same `--clone-type` flag as owned repos (default:
`mirror`; `--lfs` is not applied to starred repositories).  Use `--prefer-ssh`
to clone via SSH instead of HTTPS.

### Not included in `--all`

`--clone-starred` is deliberately **not** enabled by `--all` because it can
consume significant disk space and run time for users with hundreds or
thousands of starred repositories.  Add it explicitly (`--all --clone-starred`
works) when needed.  It is skipped in a `--dry-run`.

## Watched Repositories

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup --watched
```

Saves repositories the owner is watching (not just the automatic watch on own repos):

```
json/watched.json
```

## Followers

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup --followers
```

Saves the list of accounts that follow the owner:

```
json/followers.json
```

## Following

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup --following
```

Saves the list of accounts the owner follows:

```
json/following.json
```

---

## Organisation Members

> **Organisation targets only** — this flag has no effect when backing up a regular user account.

```bash
github-backup my-org --org --token $GITHUB_TOKEN --output /backup --org-members
```

Saves the member list of the organisation that the token can see (members whose membership is private are listed only to a token that may see them):

```
json/org_members.json
```

Each entry is the user object exactly as GitHub returns it (login, ID, avatar URL, profile URL, ...).

## Organisation Teams

> **Organisation targets only** — this flag has no effect when backing up a regular user account.

```bash
github-backup my-org --org --token $GITHUB_TOKEN --output /backup --org-teams
```

Saves all teams within the organisation, including nested parent–child relationships:

```
json/org_teams.json
```

Each entry includes the team's name, slug, description, privacy setting, permission level, member and repository URLs, and an optional `parent` field for nested teams.

---

## All User Data Together

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup \
  --starred --watched --followers --following
```

For an organisation backup with all owner-level data:

```bash
github-backup my-org --org --token $GITHUB_TOKEN --output /backup \
  --starred --watched --followers --following \
  --org-members --org-teams
```

Or simply use `--all` to include everything.

---

## Organisation Notes

When using `--org`, all flags are available but some apply only to organisation targets.  `--starred`, `--watched`, `--followers` and `--following` call GitHub's `/users/<owner>/...` endpoints with the organisation's name; what GitHub returns for an organisation account has not been verified by the project:

| Flag | User target | Org target |
|------|-------------|-----------|
| `--starred` | User's starred repos | Org's starred repos |
| `--watched` | User's watched repos | Org's watched repos |
| `--followers` | User's followers | Org's followers |
| `--following` | User's following | Org's following |
| `--org-members` | *Silently skipped* | Organisation member list |
| `--org-teams` | *Silently skipped* | Organisation team list |
