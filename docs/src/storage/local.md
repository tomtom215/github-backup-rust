# Local Filesystem Storage

By default `github-backup` writes all artefacts to the local filesystem.  The root output directory is set with `--output` (or `output` in the config file).

## Output Directory Layout

```
<output>/
└── <owner>/
    ├── git/
    │   ├── repos/
    │   │   ├── <repo-name>.git/        ← bare mirror clone
    │   │   └── …
    │   ├── wikis/
    │   │   ├── <repo-name>.wiki.git/   ← wiki mirror clone
    │   │   └── …
    │   └── gists/
    │       ├── <gist-id>.git/          ← gist clone
    │       └── …
    └── json/
        ├── starred.json
        ├── watched.json
        ├── followers.json
        ├── following.json
        ├── gists/
        │   ├── <gist-id>.json
        │   └── …
        └── repos/
            └── <repo-name>/
                ├── issues.json
                ├── issue_comments.json
                ├── issue_events.json
                ├── pulls.json
                ├── pull_comments.json
                ├── pull_commits.json
                ├── pull_reviews.json
                ├── releases.json
                ├── releases/
                │   └── <tag>/
                │       └── <asset-file>
                ├── labels.json
                ├── milestones.json
                ├── hooks.json
                └── security_advisories.json
```

## Incremental Updates

`github-backup` is designed to be run repeatedly.  On subsequent runs:

- **Git repositories**: `git remote update --prune` (or `git fetch --all --prune`) updates existing clones in-place.
- **JSON files**: Overwritten with the latest data from the API.
- **Release assets**: Skipped if the file already exists and is non-empty.

## Disk Space Estimates

| Content type | Typical size |
|-------------|-------------|
| Mirror clone (small repo, full history) | 1–100 MB |
| Mirror clone (large repo) | 100 MB – 10 GB |
| Issues JSON (1 000 issues) | ~ 5 MB |
| PR JSON (1 000 PRs) | ~ 10 MB |
| Release assets | Highly variable |

Run `du -sh <output>/<owner>` to measure actual usage.

## Permissions

The backup process needs:
- **Read/write** on the `--output` directory
- **Execute** permission on `git` (and `git-lfs` if using `--lfs`)

For automated (root-less) backups, create a dedicated system user:

```bash
sudo useradd -r -m -d /var/backup/github github-backup
sudo -u github-backup github-backup octocat --token $GITHUB_TOKEN --output /var/backup/github --all
```

## Restoring from a Mirror Clone

A bare mirror clone is a fully valid git repository:

```bash
# Clone from the backup
git clone /var/backup/github/octocat/git/repos/Hello-World.git hello-world

# Or push it to a new remote
git -C /var/backup/github/octocat/git/repos/Hello-World.git \
  push --mirror https://github.com/neworg/Hello-World.git
```
