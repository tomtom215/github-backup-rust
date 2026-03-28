# Gists & Wikis

## Gists

GitHub Gists are small snippets or files hosted on `gist.github.com`.  Each gist has its own git repository.

### Backup Owned Gists

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup --gists
```

Clones all gists owned by `octocat` as bare mirror repositories:

```
git/gists/<gist-id>.git/
json/gists/<gist-id>.json
```

The JSON file contains the gist metadata:
- Gist ID, description, visibility (public/secret)
- List of files (filename, language, size)
- Owner, created/updated timestamps
- Fork and star counts
- Git URLs (HTTPS and SSH)

### Backup Starred Gists

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup --starred-gists
```

Backs up all gists starred by the **authenticated user** (not necessarily `octocat`).  This requires the `gist` OAuth scope.

### Combined

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup \
  --gists --starred-gists
```

### Gist update behaviour

On subsequent runs, gists are updated in-place with `git remote update --prune` (or without `--prune` if `--no-prune` is set), matching the mirror clone behaviour for repositories.

---

## Wikis

GitHub repository wikis are stored as separate git repositories (the `<repo>.wiki.git` URL).

### Backup Wikis

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup \
  --repositories --wikis
```

`--wikis` runs independently of `--repositories` — you do not need to enable `--repositories` to back up wikis, but both flags are commonly used together.

### Output

```
git/wikis/<repo>.wiki.git/
```

Each wiki is cloned as a bare mirror.  The repository contains all wiki pages as Markdown files, plus the full commit history.

### Notes

- Repositories that have no wiki will be skipped silently.
- A wiki must be initialised (have at least one page) before it can be cloned.
- Private repository wikis require a token with the `repo` scope.
