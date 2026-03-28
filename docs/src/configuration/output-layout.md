# Output Directory Layout

This page explains the directory and file structure that `github-backup` creates under `--output`.

## Tree

```
<output>/
└── <owner>/                            ← GitHub username or org name
    ├── git/                            ← Git repositories
    │   ├── repos/
    │   │   ├── <repo>.git/             ← bare mirror clone
    │   │   └── …
    │   ├── wikis/
    │   │   ├── <repo>.wiki.git/        ← wiki mirror clone
    │   │   └── …
    │   └── gists/
    │       ├── <gist-id>.git/          ← gist clone
    │       └── …
    └── json/                           ← JSON metadata
        ├── starred.json                ← starred repositories
        ├── watched.json                ← watched repositories
        ├── followers.json              ← follower list
        ├── following.json              ← following list
        ├── gists/
        │   ├── <gist-id>.json          ← gist metadata
        │   └── …
        └── repos/
            └── <repo>/
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
                │       └── <asset-file>  ← binary release asset
                ├── labels.json
                ├── milestones.json
                ├── hooks.json
                └── security_advisories.json
```

## File Descriptions

### Git repositories (`git/`)

| Path pattern | Clone command | Contents |
|-------------|---------------|----------|
| `git/repos/<repo>.git/` | `git clone --mirror` | All refs, all history (default) |
| `git/wikis/<repo>.wiki.git/` | `git clone --mirror` | Wiki pages as Markdown |
| `git/gists/<gist-id>.git/` | `git clone --mirror` | Gist file history |

### JSON metadata (`json/`)

| File | Enabled by |
|------|-----------|
| `starred.json` | `--starred` |
| `watched.json` | `--watched` |
| `followers.json` | `--followers` |
| `following.json` | `--following` |
| `gists/<id>.json` | `--gists` or `--starred-gists` |
| `repos/<name>/issues.json` | `--issues` |
| `repos/<name>/issue_comments.json` | `--issue-comments` |
| `repos/<name>/issue_events.json` | `--issue-events` |
| `repos/<name>/pulls.json` | `--pulls` |
| `repos/<name>/pull_comments.json` | `--pull-comments` |
| `repos/<name>/pull_commits.json` | `--pull-commits` |
| `repos/<name>/pull_reviews.json` | `--pull-reviews` |
| `repos/<name>/releases.json` | `--releases` |
| `repos/<name>/releases/<tag>/<file>` | `--release-assets` |
| `repos/<name>/labels.json` | `--labels` |
| `repos/<name>/milestones.json` | `--milestones` |
| `repos/<name>/hooks.json` | `--hooks` |
| `repos/<name>/security_advisories.json` | `--security-advisories` |

## Design Rationale

- **`git/` and `json/` are siblings** — git clones and JSON metadata are clearly separated.
- **Owner subdirectory** — supports backing up multiple users/orgs into a single `--output` root.
- **Stable paths** — file names are deterministic: the same repo always maps to the same path, enabling incremental updates.
- **Standard git layout** — `*.git/` directories are valid bare git repos that any git tool can read directly.
