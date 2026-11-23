# Output Directory Layout

This page explains the directory and file structure that `github-backup` creates under `--output`.

## The JSON files are GitHub's own responses

Every `*.json` file listed below holds the objects **exactly as the GitHub API
returned them**: every property, in GitHub's key order, including the ones this
tool does not use itself (`reactions`, `node_id`, `_links`, URL templates,
review-comment anchors, ...).  Nothing is dropped and nothing is invented (a
missing property stays missing; it is not turned into `null`).

- A list that contains an object the tool cannot interpret (for example a
  `null` where an object is expected) is **still written in full**: that
  object is appended verbatim after the others and a warning naming the
  endpoint, position and `id`/`number` is logged.  One odd object never stops
  the list or the other categories.
- Files are written deterministically: the same data produces the same bytes,
  so diffs and checksums are meaningful.
- Expect the JSON to be several times larger than a summary would be.

Files that are lists hold a JSON array.  `workflows.json`, `workflow_runs_<id>.json`
and `environments.json` hold the array of items (every page of GitHub's
`{"total_count": ..., "<items>": [...]}` response merged); `topics.json` holds
the list of topic names (taken from the repository object);
`branch_protections.json` is an object keyed by branch name; `rulesets.json`
holds the full object of every ruleset, ordered by `id`.

Two exceptions to "exactly as returned": `temp_clone_token`, a short-lived
credential GitHub may put in repository objects, is removed (a credential is
never written to a file), and a file that **accumulates** (below) holds every
object it has ever seen, the current ones as GitHub returns them now.

## Snapshots and accumulating history

