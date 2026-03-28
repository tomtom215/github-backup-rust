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

## All User Data Together

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup \
  --starred --watched --followers --following
```

Or simply use `--all` to include everything.

---

## Organisation Notes

When using `--org`, the following flags behave differently:

| Flag | User target | Org target |
|------|-------------|-----------|
| `--starred` | User's starred repos | *Not applicable* |
| `--watched` | User's watched repos | *Not applicable* |
| `--followers` | User's followers | *Not applicable* |
| `--following` | User's following | *Not applicable* |

User-level data flags are silently skipped when `--org` is passed.
