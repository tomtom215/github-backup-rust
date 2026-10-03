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

**`--issues`** saves a JSON array for every repository, each object exactly as
GitHub returns it (the excerpt below shows only a few of its ~40 properties):
- Issue number, title, body, state (`open`/`closed`), `state_reason`, `locked`
- Labels, assignees, milestone
- Created/updated/closed timestamps
- Author (full user object), `author_association`, `reactions`
- URL and HTML URL

GitHub's issues API lists **pull requests as issues too**; they are recognisable
by their `pull_request` property.  Issues and pull requests share one number
space.

**`--issue-comments`** saves, for every issue **and every pull request**, an array of
its comments (body, author, timestamps).  For a pull request this is the
conversation thread under the description; inline review comments are
`--pull-comments`.

**`--issue-events`** saves, for every issue and pull request, its events from
`/issues/<n>/events`: closed/reopened, labeled/unlabeled, assigned, milestoned,
renamed, locked, merged, ...  (GitHub's separate `/timeline`, with
cross-references and commits, is not fetched.)

### Output structure

```
json/repos/<repo>/
├── issues.json              ← array of issue objects (pull requests included)
├── issue_comments/<n>.json  ← comments of issue or pull request <n>
└── issue_events/<n>.json    ← events of issue or pull request <n>
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

**`--pulls`** saves the pull request list objects exactly as GitHub returns them:
- Number, title, body, state (`open`/`closed`; a merged PR is `closed` with a non-null `merged_at`)
- Head and base branch / commit SHA, `_links`
- Assignees, labels, milestone, requested reviewers, `auto_merge`, `draft`
- `merge_commit_sha`

The list endpoint does not include merge statistics (`merged`, `commits`,
`additions`, `deletions`, `changed_files`); those exist only on the
single-pull-request endpoint, which is not fetched, and they are not
written.

**`--pull-comments`** saves review comments (inline code comments attached to specific lines, with their diff hunk and reply threading).

**`--pull-commits`** saves the list of commits included in each PR (GitHub returns at most 250).

**`--pull-reviews`** saves review decisions (approved, changes requested, dismissed) and review bodies.

The conversation thread and the events of a pull request are written by
`--issue-comments` and `--issue-events` (see above).

### Output structure

```
json/repos/<repo>/
├── pulls.json               ← array of PR objects
├── pull_comments/<n>.json   ← review comments of pull request <n>
├── pull_commits/<n>.json    ← commits of pull request <n>
└── pull_reviews/<n>.json    ← reviews of pull request <n>
```

### API calls per repository

| Flag | Extra API calls |
|------|----------------|
| `--pulls` | 1 paginated list call |
| `--pull-comments` | 1 per PR |
| `--pull-commits` | 1 per PR |
| `--pull-reviews` | 1 per PR |
| `--issue-comments`, `--issue-events` | 1 per issue **and PR** each |

For repositories with many PRs, `--pull-commits` and `--pull-reviews` generate more API traffic.  Consider rate limit budgets for large organisations.
