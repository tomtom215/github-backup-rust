# Backup Categories

`github-backup` organises backup targets into distinct categories.  Each category can be enabled individually with a flag, or all can be enabled at once with `--all`.

## Repositories

| Flag | Description |
|------|-------------|
| `--repositories` | Clone all repositories for the owner |
| `--forks` | Include forked repositories |
| `--private` | Include private repositories (requires `repo` scope) |
| `--prefer-ssh` | Use SSH URLs instead of HTTPS for cloning |
| `--clone-type` | Clone mode: `mirror` (default), `bare`, `full`, `shallow:<n>` |
| `--lfs` | Enable Git LFS support |
| `--no-prune` | Skip pruning deleted remote refs on update |

### Clone Types Explained

| Type | Command | Use Case |
|------|---------|----------|
| `mirror` (default) | `git clone --mirror` | Complete backup: all refs, all branches, full history |
| `bare` | `git clone --bare` | Bare repo without remote-tracking refs; slightly smaller |
| `full` | `git clone` | Working-tree clone; use to browse/build source |
| `shallow:<n>` | `git clone --depth <n>` | Limited history; saves disk space; not for archival |

Example:
```bash
# Mirror clone (default) — recommended
github-backup octocat --token $GITHUB_TOKEN --output /backup --repositories

# Shallow clone, last 5 commits only
github-backup octocat --token $GITHUB_TOKEN --output /backup \
  --repositories --clone-type shallow:5
```

Output: `<output>/<owner>/git/repos/<repo-name>.git/`

---

## Issues

| Flag | Description |
|------|-------------|
| `--issues` | Issue metadata (title, body, state, labels, assignees) |
| `--issue-comments` | Issue comment threads |
| `--issue-events` | Issue timeline events (e.g. label, assign, close events) |

Output: `<output>/<owner>/json/repos/<repo>/issues.json`

---

## Pull Requests

| Flag | Description |
|------|-------------|
| `--pulls` | PR metadata (title, body, state, head/base refs) |
| `--pull-comments` | Review comments on PRs |
| `--pull-commits` | List of commits in each PR |
| `--pull-reviews` | PR reviews (approve/request changes/comment) |

Output: `<output>/<owner>/json/repos/<repo>/pulls.json`

---

## Releases

| Flag | Description |
|------|-------------|
| `--releases` | Release metadata (tag, title, body, assets list) |
| `--release-assets` | Download binary release assets (requires `--releases`) |

> **Warning**: `--release-assets` can consume significant disk space for projects with large binary releases.

Output: `<output>/<owner>/json/repos/<repo>/releases.json`

---

## Wikis

| Flag | Description |
|------|-------------|
| `--wikis` | Clone repository wikis as bare mirror repos |

Output: `<output>/<owner>/git/wikis/<repo>.wiki.git/`

---

## Repository Metadata

| Flag | Description |
|------|-------------|
| `--labels` | Repository label definitions |
| `--milestones` | Repository milestones |
| `--hooks` | Webhook configurations (requires admin access) |
| `--security-advisories` | Published security advisories |
| `--topics` | Repository topics (tags) |
| `--branches` | Branch list with tip SHAs and protection status |
| `--deploy-keys` | Deploy keys attached to the repository (requires admin access) |
| `--collaborators` | Collaborator list with permissions (requires admin access) |

Output: `<output>/<owner>/json/repos/<repo>/labels.json`, `milestones.json`, `topics.json`, `branches.json`, `deploy_keys.json`, `collaborators.json`, etc.

> **Note**: `--hooks`, `--deploy-keys`, and `--collaborators` all require admin access to the repository.
> On repositories where the token lacks admin rights the tool logs a warning and continues rather than failing the entire backup.

---

## Gists

| Flag | Description |
|------|-------------|
| `--gists` | Clone gists owned by the backup target |
| `--starred-gists` | Clone gists starred by the authenticated user |

Output:
- Git: `<output>/<owner>/git/gists/<gist-id>.git/`
- Metadata: `<output>/<owner>/json/gists/<gist-id>.json`

---

## User / Organisation Data

| Flag | Description | Target |
|------|-------------|--------|
| `--starred` | Repositories starred by the owner | User & Org |
| `--watched` | Repositories watched by the owner | User & Org |
| `--followers` | Follower list | User & Org |
| `--following` | Following list | User & Org |
| `--org-members` | Organisation member list | **Org only** |
| `--org-teams` | Organisation team list | **Org only** |

Output: `<output>/<owner>/json/starred.json`, `watched.json`, `org_members.json`, `org_teams.json`, etc.

> **Note**: `--org-members` and `--org-teams` are silently skipped for user targets even if specified; they only apply when `--org` is passed.

---

## The `--all` Flag

`--all` enables every category above except:
- `--lfs` (requires git-lfs to be installed)
- `--prefer-ssh` (requires SSH keys to be set up)
- `--no-prune` (affects update behaviour)
- `--concurrency` (set separately)

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup --all
```