A backup should never forget what it captured, so most files **accumulate**:
each run merges what GitHub returns now into what is already stored, matching
objects on a stable key.  An object GitHub returns again replaces the stored
copy; an object GitHub no longer returns (deleted, or past GitHub's retention)
**stays**.  Files that describe *who has access or how the repository is
configured now* are **snapshots** instead, replaced on every run, because a
removed deploy key or collaborator must not look as if it were still there.

| Kind | Files (key) |
|------|-------------|
| Accumulating | `repos.json` (`id`); `issues.json`, `pulls.json` (`number`); `issue_comments/<n>.json`, `issue_events/<n>.json`, `pull_comments/<n>.json`, `pull_reviews/<n>.json` (`id`); `pull_commits/<n>.json` (`sha`); `releases.json`, `labels.json`, `milestones.json`, `commit_comments.json`, `workflows.json`, `workflow_runs_<id>.json` (`id`); `security_advisories.json` (`ghsa_id`); `gists/index.json`, `gists/<id>.comments.json` (`id`); `packages_<type>.json`, `package_versions_<type>_<name>.json` (`id`); `projects/projects.json`, `projects/<n>/fields.json`, `projects/<n>/items.json` (`id`); `discussions.json`, `discussion_categories.json`, `discussion_comments/<n>.json` (`id`; replies are kept inside their comment); `attachments/index.json` (the URL) |
| Snapshot | `info.json`, `topics.json`, `branches.json`, `branch_protections.json`, `rulesets.json`, `hooks.json`, `deploy_keys.json`, `collaborators.json`, `environments.json`, `starred.json`, `watched.json`, `followers.json`, `following.json`, `org_members.json`, `org_teams.json`, `gists/starred_index.json`, `gists/<id>.json`, `gists/<id>.starred.json` |

Within an accumulating file the objects are ordered by their key when it is a
number (by stored order, then new objects, otherwise).  Note that a restore
(`--restore`) from `labels.json` and `milestones.json` therefore also recreates
labels and milestones that were deleted on GitHub after they were backed up.

## Tree

```
<output>/
├── .github-backup.lock                     ← process lock (stays on disk, see below)
└── <owner>/                                ← GitHub username or org name
    ├── git/                                ← Git repositories
    │   ├── repos/
    │   │   ├── <repo>.git/                 ← mirror clone (default); bare, shallow, --lfs too
    │   │   ├── <repo>/                     ← working tree, with --clone-type full
    │   │   └── …
    │   ├── wikis/
    │   │   ├── <repo>.wiki.git/            ← wiki mirror clone
    │   │   └── …
    │   ├── gists/
    │   │   ├── <gist-id>.git/              ← gist mirror clone
    │   │   └── …
    │   └── starred/                        ← --clone-starred
    │       └── <upstream-owner>/<repo>.git/
    └── json/                               ← JSON metadata
        ├── repos.json                      ← the repositories this backup covers
        ├── starred.json                    ← --starred
        ├── watched.json                    ← --watched
        ├── followers.json                  ← --followers
        ├── following.json                  ← --following
        ├── org_members.json                ← --org-members   (org targets)
        ├── org_teams.json                  ← --org-teams     (org targets)
        ├── packages_<type>.json            ← --packages
        ├── package_versions_<type>_<name>.json ← <name> percent-encoded
        ├── projects/                       ← --projects (Projects v2)
        │   ├── projects.json
        │   └── <number>/
        │       ├── fields.json
        │       └── items.json              ← items with their field values
        ├── starred_clone_queue.json        ← --clone-starred progress
        ├── backup_state.json               ← incremental watermarks per repository
        ├── backup_history.json             ← recent runs (--history-size)
        ├── backup_checkpoint.json          ← only after an interrupted run
        ├── backup_manifest.json            ← --manifest
        ├── .backup.lock                    ← engine lock (stays on disk, see below)
        ├── gists/
        │   ├── <gist-id>.json              ← --gists
        │   ├── <gist-id>.comments.json     ← --gists, when the gist has comments
        │   ├── <gist-id>.starred.json      ← --starred-gists
        │   ├── index.json                  ← all owned gists
        │   └── starred_index.json          ← all starred gists
        └── repos/
            ├── <repo>@<id>/                ← an earlier repository of that name (see below)
            └── <repo>/
                ├── info.json               ← GET /repos/<owner>/<repo> (always written)
                ├── issues.json
                ├── issue_comments/<n>.json ← one file per issue or pull request
                ├── issue_events/<n>.json   ← one file per issue or pull request
                ├── pulls.json
                ├── pull_comments/<n>.json  ← one file per pull request
                ├── pull_commits/<n>.json
                ├── pull_reviews/<n>.json
                ├── releases.json
                ├── release_assets/
                │   └── <tag>/
                │       └── <asset-file>        ← binary release asset
                ├── release_checksums/
                │   └── <tag>/
                │       └── <asset-file>.sha256 ← its checksum
                ├── labels.json
                ├── milestones.json
                ├── hooks.json
                ├── security_advisories.json
                ├── topics.json
                ├── branches.json
                ├── branch_protections.json ← protected branches (needs admin access)
                ├── rulesets.json
                ├── deploy_keys.json
                ├── collaborators.json
                ├── commit_comments.json    ← --all
                ├── attachments/            ← --attachments
                │   ├── <sha256>.<ext>      ← one attached file, named by its content
                │   └── index.json          ← URL → file, size, type, digest, first seen
                ├── workflows.json
                ├── workflow_runs_<id>.json ← one per workflow (--action-runs)
                ├── environments.json
                ├── discussions.json        ← GraphQL nodes, most recently updated first
                ├── discussion_categories.json
                └── discussion_comments/<n>.json ← comments of discussion <n>, each with its replies
```

The discussion files hold GraphQL nodes (camelCase properties such as
`updatedAt`), not REST objects; see
[Discussions](../backup-categories.md#discussions).
Repositories that the options exclude (forks without `--forks`, private
repositories without `--private`, `--include-repos` / `--exclude-repos`) get
neither a clone nor a metadata directory.  The include and exclude patterns
are matched against the repository **name** only (`my-repo`), never against
`owner/name`.

### A repository deleted and recreated under the same name

Directories are named after the repository, so a repository that was deleted
on GitHub and recreated under the same name would land in the old one's
directory and mix two repositories' issues and history.  Before backing up a
repository, the tool compares the `id` in its stored `info.json` with the
repository's current `id`.  When they differ, the earlier backup is set aside,
not deleted: `json/repos/<repo>/` becomes `json/repos/<repo>@<old id>/`, and
`git/repos/<repo>.git` (or `<repo>/`) and `git/wikis/<repo>.wiki.git` get the
same `@<old id>` suffix.  `@` cannot occur in a GitHub repository name, so the
new name never collides with a real repository.  A warning is logged and the
new repository is backed up from scratch.

### Bookkeeping files

| File | Meaning |
|------|---------|
| `repos.json` | The repositories this backup covers, as GitHub listed them: only the ones the options include, merged with earlier listings so that a repository deleted on GitHub stays recorded.  `--diff-with` compares it between two backups. |
| `backup_state.json` | One watermark per repository (`repos.<owner/repo>.at`, plus the categories it covers and `fetched_at`, the last clean run that fetched its clone) and `last_successful_run`.  See [Incremental runs](../monitoring.md#incremental-runs-and-the-state-file).  Safe to delete: the next run fetches everything once. |
| `backup_history.json` | One entry per run (time, repositories backed up, duration, `success`, `failures`, tool version), newest last, at most `--history-size` entries. |
| `backup_checkpoint.json` | Which repositories an **interrupted** run had finished.  Removed when a run completes (a checkpoint that cannot be removed is a failure of the run).  A later run resumes from it only if it is less than 6 hours old and the run has the same owner and options; otherwise it is ignored. |
| `backup_manifest.json` | `--manifest`: SHA-256 of every data file under `json/`, except the history, state, checkpoint and lock files and temporary files of an unfinished write (the git clones are not included).  Paths use `/` on every platform. |
| `starred_clone_queue.json` | `--clone-starred` progress; see [User data](../user-data.md#durable-queue-and-refresh). |

### Lock files

Two lock files serialise runs: `<output>/.github-backup.lock` and
`<owner>/json/.backup.lock`.  They are operating-system file locks held for
the duration of the run and released by the kernel when the process ends, however it
ends (including `kill -9`, an out-of-memory kill or a power cut).  The files
themselves are **left on disk** and are harmless; there is nothing to clean up
and no stale-lock state.  A second run on the same owner stops with "another
backup ... is already running" (exit status `1`).

## Issue and pull request numbers share one space

GitHub numbers issues and pull requests from the same counter, so `#42` is
either an issue or a pull request, never both.  The per-item files are named by
that number:

| Number is a... | `issue_comments/<n>.json` | `issue_events/<n>.json` | `pull_comments/<n>.json`, `pull_commits/<n>.json`, `pull_reviews/<n>.json` |
|---|---|---|---|
| issue | comments | events | — |
| pull request | the **conversation** thread (the comments under the PR description) | label, assign, close, merge, ... events | inline review comments, commits, reviews |

A pull request appears in `issues.json` as well as in `pulls.json` (GitHub's
issues API lists pull requests too; they carry a `pull_request` property).

## File Descriptions

### Git repositories (`git/`)

| Path pattern | Clone command | Contents |
|-------------|---------------|----------|
| `git/repos/<repo>.git/` | `git clone --mirror` | All refs, all history (default) |
| `git/repos/<repo>/` | `git clone` | Working tree (`--clone-type full`); not pushed by `--mirror-to` |
| `git/wikis/<repo>.wiki.git/` | `git clone --mirror` | Wiki pages as Markdown |
| `git/gists/<gist-id>.git/` | `git clone --mirror` | Gist file history |
| `git/starred/<owner>/<repo>.git/` | per `--clone-type` | Starred repositories (`--clone-starred`) |

`--clone-type bare` and `shallow:<n>` also use `<repo>.git`; `--lfs` uses a
mirror clone plus the LFS objects in the same directory.

### JSON metadata (`json/`)

| File | Enabled by |
|------|-----------|
| `repos.json` | always (see above) |
| `starred.json` | `--starred` |
| `watched.json` | `--watched` |
| `followers.json` | `--followers` |
| `following.json` | `--following` |
| `org_members.json` | `--org-members` (org targets) |
| `org_teams.json` | `--org-teams` (org targets) |
| `packages_<type>.json`, `package_versions_<type>_<name>.json` | `--packages` |
| `projects/projects.json`, `projects/<number>/fields.json`, `projects/<number>/items.json` | `--projects` |
| `gists/<id>.json`, `gists/<id>.comments.json`, `gists/index.json` | `--gists` |
| `gists/<id>.starred.json`, `gists/starred_index.json` | `--starred-gists` |
| `repos/<name>/info.json` | always, for every backed-up repository |
| `repos/<name>/issues.json` | `--issues` |
| `repos/<name>/issue_comments/<n>.json` | `--issue-comments` |
| `repos/<name>/issue_events/<n>.json` | `--issue-events` |
| `repos/<name>/pulls.json` | `--pulls` |
| `repos/<name>/pull_comments/<n>.json` | `--pull-comments` |
| `repos/<name>/pull_commits/<n>.json` | `--pull-commits` |
| `repos/<name>/pull_reviews/<n>.json` | `--pull-reviews` |
| `repos/<name>/releases.json` | `--releases` |
| `repos/<name>/release_assets/<tag>/<file>`, `release_checksums/<tag>/<file>.sha256` | `--release-assets` |
| `repos/<name>/labels.json` | `--labels` |
| `repos/<name>/milestones.json` | `--milestones` |
| `repos/<name>/hooks.json` | `--hooks` |
| `repos/<name>/security_advisories.json` | `--security-advisories` |
| `repos/<name>/topics.json` | `--topics` |
| `repos/<name>/branches.json`, `branch_protections.json`, `rulesets.json` | `--branches` |
| `repos/<name>/deploy_keys.json` | `--deploy-keys` |
| `repos/<name>/collaborators.json` | `--collaborators` |
| `repos/<name>/commit_comments.json` | `--all` |
| `repos/<name>/attachments/<sha256>.<ext>`, `attachments/index.json` | `--attachments` |
| `repos/<name>/workflows.json` | `--actions` |
| `repos/<name>/workflow_runs_<id>.json` | `--action-runs` |
| `repos/<name>/environments.json` | `--environments` |

### Attachments

Files attached to issues, pull requests, discussions, comments, reviews, commit
comments and release notes (see [Attachments](../backup-categories.md#attachments)).  The
stored JSON keeps the original links; `index.json` is an object keyed by the
link as it appears in the body (`&amp;` read as `&`, without a `#fragment`):

```json
{
  "https://github.com/user-attachments/assets/0b1f6c2e-...": {
    "file": "5f70bf18a086007016e948b04aed3b82103a36bea41755b6cddfaf10ace3c6ef.png",
    "size": 48213,
    "content_type": "image/png",
    "sha256": "5f70bf18a086007016e948b04aed3b82103a36bea41755b6cddfaf10ace3c6ef",
    "first_seen": "2026-11-23T10:00:00Z"
  },
  "https://github.com/user-attachments/files/17/deleted.log": {
    "first_seen": "2026-11-23T10:00:00Z",
    "missing": 404,
    "checked_at": "2026-11-23T10:00:00Z"
  }
}
```

`file` is relative to `attachments/`; a file attached under several links is
stored once.  `missing` and `checked_at` mark a link GitHub answered 404 or 410
(or, for an expired signed link, another client error); an earlier download of
that link keeps its `file`.  A download is streamed to `attachments/.incoming`
and renamed once its digest is known; the index is written every 25 downloads
and at the end, so an interrupted run keeps most of its work.  The S3 sync
uploads `attachments/` like the JSON files (only `release_assets/` is held back
without `--s3-include-assets`).

### Release assets

Assets are downloaded to a temporary `.<file>.part-<pid>-<n>` next to their destination
and renamed into place only when complete, so an interrupted run never leaves a
truncated asset.  Each download is checked against the size GitHub reports and,
when GitHub provides one, its SHA-256 digest;
`release_checksums/<tag>/<file>.sha256` records the checksum
(`<sha256>  <file>`), in a directory of its own so that it never collides
with an asset of that name.  On the next run an asset is skipped only if it
is complete: its size equals GitHub's and its content matches the digest (or,
when GitHub has none, the sidecar).  Otherwise it is downloaded again.  See
[Releases](../releases.md) for how sidecars of earlier versions are moved.

## Clones Follow GitHub

A mirror clone is updated with `git fetch --all --no-prune --no-prune-tags`.
A branch or tag that is deleted on GitHub **stays** in the backup (whatever
`fetch.prune` says in your git configuration), so a deleted branch can still be
recovered; a force-pushed branch is overwritten, and the commits it used to
point at stay in the repository's object store only until `git gc` removes
them.  Pass `--prune` (config key `prune`) to delete refs that were deleted on
GitHub instead, which makes each clone an exact copy of GitHub's current refs.
Snapshot the output directory with a tool such as restic, borg or ZFS if you
need the history of force-pushed branches.

One case needs pruning to continue: if a deleted branch `foo` is replaced by a
new branch `foo/bar`, git cannot hold both refs.  The update then prunes once,
logs a warning, and carries on, so that repository keeps being backed up (the
stale refs are lost in that case).

## Design Rationale

- **`git/` and `json/` are siblings** — git clones and JSON metadata are clearly separated.
- **Owner subdirectory** — supports backing up multiple users/orgs into a single `--output` root.
- **Stable paths** — file names are deterministic: the same repo always maps to the same path, enabling incremental updates.
- **Standard git layout** — `*.git/` directories are valid bare git repos that any git tool can read directly.
