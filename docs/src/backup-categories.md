# Backup Categories

`github-backup` organises backup targets into distinct categories.  Each category can be enabled individually with a flag, or all can be enabled at once with `--all`.

Every JSON file a category writes holds GitHub's complete API response (all
properties, GitHub's key order), not a summary: see
[Output Directory Layout](configuration/output-layout.md).

## Repositories

| Flag | Description |
|------|-------------|
| `--repositories` | Clone all repositories for the owner |
| `--forks` | Include forked repositories |
| `--private` | Include private repositories (requires `repo` scope; see below) |
| `--prefer-ssh` | Use SSH URLs instead of HTTPS for cloning |
| `--clone-type` | Clone mode: `mirror` (default), `bare`, `full`, `shallow:<n>` |
| `--lfs` | Enable Git LFS support |
| `--prune` | Delete branches and tags from the clone when they were deleted on GitHub (default: keep them) |

### Clone Types Explained

| Type | First clone | Later runs | Use case |
|------|-------------|-----------|----------|
| `mirror` (default) | `git clone --mirror` | `git fetch --all --prune` | Complete backup: every branch and tag, `refs/pull/*`, full history |
| `bare` | `git clone --bare` | `git fetch --prune origin '+refs/heads/*:refs/heads/*' '+refs/tags/*:refs/tags/*'` | Branches and tags only, no pull-request refs; slightly smaller |
| `full` | `git clone --no-local` into `repos/<repo>/` | `git fetch --all --prune` | A working tree you can browse or build; not pushed by `--mirror-to` |
| `shallow:<n>` | `git clone --mirror --depth <n>` | `git fetch --depth <n>` | Limited history to save disk space; not an archive |

How the clone types behave in detail:

