# Restore Guide

This guide covers all restore scenarios: automated restore via `--restore`,
manual git/JSON procedures, and decrypting S3-encrypted archives.

---

## Automated Restore (`--restore`)

The `--restore` flag re-creates **labels**, **milestones**, and **issues** from
the JSON backup into a target GitHub organisation via the GitHub REST API.

### Basic usage

```bash
github-backup octocat \
  --token ghp_your_write_token \
  --output /var/backup/github \
  --restore \
  --restore-target-org new-org
```

By default this prints a warning banner and prompts you to type `yes` before
writing any data.  In non-interactive environments (CI, scripts) pass
`--restore-yes` to skip the prompt:

```bash
github-backup octocat \
  --token ghp_write_token \
  --output /var/backup/github \
  --restore \
  --restore-target-org new-org \
  --restore-yes
```

### Dry run

Use `--dry-run` to see what *would* be restored without making any API calls.
All counters are reported as normal, but no GitHub data is created:

```bash
github-backup octocat \
  --token ghp_write_token \
  --output /var/backup/github \
  --restore \
  --restore-target-org new-org \
  --dry-run
```

### What is restored

| Artefact | Source JSON | GitHub API endpoint |
|----------|-------------|---------------------|
| Labels | `json/repos/<repo>/labels.json` | `POST /repos/{org}/{repo}/labels` |
| Milestones | `json/repos/<repo>/milestones.json` | `POST /repos/{org}/{repo}/milestones` |
| Issues | `json/repos/<repo>/issues.json` | `POST /repos/{org}/{repo}/issues` |

Pull requests, comments, reactions, and review threads are **not** restored;
see [Manual Procedures](#manual-restore-procedures) for alternatives.

### Behaviour

- **Additive only** — existing resources are never deleted or modified.
- **Idempotent** — re-running with the same backup is safe; HTTP 422 (already
  exists) responses are silently skipped and counted as "skipped".
- **Issues vs pull requests** — items whose `pull_request` field is set are
  skipped; only true issues are created.
- **Per-repository** — iterates over every directory under `json/repos/` and
  restores each independently.  A failure in one repository is logged but does
  not abort the rest.

### Token requirements

The token must have:

| Token type | Required scopes / permissions |
|------------|-------------------------------|
| Classic PAT | `repo` |
| Fine-grained PAT | `contents: write`, `issues: write` |

The token must have push access (or owner access) to the target organisation.

---

## Decrypting S3-Encrypted Backups

If you used `--encrypt-key` (AES-256-GCM) when syncing to S3, decrypt
individual files before reading them:

```bash
# Using the --decrypt subcommand
github-backup \
  --encrypt-key "$BACKUP_ENCRYPT_KEY" \
  --decrypt \
  --decrypt-input issues.json.enc \
  --decrypt-output issues.json
```

Or from a shell script using OpenSSL:

```bash
key_hex="$BACKUP_ENCRYPT_KEY"   # 64 hex characters

# Split nonce (first 12 bytes) and ciphertext+tag
dd if=issues.json.enc bs=12 count=1 of=nonce.bin 2>/dev/null
dd if=issues.json.enc bs=12 skip=1 of=ct.bin 2>/dev/null

openssl enc -d -aes-256-gcm \
  -K "$key_hex" \
  -iv "$(xxd -p nonce.bin)" \
  -in ct.bin \
  -out issues.json
```

The wire format is `[12-byte random nonce][ciphertext + 16-byte GCM tag]`.

---

## Manual Restore Procedures

### Repository Git Data

```bash
# The backup is a bare/mirror repo; push directly to a new remote
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
  --title "v1.0.0" --notes "Restored from backup" \
  --repo new-org/my-repo
```

### Manual Labels via `curl`

If you prefer scripting over `--restore`:

```bash
jq -c '.[]' /backup/octocat/json/repos/my-repo/labels.json | \
while read -r label; do
  name=$(jq -r '.name'        <<< "$label")
  color=$(jq -r '.color'      <<< "$label")
  desc=$(jq -r '.description // ""' <<< "$label")
  curl -s -X POST \
    -H "Authorization: Bearer $GITHUB_TOKEN" \
    -H "Content-Type: application/json" \
    -d "{\"name\":\"$name\",\"color\":\"$color\",\"description\":\"$desc\"}" \
    "https://api.github.com/repos/new-org/my-repo/labels"
done
```

### Branch Protection Rules

`branch_protections.json` is an object mapping branch name → protection rules.
Re-apply using the GitHub API:

```bash
# Re-apply protection for 'main'
jq '.main' /backup/octocat/json/repos/my-repo/branch_protections.json | \
curl -s -X PUT \
  -H "Authorization: Bearer $GITHUB_TOKEN" \
  -H "Content-Type: application/json" \
  -H "Accept: application/vnd.github+json" \
  -d @- \
  "https://api.github.com/repos/new-org/my-repo/branches/main/protection"
```

### Deploy Keys and Collaborators

`deploy_keys.json` and `collaborators.json` are informational archives.  The
public key material in `deploy_keys.json` can be re-added via the GitHub API.
Collaborator entries record access levels but restoring them requires
re-inviting each user.

### Starred, Followed, and Organisation Data

These JSON files are reference archives.  No automated restoration path
exists — they are useful for auditing and for manually reconstructing the
social graph after account migration.

---

## Backup Layout Reference

```
<output>/
└── <owner>/
    ├── git/
    │   ├── repos/                 # bare mirror git repos
    │   │   └── <repo>.git/
    │   ├── wikis/                 # wiki repos
    │   │   └── <repo>.wiki.git/
    │   └── gists/                 # gist repos
    │       └── <gist-id>.git/
    ├── json/
    │   ├── repos.json
    │   ├── starred.json
    │   ├── watched.json
    │   ├── followers.json
    │   ├── following.json
    │   ├── org_members.json       (org targets only)
    │   ├── org_teams.json         (org targets only)
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
    │           ├── branch_protections.json  (protected branches; admin token)
    │           ├── hooks.json              (admin token required)
    │           ├── security_advisories.json
    │           ├── deploy_keys.json        (admin token required)
    │           ├── collaborators.json      (admin token required)
    │           └── issues/ pulls/          (comment/event/review sub-directories)
    └── assets/
        └── repos/
            └── <repo>/
                └── releases/
                    └── <tag>/              # downloaded release asset binaries
```
