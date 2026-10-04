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
the list of topic names; `branch_protections.json` is an object keyed by
branch name.

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
        ├── package_versions_<type>_<name>.json
        ├── starred_clone_queue.json        ← --clone-starred progress
        ├── backup_state.json               ← incremental watermarks per repository
        ├── backup_history.json             ← recent runs (--history-size)
        ├── backup_checkpoint.json          ← only after an interrupted run
        ├── backup_manifest.json            ← --manifest
        ├── .backup.lock                    ← engine lock (stays on disk, see below)
        ├── gists/
        │   ├── <gist-id>.json              ← --gists
        │   ├── <gist-id>.starred.json      ← --starred-gists
        │   ├── index.json                  ← all owned gists
        │   └── starred_index.json          ← all starred gists
        └── repos/
            └── <repo>/
                ├── info.json               ← the repository object (always written)
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
                │       ├── <asset-file>        ← binary release asset
                │       └── <asset-file>.sha256 ← its checksum
                ├── labels.json
                ├── milestones.json
                ├── hooks.json
                ├── security_advisories.json
                ├── topics.json
                ├── branches.json
                ├── branch_protections.json ← protected branches (needs admin access)
                ├── deploy_keys.json
                ├── collaborators.json
                ├── workflows.json
                ├── workflow_runs_<id>.json ← one per workflow (--action-runs)
                └── environments.json
```

`discussions.json`, `discussion_comments_<n>.json`, `projects.json` and
`project_columns_<id>.json` are **not** produced: GitHub's REST API has no such
endpoints, see [Discussions and Classic Projects](../backup-categories.md#discussions).
Repositories that the options exclude (forks without `--forks`, private
repositories without `--private`, `--include-repos` / `--exclude-repos`) get
neither a clone nor a metadata directory.

### Bookkeeping files

| File | Meaning |
|------|---------|
| `repos.json` | The repositories this backup covers, as GitHub listed them: only the ones the options include, merged with earlier listings so that a repository deleted on GitHub stays recorded.  `--diff-with` compares it between two backups. |
| `backup_state.json` | One watermark per repository (`repos.<owner/repo>.at`, plus the categories it covers) and `last_successful_run`.  See [Incremental runs](../monitoring.md#incremental-runs-and-the-state-file).  Safe to delete: the next run fetches everything once. |
| `backup_history.json` | One entry per run (time, repositories backed up, duration, `success`, `failures`, tool version), newest last, at most `--history-size` entries. |
| `backup_checkpoint.json` | Which repositories an **interrupted** run had finished.  Removed when a run completes.  A later run resumes from it only if it is less than 6 hours old; an older one is ignored. |
| `backup_manifest.json` | `--manifest`: SHA-256 of every data file under `json/`, except the history, state, checkpoint and lock files (the git clones are not included). |
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
| `gists/<id>.json`, `gists/index.json` | `--gists` |
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
| `repos/<name>/release_assets/<tag>/<file>` and `<file>.sha256` | `--release-assets` |
| `repos/<name>/labels.json` | `--labels` |
| `repos/<name>/milestones.json` | `--milestones` |
| `repos/<name>/hooks.json` | `--hooks` |
| `repos/<name>/security_advisories.json` | `--security-advisories` |
| `repos/<name>/topics.json` | `--topics` |
| `repos/<name>/branches.json`, `branch_protections.json` | `--branches` |
| `repos/<name>/deploy_keys.json` | `--deploy-keys` |
| `repos/<name>/collaborators.json` | `--collaborators` |
| `repos/<name>/workflows.json` | `--actions` |
| `repos/<name>/workflow_runs_<id>.json` | `--action-runs` |
| `repos/<name>/environments.json` | `--environments` |

### Release assets

Assets are downloaded to a temporary `.<file>.part` next to their destination
and renamed into place only when complete, so an interrupted run never leaves a
truncated asset.  Each download is checked against the size GitHub reports and,
when GitHub provides one, its SHA-256 digest; `<file>.sha256` records the
checksum (`<sha256>  <file>`).  On the next run an asset is skipped only if it
is complete: its size equals GitHub's and its content matches the digest (or,
when GitHub has none, the sidecar).  Otherwise it is downloaded again.

## Clones Follow GitHub

A mirror clone is updated with `git fetch --all --prune`, so a branch or tag
that is deleted on GitHub is deleted from the backup on the next run, and a
force-pushed branch is overwritten.  The backup is a faithful copy of the
current state, not an archive of every state; objects that become unreachable
stay in the repository's object store until `git gc` removes them.  Use
`--no-prune` to keep deleted refs (force-pushed refs are still updated), and
snapshot the output directory with a tool such as restic, borg or ZFS if you
need history.

## Design Rationale

- **`git/` and `json/` are siblings** — git clones and JSON metadata are clearly separated.
- **Owner subdirectory** — supports backing up multiple users/orgs into a single `--output` root.
- **Stable paths** — file names are deterministic: the same repo always maps to the same path, enabling incremental updates.
- **Standard git layout** — `*.git/` directories are valid bare git repos that any git tool can read directly.