* Updates are `fetch`es into the existing directory; nothing is cloned twice.
  Branches and tags deleted on GitHub are kept in the clone unless `--prune` is
  given; force-pushed branches are overwritten either way (see
  [Clones Follow GitHub](configuration/output-layout.md#clones-follow-github)).
* A fresh clone is made in a hidden staging directory next to the target and
  renamed into place only when it is complete, so an interrupted clone leaves
  nothing under the real name (the staging directory is removed on the next
  run).
* A **shallow** clone contains only the repository's default branch (and the
  tags that point into it) at first, because git's `--depth` implies a single
  branch for the initial clone; the next update fetches every branch at the
  requested depth.
* `--lfs` adds `git lfs fetch --all origin` after the mirror update (the clone
  itself is always a mirror: `--lfs` overrides `--clone-type`).  It needs
  `git-lfs` on the `PATH`; without `--lfs` the clone holds LFS pointer files
  only, not the large objects.
* A `git` command that prints no output for 10 minutes is treated as stalled
  and killed; a clone that keeps making progress can take as long as it needs.
* `--prefer-ssh` clones over SSH; the token is then not used for git and git
  needs working SSH credentials.

Example:
```bash
# Mirror clone (default), recommended
github-backup octocat --output /backup --repositories

# Shallow clone, last 5 commits
github-backup octocat --output /backup --repositories --clone-type shallow:5
```

Output: `<output>/<owner>/git/repos/<repo>.git/` (`<repo>/` for `--clone-type full`)

### Private repositories

GitHub's public listing (`GET /users/<owner>/repos`) never contains private
repositories.  When the token **belongs to the account you are backing up**
(the login is compared case-insensitively), the tool lists
`GET /user/repos?affiliation=owner&visibility=all` instead, which includes the
account's own private repositories, and `--private` then takes effect.  Only
repositories the account owns are listed this way; repositories owned by
organisations are backed up with `--org`.

For any other user, without a token, or with a token that cannot call
`GET /user` (a GitHub App installation token; a warning is logged), only public
repositories are visible.  Organisation targets (`--org`) list private
repositories the token can see, as before.

---

## Issues

| Flag | Description |
|------|-------------|
| `--issues` | Issue metadata (title, body, state, labels, assignees) |
| `--issue-comments` | Comment threads of every issue **and pull request** (a PR's conversation) |
| `--issue-events` | Events of every issue and pull request (`closed`, `labeled`, `assigned`, ...) |

Issues and pull requests share one number space, and GitHub's issues API lists
pull requests too, so `--issue-comments` and `--issue-events` also write
`issue_comments/<n>.json` and `issue_events/<n>.json` for pull requests.  The
events come from `/issues/<n>/events`; the richer `/timeline` (cross-references,
commits, reviews) is not fetched.

Output: `<output>/<owner>/json/repos/<repo>/issues.json`, `issue_comments/<n>.json`, `issue_events/<n>.json`

---

## Pull Requests

| Flag | Description |
|------|-------------|
| `--pulls` | PR metadata (title, body, state, head/base refs) |
| `--pull-comments` | Inline review comments on PRs |
| `--pull-commits` | List of commits in each PR (GitHub lists at most 250) |
| `--pull-reviews` | PR reviews (approve/request changes/comment) |

Output: `<output>/<owner>/json/repos/<repo>/pulls.json`, `pull_comments/<n>.json`, `pull_commits/<n>.json`, `pull_reviews/<n>.json`

A pull request's conversation thread and events are written by
`--issue-comments` / `--issue-events` (see Issues above).

---

## Releases

| Flag | Description |
|------|-------------|
| `--releases` | Release metadata (tag, title, body, assets list) |
| `--release-assets` | Download binary release assets (requires `--releases`) |

> **Warning**: `--release-assets` can consume significant disk space for projects with large binary releases.

Output: `<output>/<owner>/json/repos/<repo>/releases.json`, assets in `release_assets/<tag>/<file>` (+ `<file>.sha256`)

---

## Wikis

| Flag | Description |
|------|-------------|
| `--wikis` | Clone repository wikis as bare mirror repos |

Output: `<output>/<owner>/git/wikis/<repo>.wiki.git/`

---

## Repository Metadata

| Flag | Description |
|------|-------------|
| `--labels` | Repository label definitions |
| `--milestones` | Repository milestones |
| `--hooks` | Webhook configurations (requires admin access) |
| `--security-advisories` | Published security advisories |
| `--topics` | Repository topics (tags) |
| `--branches` | Branch list with tip SHAs and protection status; detailed protection rules of protected branches in `branch_protections.json` (admin access) |
| `--deploy-keys` | Deploy keys attached to the repository (requires admin access) |
| `--collaborators` | Collaborator list with permissions (requires admin access) |

Output: `<output>/<owner>/json/repos/<repo>/labels.json`, `milestones.json`, `topics.json`, `branches.json`, `deploy_keys.json`, `collaborators.json`, etc.

> **Note**: `--hooks`, `--deploy-keys`, `--collaborators`, `--security-advisories`, the protection rules of
> `--branches`, `--actions` and `--environments` need admin or equivalent access.  When GitHub answers
> **403 or 404** for a repository the tool treats it as "not available to this token", writes an `INFO`
> line and skips that file.  This is **not** counted as a failure and does not change the exit status, so
> a token without the needed permission yields a backup without those files and exit status `0`.
> Look for the file in the output (or for `skipping` in the log) when you rely on these categories.

---

## Gists

| Flag | Description |
|------|-------------|
| `--gists` | Clone gists owned by the backup target (secret gists too, see below) |
| `--starred-gists` | Save the metadata of gists starred by the authenticated user (**not cloned**) |

Output:
- Git (owned gists only): `<output>/<owner>/git/gists/<gist-id>.git/`
- Metadata: `<output>/<owner>/json/gists/<gist-id>.json`, `index.json`
- Starred gists: `<output>/<owner>/json/gists/<gist-id>.starred.json`, `starred_index.json`

`--starred-gists` records the gist objects GitHub lists (description, owner,
file names and sizes, URLs); it does not clone their contents.

GitHub's public listing (`GET /users/<owner>/gists`) omits secret gists.  When
the token belongs to the account being backed up, `GET /gists` is used and
secret gists are included; otherwise only public gists are visible.

---

## User / Organisation Data

| Flag | Description | Target |
|------|-------------|--------|
| `--starred` | Starred repos as a JSON list | User & Org |
| `--clone-starred` | Clone every starred repo as a bare mirror (durable queue, pause/resume) | User & Org |
| `--watched` | Repositories watched by the owner | User & Org |
| `--followers` | Follower list | User & Org |
| `--following` | Following list | User & Org |
| `--org-members` | Organisation member list | **Org only** |
| `--org-teams` | Organisation team list | **Org only** |

Output: `<output>/<owner>/json/starred.json`, `watched.json`, `org_members.json`, `org_teams.json`, etc.
Cloned starred repos: `<output>/<owner>/git/starred/<upstream-owner>/<repo>.git`

> **Note**: `--org-members` and `--org-teams` are silently skipped for user targets. `--clone-starred` is intentionally omitted from `--all` due to its potentially large footprint.

---

## GitHub Actions

| Flag | Description |
|------|-------------|
| `--actions` | Workflow metadata (id, name, path, state, badge URL) |
| `--action-runs` | Run history per workflow (requires `--actions`) |

`--actions` saves `workflows.json` to each repository's metadata directory.
The actual workflow YAML files are already captured by the git clone; this flag
records the API-level metadata that is not part of the repository tree (workflow
IDs, states, badge URLs).

`--action-runs` writes one file per workflow (`workflow_runs_<id>.json`) with
the workflow's runs; every page of GitHub's response is fetched, so this is the
full retained history.  This can be **very large** for active repositories;
opt in deliberately. It is omitted from `--all`.

Output: `<output>/<owner>/json/repos/<repo>/workflows.json`, `workflow_runs_<id>.json`

> **Token scope**: `actions:read` or a classic `repo` token is sufficient.
> Repositories with Actions disabled return 404, which is logged and skipped.

---

## Deployment Environments

| Flag | Description |
|------|-------------|
| `--environments` | Deployment environment configs (protection rules, reviewers, branch policies) |

Environments model deployment targets such as `staging` or `production`.  Their
configurations include protection rules (required reviewers, wait timers) and
branch policies that gate automated deployments.  Backing up this metadata makes
it possible to audit and reproduce deployment gate configurations without a live
GitHub connection.

Output: `<output>/<owner>/json/repos/<repo>/environments.json`

> **Note**: Repositories without environments return 404, which is logged and skipped silently.

---

## Discussions

| Flag | Description |
|------|-------------|
| `--discussions` | **Not functional on github.com** |

> **Not supported by the GitHub REST API.**  GitHub exposes Discussions through
> GraphQL only; the REST route this flag calls
> (`GET /repos/<owner>/<repo>/discussions`) does not exist in GitHub's published
> API description (github.com, GHEC, GHES 3.17-3.19) and answers 404.  The flag
> is accepted, a single warning per run says that nothing is backed up, and no
> `discussions.json` is written.  Do not rely on it for a backup of Discussions.

---

## Classic Projects

| Flag | Description |
|------|-------------|
| `--projects` | **Not functional on github.com** |

> **Not supported by the GitHub REST API.**  GitHub sunset Classic Projects; the
> REST routes this flag calls (`/repos/<owner>/<repo>/projects`,
> `/projects/<id>/columns`) are absent from GitHub's published API description
> and answer 404/410.  The flag is accepted, a single warning per run says that
> nothing is backed up, and no `projects.json` is written.  Projects v2 are not
> implemented.

---

## GitHub Packages

| Flag | Description |
|------|-------------|
| `--packages` | GitHub Packages metadata for the target user |

Iterates over the supported package ecosystems (container, npm, maven,
rubygems, nuget, docker) and saves the package list and version metadata to
the owner's JSON directory.  Requires the `read:packages` OAuth scope.

Output: `<output>/<owner>/json/packages_<type>.json`

---

## The `--all` Flag

`--all` enables every category above (including the non-functional `--discussions` and `--projects`, which
log a warning) except:
- `--lfs` (requires git-lfs to be installed)
- `--prefer-ssh` (requires SSH keys to be set up)
- `--prune` (affects update behaviour)
- `--action-runs` (can be very large for active repositories)
- `--clone-starred` (can consume substantial disk space)
- `--concurrency` (set separately)

`--forks` and `--private` **are** part of `--all`.  The excluded options can be given next to `--all` and are
honoured (`--all --clone-starred`, `--all --action-runs`, `--all --lfs`).  Individual category flags cannot be
combined with `--all` on the command line.

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup --all
```
