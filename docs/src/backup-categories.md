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
| `mirror` (default) | `git clone --mirror` | `git fetch --all` | Complete backup: every branch and tag, `refs/pull/*`, full history |
| `bare` | `git clone --bare` | `git fetch origin '+refs/heads/*:refs/heads/*' '+refs/tags/*:refs/tags/*'` | Branches and tags only, no pull-request refs; slightly smaller |
| `full` | `git clone --no-local` into `repos/<repo>/` | `git fetch --update-head-ok origin` into local branches, `origin/*` and tags, then `git reset --hard` | A working tree you can browse or build; not pushed by `--mirror-to` |
| `shallow:<n>` | `git clone --mirror --no-single-branch --depth <n>` | `git fetch --depth <n>` | Limited history to save disk space; not an archive |

Every update also passes the pruning choice explicitly (`--prune`, or
`--no-prune --no-prune-tags`), so a `fetch.prune` or `fetch.pruneTags` setting
in your own git configuration never deletes refs the backup keeps.

How the clone types behave in detail:

* Updates are `fetch`es into the existing directory; nothing is cloned twice.
  Branches and tags deleted on GitHub are kept in the clone unless `--prune` is
  given; force-pushed branches are overwritten either way (see
  [Clones Follow GitHub](configuration/output-layout.md#clones-follow-github)).
* A fresh clone is made in a hidden staging directory next to the target and
  renamed into place only when it is complete, so an interrupted clone leaves
  nothing under the real name (the staging directory is removed on the next
  run).
* A **shallow** clone holds every branch and tag, each cut at `<n>` commits.
  Updates fetch what is new and keep the history cut at `<n>`; they do not
  deepen it.
* A **full** clone is the tool's own copy: each update sets every local
  branch, every `origin/*` branch and every tag to where it is on GitHub and
  resets the working tree (`git reset --hard`) to the checked-out branch, so
  local edits in it are discarded.  The checkout stays on the branch it was
  cloned with (the default branch at the time); if that branch is deleted on
  GitHub and `--prune` removes it, the working tree is left as it was.
* `--lfs` adds `git lfs fetch --all origin` after the mirror update (the clone
  itself is always a mirror: `--lfs` overrides `--clone-type`).  It needs
  `git-lfs` on the `PATH`; without `--lfs` the clone holds LFS pointer files
  only, not the large objects.
* A `git` command that prints no output for 10 minutes is treated as stalled
  and stopped (`SIGTERM`, then `SIGKILL` two seconds later); a clone that keeps
  making progress can take as long as it needs.  `git lfs` is told to report
  progress too (`GIT_LFS_FORCE_PROGRESS=1`).
* Before updating a repository, `.lock` files that a killed `git` left in it
  (under `refs/`, or `packed-refs.lock`, `index.lock` and the like) are
  removed when they are older than the stall limit; a younger one fails the
  update with a hint instead.
* `--prefer-ssh` clones over SSH; the token is then not used for git and git
  needs working SSH credentials.  Unless you set `GIT_SSH_COMMAND`, `GIT_SSH`
  or `core.sshCommand` yourself, `ssh` runs with `-o BatchMode=yes`: an unknown
  host key or a key with a passphrase fails the clone instead of waiting for an
  answer.  Put the host keys in `known_hosts` and use a key without a
  passphrase (or an agent).

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
`issue_comments/<n>.json` and `issue_events/<n>.json` for pull requests.  This
also holds for a repository with Issues switched off: GitHub then answers the
issues listing with `410 Gone` (no `issues.json`, not a failure), and the
conversation and events of every pull request are still fetched.  The events
come from `/issues/<n>/events`; the richer `/timeline` (cross-references,
commits, reviews) is not fetched.  The issue count in the summary does not
include pull requests.

An issue whose comments or events cannot be fetched is recorded as a failure
and the other issues are still backed up; an issue deleted in the meantime
(404/410) is skipped.  See [Issues and Pull Requests](issues-and-prs.md) for
the incremental behaviour.

Output: `<output>/<owner>/json/repos/<repo>/issues.json`, `issue_comments/<n>.json`, `issue_events/<n>.json`

---

## Pull Requests

| Flag | Description |
|------|-------------|
| `--pulls` | PR metadata (title, body, state, head/base refs) |
| `--pull-comments` | Inline review comments on PRs |
| `--pull-commits` | List of commits in each PR (beyond GitHub's 250, see below) |
| `--pull-reviews` | PR reviews (approve/request changes/comment) |

Output: `<output>/<owner>/json/repos/<repo>/pulls.json`, `pull_comments/<n>.json`, `pull_commits/<n>.json`, `pull_reviews/<n>.json`

A pull request's conversation thread and events are written by
`--issue-comments` / `--issue-events` (see Issues above).

GitHub's commit list of a pull request stops at 250 commits.  When it returns
250, the commits are taken from the comparison of the pull request's base and
head commits instead (`GET /repos/<owner>/<repo>/compare/<base>...<head>`,
every page), which has no such limit; if that comparison fails, the pull
request is recorded as a failure rather than stored truncated.

---

## Releases

| Flag | Description |
|------|-------------|
| `--releases` | Release metadata (tag, title, body, assets list) |
| `--release-assets` | Download binary release assets (requires `--releases`) |

> **Warning**: `--release-assets` can consume significant disk space for projects with large binary releases.

Output: `<output>/<owner>/json/repos/<repo>/releases.json`, assets in `release_assets/<tag>/<file>`, their checksums in `release_checksums/<tag>/<file>.sha256` (see [Releases](releases.md))

---

## Attachments

| Flag | Description |
|------|-------------|
| `--attachments` | Download the files and images attached to issues, pull requests, comments, reviews, commit comments and release notes |

GitHub has no API for attachments: they are links in the Markdown `body` of an
item.  After the other categories of a repository, the backup reads the bodies
it has stored (`issues.json`, `pulls.json`, `releases.json`,
`commit_comments.json`, and `issue_comments/<n>.json`, `pull_comments/<n>.json`
and `pull_reviews/<n>.json` of every stored issue and pull request), picks out
the links to GitHub's attachment hosts and downloads each URL once.  The stored
JSON is not changed; `attachments/index.json` maps every URL to its file.

Only `https` links of these forms are downloaded, wherever they appear in a body
(Markdown links and images, HTML `src`/`href`, bare text, also inside code
spans); no other host is contacted, except where GitHub itself redirects:

| Host | Path |
|------|------|
| `github.com` | `/user-attachments/assets/<id>`, `/user-attachments/files/<n>/<name>` |
| `github.com` | `/<owner>/<repo>/assets/<n>/<id>`, `/<owner>/<repo>/files/<n>/<name>` (older form) |
| `user-images.githubusercontent.com` | any (older public images) |
| `private-user-images.githubusercontent.com` | any (signed links that expire minutes after a page is rendered; Markdown bodies rarely contain them) |

Attachments on a GitHub Enterprise Server host are not downloaded.

Output: `<output>/<owner>/json/repos/<repo>/attachments/<sha256>.<ext>`, named by
the SHA-256 of the content (a file attached twice is stored once), with the
extension of the link's file name, else one for the media type GitHub sends,
else none; and `attachments/index.json`.  Each file is streamed to disk and
limited to 100 MiB (GitHub accepts at most 100 MB, for videos on paid plans).

The index entry of a URL holds `file`, `size`, `content_type`, `sha256` and
`first_seen`.  It is never removed, and an indexed URL whose file is on disk is
not downloaded again.  Every run reads all stored bodies again: for a
repository with 10,000 issues and 30,000 comments (50 MB of JSON) that took
0.2 s in a release build on the project's test machine, with every link
already indexed.

**Private repositories.**  Since May 2023 an attachment of a private repository
needs authentication.  The token is sent, as HTTP Basic authentication, to
`https://github.com` only (never to the storage host GitHub redirects to, and
never by a backup of a GitHub Enterprise Server).  python-github-backup
reports that classic personal access tokens work and that fine-grained and
GitHub App tokens are answered `404 Not Found` for many attachments of private
repositories; see [Authentication](getting-started/authentication.md#attachments-of-private-repositories).

What a failed download means:

* **404 or 410** (or any client error for an expired signed link): the URL is
  recorded in the index as missing (`missing` holds the status, `checked_at`
  the time), one warning per repository says so, and it is not a failure.  It
  is asked for again after 30 days, or on a run with `--full`.  Because GitHub
  answers 404 to tokens it does not accept for private attachments, run once
  with `--full` and a classic token after switching tokens.
* **401, 403**, or a web page (`text/html`, a sign-in page) instead of a file:
  the token may not read it.  With `--attachments` this is a failure (exit
  status `3`); with `--all` or the default selection one warning per run.  The
  URL is tried again next run.
* Anything else, including a file larger than 100 MiB: a failure of the
  repository's `attachments` step (exit status `3`); tried again next run.

---

## Wikis

| Flag | Description |
|------|-------------|
| `--wikis` | Clone repository wikis as bare mirror repos |

The wiki is cloned the same way as the repository: over SSH with
`--prefer-ssh`, and with `--clone-host` applied.

Output: `<output>/<owner>/git/wikis/<repo>.wiki.git/`

---

## Repository Metadata

| Flag | Description |
|------|-------------|
| `--labels` | Repository label definitions |
| `--milestones` | Repository milestones |
| `--hooks` | Webhook configurations (requires admin access) |
| `--security-advisories` | Security advisories (unpublished ones need a security manager or admin) |
| `--topics` | Repository topics (tags), taken from the repository object (no extra request) |
| `--branches` | Branch list with tip SHAs and protection status; detailed protection rules of protected branches in `branch_protections.json` (admin access); the repository rulesets, including those an organisation applies, in `rulesets.json` |
| `--deploy-keys` | Deploy keys attached to the repository (requires admin access) |
| `--collaborators` | Collaborator list with permissions (requires write access; `read:org` too for an organisation's repository) |

`--all` also saves the comments made on commits (`commit_comments.json`).

Output: `<output>/<owner>/json/repos/<repo>/labels.json`, `milestones.json`, `topics.json`, `branches.json`, `rulesets.json`, `deploy_keys.json`, `collaborators.json`, etc.

`info.json` is the repository as `GET /repos/<owner>/<repo>` returns it, with
`parent`, `source`, `template_repository` and the merge settings that the
repository listing leaves out (`temp_clone_token`, a short-lived credential, is
removed).  If that request fails, the listed object is stored instead and the
failure is reported.

### When GitHub answers 403, 404 or 410

* **404 or 410** for a category means it does not exist for that repository
  (Issues switched off, no rulesets API on an older GitHub Enterprise Server,
  ...): it is skipped with an `INFO` line and is not a failure.
* **403** means the token is not permitted to read it (for example webhooks
  without admin access).  For a category you asked for with its own flag, this
  is a **failure** that names the missing permission, so the run exits with
  status `3`.  With `--all`, which asks for everything that is available rather
  than for that category, the category is skipped and **one warning per
  category per run** names the missing permission; the exit status is not
  affected.  Rate limits are not 403s for this purpose: they are waited out.

---

## Gists

| Flag | Description |
|------|-------------|
| `--gists` | Clone gists owned by the backup target (secret gists too, see below) |
| `--starred-gists` | Save the metadata of gists starred by the authenticated user (**not cloned**) |

Output:
- Git (owned gists only): `<output>/<owner>/git/gists/<gist-id>.git/`
- Metadata: `<output>/<owner>/json/gists/<gist-id>.json`, `index.json`
- Comments (owned gists that have any): `<output>/<owner>/json/gists/<gist-id>.comments.json`
- Starred gists: `<output>/<owner>/json/gists/<gist-id>.starred.json`, `starred_index.json`

A gist id that is not a safe file name is never used as one (the gist is
reported as a failure), for owned and starred gists alike.

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
> Each of these lists is fetched on its own: one that fails is recorded and the
> others are still written.

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
full retained history.  GitHub deletes runs after its retention period; both
files accumulate (merged by `id`), so runs GitHub has deleted stay in the
backup.  This can be **very large** for active repositories; opt in
deliberately. It is omitted from `--all`.

Output: `<output>/<owner>/json/repos/<repo>/workflows.json`, `workflow_runs_<id>.json`

> **Token scope**: `actions:read` or a classic `repo` token is sufficient.
> Repositories with Actions disabled return 404, which is logged and skipped.
> A workflow whose runs cannot be fetched is recorded as a failure; the other
> workflows are still backed up.

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
| `--discussions` | **Nothing is backed up yet** |

> **Not available through the GitHub REST API.**  GitHub serves Discussions
> through its GraphQL API only; its published REST description has no
> repository discussions route, and this version does not back up through
> GraphQL.  The flag is accepted, **one warning per run** says that no
> discussion data is saved, and nothing is written.  `--all` does not include
> it.

---

## Projects

| Flag | Description |
|------|-------------|
| `--projects` | The owner's Projects (v2): every project, its fields and its items with their field values |

Projects (v2) belong to a user or an organisation, so they are owner-level
data, fetched through `GET /users/<owner>/projectsV2` or
`GET /orgs/<org>/projectsV2`, then each project's `fields` and `items`; the
items are asked for with the value of every field (GitHub returns at most 50
fields' values; a project with more is reported).  A classic token needs the
`read:project` scope.  Classic projects (v1) were sunset by GitHub and are not
backed up.

Output: `<output>/<owner>/json/projects/projects.json`, `projects/<number>/fields.json`, `projects/<number>/items.json`

---

## GitHub Packages

| Flag | Description |
|------|-------------|
| `--packages` | GitHub Packages metadata for the target user or organisation |

Iterates over the supported package ecosystems (container, npm, maven,
rubygems, nuget, docker) and saves the package list and version metadata to
the owner's JSON directory.  Requires the `read:packages` OAuth scope.  An
organisation's packages are listed under `/orgs/<org>`; when the token belongs
to the user being backed up, `/user/packages` is used, which includes private
packages; for anybody else only what `/users/<user>/packages` shows.  A
package type or package whose listing fails is recorded and the others are
still backed up.

Output: `<output>/<owner>/json/packages_<type>.json`, `package_versions_<type>_<name>.json`

In `<name>` every character other than ASCII letters, digits, `.`, `-` and
`_` is percent-encoded (`team/image` becomes `team%2Fimage`), so two packages
never share a file.  Names made only of those characters keep the file name
earlier versions used; a scoped or nested name (`@scope/pkg`, `team/image`)
gets a new one, and the file written under the old name stays as it was.

---

## The `--all` Flag

`--all` enables every category above, and commit comments, except:
- `--lfs` (requires git-lfs to be installed)
- `--prefer-ssh` (requires SSH keys to be set up)
- `--prune` (affects update behaviour)
- `--action-runs` (can be very large for active repositories)
- `--clone-starred` (can consume substantial disk space)
- `--discussions` (nothing is backed up yet)
- `--concurrency` (set separately)

With `--all`, a category the token is not permitted to read (HTTP 403) is a
warning, once per category per run, not a failure (see
[When GitHub answers 403, 404 or 410](#when-github-answers-403-404-or-410)).

`--forks` and `--private` **are** part of `--all`.  The excluded options can be given next to `--all` and are
honoured (`--all --clone-starred`, `--all --action-runs`, `--all --lfs`).  Individual category flags cannot be
combined with `--all` on the command line.

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup --all
```
