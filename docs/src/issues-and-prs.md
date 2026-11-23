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

When Issues are switched off for a repository, GitHub answers the issue
listing with `410 Gone`: there is no `issues.json`, which is not a failure,
and the conversation and events of every pull request are still fetched.

`issues.json` and every per-issue file **accumulate**: they are merged with
what earlier runs stored (issues by `number`, comments and events by `id`),
so an issue or comment deleted on GitHub stays in the backup.  A later run
lists only the issues updated since the repository's watermark
(`GET .../issues?state=all&since=`) and merges them in, provided
`issues.json` is intact; the issue count in the summary is then that of the
merged file.  An issue that
the tool's model cannot read is still written as received and still gets its
comments and events by its `number`; an element without a `number` is
reported as a failure.  An issue whose comments or events cannot be fetched is
reported and the remaining issues are still backed up.

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

**`--pull-commits`** saves the list of commits included in each PR.  GitHub's
list stops at 250 commits; when it returns 250, the commits come from the
comparison of the PR's base and head commits instead (paginated, no limit), and
if that fails the PR is reported as a failure rather than stored truncated.

**`--pull-reviews`** saves review decisions (approved, changes requested, dismissed) and review bodies.

The conversation thread and the events of a pull request are written by
`--issue-comments` and `--issue-events` (see above).

Like the issue files, `pulls.json` and the per-PR files accumulate (pull
requests by `number`, comments and reviews by `id`, commits by `sha`, so a
commit force-pushed away stays listed).

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
| `--pulls` | 1 paginated list call (1 request per 100 pull requests; on a later run, per 100 updated since the watermark) |
| `--pull-comments` | 1 per PR |
| `--pull-commits` | 1 per PR |
| `--pull-reviews` | 1 per PR |
| `--issue-comments`, `--issue-events` | 1 per issue **and PR** each |
| `--pull-commits` on a PR with 250+ commits | 1 per 100 commits (comparison) |
| `--issue-comments` / `--pull-comments` on a later run | 1 paginated list of the comments created or edited since the watermark |

The per-item calls are made on the first run and, on later runs, only for issues and pull requests that changed since the repository's watermark, or that have a conversation comment (`GET .../issues/comments?since=`) or review comment (`GET .../pulls/comments?since=`) created or edited since then (editing a comment does not change its issue's `updated_at`); if that list cannot be fetched, the failure is reported and every item's files are fetched.  Edits to a review's own text are not detected this way (see [Incremental runs](monitoring.md#incremental-runs-and-the-state-file); `--full` repeats all of them).  The listing calls cost one request per 100 items on the first run (and with
`--full`).  On later runs they ask only for what was updated since the
watermark: issues with the `since` filter, pull requests sorted by
`updated_at`, newest first, until the first page that reaches older ones
(GitHub's pull request listing has no `since`), each merged into the stored
file; a missing or damaged `issues.json` or `pulls.json` is listed in full
again.  See [Requests per repository](monitoring.md#requests-per-repository-on-a-repeat-run).  Consider the rate-limit budget (5 000 requests per hour per token) for large organisations; [Rate limit](ops-runbook.md#rate-limit) explains what happens when it is exhausted.
