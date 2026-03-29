# Restoring from a Backup

`github-backup` creates self-contained, standard-format archives.  Restoration does not require the tool itself — each artefact can be used directly with `git`, standard JSON tooling, or the GitHub API.

---

## Repository Git Data

### Mirror / Bare Clone

```bash
# The backup is already a bare repo; push it to a new remote
git -C /backup/octocat/git/repos/my-repo.git \
    push --mirror https://github.com/new-org/my-repo.git
```

### Full Clone from Backup

```bash
git clone /backup/octocat/git/repos/my-repo.git /tmp/my-repo
```

The cloned repo will have `origin` pointing at the backup path.  Update the remote after cloning:

```bash
git -C /tmp/my-repo remote set-url origin https://github.com/new-org/my-repo.git
git -C /tmp/my-repo push --all
git -C /tmp/my-repo push --tags
```

---

## Wikis

Wiki backups are bare mirror clones, identical in layout to repository backups:

```bash
git -C /backup/octocat/git/wikis/my-repo.wiki.git \
    push --mirror https://github.com/new-org/my-repo.wiki.git
```

---

## Gists

Gist git data lives in `git/gists/<gist-id>.git/`.  Metadata (description, file names, visibility) is in `json/gists/<gist-id>.json`.

Restore the git content first, then use the GitHub API or the web UI to set the description and file names.

```bash
# Push git content to a newly created gist
git -C /backup/octocat/git/gists/abc123.git \
    push --mirror https://gist.github.com/new-gist-id.git
```

---

## Issues and Pull Requests

Issue and PR data is stored as JSON.  It is informational — GitHub does not provide a public API for bulk import.  Third-party tools such as [github-importer](https://github.com/nicowillis/github-importer) or the GitHub Enterprise Migrations API can help reconstruct issues from JSON.

```
json/repos/<repo>/issues.json
json/repos/<repo>/issues/<number>/comments.json
json/repos/<repo>/pulls.json
json/repos/<repo>/pulls/<number>/comments.json
json/repos/<repo>/pulls/<number>/commits.json
json/repos/<repo>/pulls/<number>/reviews.json
```

Inspect the data with `jq`:

```bash
# List open issues
jq '.[] | select(.state == "open") | {number, title}' \
  /backup/octocat/json/repos/my-repo/issues.json
```

---

## Releases and Assets

Release metadata is in `json/repos/<repo>/releases.json`.  Binary assets are in `assets/repos/<repo>/releases/<tag>/`.

Recreate a release via the GitHub CLI:

```bash
# Create a new release from backup data (manual steps)
gh release create v1.0.0 \
  /backup/octocat/assets/repos/my-repo/releases/v1.0.0/my-binary \
  --title "v1.0.0" --notes "Restored from backup"
```

---

## Labels and Milestones

Restore labels and milestones via the GitHub REST API using the backed-up JSON:

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

---

## Deploy Keys and Collaborators

`deploy_keys.json` and `collaborators.json` are informational archives.  Public key material in `deploy_keys.json` can be re-added via the GitHub API.  Collaborator entries record access levels but restoring them requires re-inviting each user.

---

## Starred, Followed, and Organisation Data

These JSON files are reference archives.  No automated restoration path exists — they are useful for auditing and for manually reconstructing social graph data after account migration.

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
