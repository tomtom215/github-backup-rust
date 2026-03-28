# User & Organisation Data

These categories back up list-style metadata that is associated with the owner rather than individual repositories.

## Starred Repositories

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup --starred
```

Saves a JSON array of every repository the owner has starred:

```
json/starred.json
```

Each entry contains the full repository metadata: name, description, owner, visibility, star count, fork count, topics, and clone URLs.

## Watched Repositories

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup --watched
```

Saves repositories the owner is watching (not just the automatic watch on own repos):

```
json/watched.json
```

## Followers

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup --followers
```

Saves the list of accounts that follow the owner:

```
json/followers.json
```

## Following

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup --following
```

Saves the list of accounts the owner follows:

```
json/following.json
```

---

## Organisation Members

> **Organisation targets only** — this flag has no effect when backing up a regular user account.

```bash
github-backup my-org --org --token $GITHUB_TOKEN --output /backup --org-members
```

Saves the public member list of the organisation:

```
json/org_members.json
```

Each entry includes the member's login, ID, avatar URL, and profile URL.

## Organisation Teams

> **Organisation targets only** — this flag has no effect when backing up a regular user account.

```bash
github-backup my-org --org --token $GITHUB_TOKEN --output /backup --org-teams
```

Saves all teams within the organisation, including nested parent–child relationships:

```
json/org_teams.json
```

Each entry includes the team's name, slug, description, privacy setting, permission level, member and repository URLs, and an optional `parent` field for nested teams.

---

## All User Data Together

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup \
  --starred --watched --followers --following
```

For an organisation backup with all owner-level data:

```bash
github-backup my-org --org --token $GITHUB_TOKEN --output /backup \
  --starred --watched --followers --following \
  --org-members --org-teams
```

Or simply use `--all` to include everything.

---

## Organisation Notes

When using `--org`, all flags are available but some apply only to organisation targets:

| Flag | User target | Org target |
|------|-------------|-----------|
| `--starred` | User's starred repos | Org's starred repos |
| `--watched` | User's watched repos | Org's watched repos |
| `--followers` | User's followers | Org's followers |
| `--following` | User's following | Org's following |
| `--org-members` | *Silently skipped* | Organisation member list |
| `--org-teams` | *Silently skipped* | Organisation team list |
