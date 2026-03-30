# Restoring from a Backup

`github-backup` creates self-contained, standard-format archives.

---

## Automated Restore (`--restore`)

The `--restore` flag re-creates **labels** and **milestones** from the JSON backup
into a target GitHub organisation via the GitHub REST API.

```bash
github-backup octocat \
  --token ghp_your_write_token \
  --output /var/backup/github \
  --restore \
  --restore-target-org new-org
```

### What is restored

| Artefact | Source JSON | GitHub API endpoint |
|----------|-------------|---------------------|
| Labels | `json/repos/<repo>/labels.json` | `POST /repos/{org}/{repo}/labels` |
| Milestones | `json/repos/<repo>/milestones.json` | `POST /repos/{org}/{repo}/milestones` |

### Behaviour

- **Additive only** — existing resources are never deleted or modified.
- **Idempotent** — re-running with the same backup is safe; duplicate resources
  (HTTP 422) are silently skipped.
- **Per-repository** — iterates over every repository directory under
  `json/repos/` and restores each one independently.  A failure in one repository
  is logged but does not abort the rest.

### Token requirements

The token must have the `repo` scope (classic PAT) or `contents: write` +
`issues: write` permissions (fine-grained PAT) on the target organisation's
repositories.

### Issues and pull requests

GitHub does not expose a public bulk-import REST API for issues or pull
requests.  Options for migrating issue data:

- **GitHub CLI** — `gh issue import` (GitHub Enterprise only).
- **GitHub Enterprise Migrations API** — for large-scale migrations between
  organisations or instances.
- **Third-party tools** — e.g.,
  [`github-importer`](https://github.com/nicowillis/github-importer) or
  [`ghec-importer`](https://github.com/github/ghec-importer).

---

## Manual Restore Procedures

### Repository Git Data

```bash
# The backup is already a bare repo; push it to a new remote
git -C /backup/octocat/git/repos/my-repo.git \
    push --mirror https://github.com/new-org/my-repo.git
```

```bash
# Full clone from backup, then re-point the remote
git clone /backup/octocat/git/repos/my-repo.git /tmp/my-repo
git -C /tmp/my-repo remote set-url origin https://github.com/new-org/my-repo.git
git -C /tmp/my-repo push --all
git -C /tmp/my-repo push --tags
```

### Wikis

```bash
git -C /backup/octocat/git/wikis/my-repo.wiki.git \
    push --mirror https://github.com/new-org/my-repo.wiki.git
```

### Gists

Gist git data lives in `git/gists/<gist-id>.git/`.  Metadata (description, file
names, visibility) is in `json/gists/<gist-id>.json`.

```bash
# Push git content to a newly created gist
git -C /backup/octocat/git/gists/abc123.git \
    push --mirror https://gist.github.com/new-gist-id.git
```

### Releases and Assets

```bash
gh release create v1.0.0 \
  /backup/octocat/assets/repos/my-repo/releases/v1.0.0/my-binary \
  --title "v1.0.0" --notes "Restored from backup"
```

### Manual Labels and Milestones via `curl`

If you prefer shell scripting over `--restore`:

```bash
# Re-create labels
jq -c '.[]' /backup/octocat/json/repos/my-repo/labels.json | while read -r label; do
  name=$(echo "$label" | jq -r '.name')
  color=$(echo "$label" | jq -r '.color')
  desc=$(echo "$label" | jq -r '.description // ""')
  curl -s -X POST \
    -H "Authorization: Bearer $GITHUB_TOKEN" \
    -H "Content-Type: application/json" \
    -d "{\"name\":\"$name\",\"color\":\"$color\",\"description\":\"$desc\"}" \
    "https://api.github.com/repos/new-org/my-repo/labels"
done
```

### Deploy Keys and Collaborators

`deploy_keys.json` and `collaborators.json` are informational archives.  Public
key material in `deploy_keys.json` can be re-added via the GitHub API.
Collaborator entries record access levels but restoring them requires
re-inviting each user.

### Starred, Followed, and Organisation Data

These JSON files are reference archives.  No automated restoration path exists —
they are useful for auditing and for manually reconstructing social graph data
after account migration.

---

## Backup Layout Reference

```
<output>/
└── <owner>/
    ├── git/
    │   ├── repos/          # bare mirror git repos
    │   ├── wikis/          # wiki repos
    │   └── gists/          # gist repos
    ├── json/
    │   ├── starred.json
    │   ├── watched.json
    │   ├── followers.json
    │   ├── following.json
    │   ├── org_members.json    (org targets only)
    │   ├── org_teams.json      (org targets only)
    │   ├── gists/
    │   │   └── <gist-id>.json
    │   └── repos/
    │       └── <repo>/
    │           ├── issues.json
    │           ├── pulls.json
    │           ├── releases.json
    │           ├── labels.json
    │           ├── milestones.json
    │           ├── topics.json
    │           ├── branches.json
    │           ├── hooks.json
    │           ├── security_advisories.json
    │           ├── deploy_keys.json
    │           ├── collaborators.json
    │           └── issues/ pulls/  (sub-directories for comments/events/reviews)
    └── assets/
        └── repos/
            └── <repo>/
                └── releases/
                    └── <tag>/   # downloaded release asset binaries
```
