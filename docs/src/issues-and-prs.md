# Issues & Pull Requests

## Issues

Enable issue backup with one or more of:

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup \
  --repositories \
  --issues \
  --issue-comments \
  --issue-events
```

### What is backed up

**`--issues`** saves a JSON array for every repository:
- Issue number, title, body, state (`open`/`closed`)
- Labels, assignees, milestone
- Created/updated/closed timestamps
- Author (login, avatar URL)
- URL and HTML URL

**`--issue-comments`** saves a separate array of all comment bodies, authors, and timestamps.

**`--issue-events`** saves the timeline — labels added/removed, assignments, cross-references, etc.

### Output structure

```
json/repos/<repo>/
├── issues.json          ← array of issue objects
├── issue_comments.json  ← array of comment objects
└── issue_events.json    ← array of event objects
```

### JSON schema excerpt

```json
[
  {
    "number": 1,
    "title": "Found a bug",
    "state": "closed",
    "body": "I found a bug...",
    "user": { "login": "octocat" },
    "labels": [{ "name": "bug" }],
    "created_at": "2022-01-01T00:00:00Z",
    "closed_at": "2022-01-02T00:00:00Z"
  }
]
```

---

## Pull Requests

Enable PR backup with one or more of:

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup \
  --repositories \
  --pulls \
  --pull-comments \
  --pull-commits \
  --pull-reviews
```

### What is backed up

**`--pulls`** saves PR metadata:
- Number, title, body, state (`open`/`closed`/`merged`)
- Head and base branch / commit SHA
- Assignees, labels, milestone, requested reviewers
- Merge commit SHA (if merged)

**`--pull-comments`** saves review comments (inline code comments attached to specific lines).

**`--pull-commits`** saves the list of commits included in each PR.

**`--pull-reviews`** saves review decisions (approved, changes requested, dismissed) and review bodies.

### Output structure

```
json/repos/<repo>/
├── pulls.json           ← array of PR objects
├── pull_comments.json   ← array of review comment objects
├── pull_commits.json    ← array of commit list objects
└── pull_reviews.json    ← array of review objects
```

### API calls per repository

| Flag | Extra API calls |
|------|----------------|
| `--pulls` | 1 paginated list call |
| `--pull-comments` | 1 per repository |
| `--pull-commits` | 1 per PR |
| `--pull-reviews` | 1 per PR |

For repositories with many PRs, `--pull-commits` and `--pull-reviews` generate more API traffic.  Consider rate limit budgets for large organisations.
