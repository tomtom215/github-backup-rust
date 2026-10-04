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

The JSON file contains the gist object exactly as GitHub returns it:
- Gist ID, description, visibility (public/secret)
- List of files (filename, language, size)
- Owner, created/updated timestamps
- Comment count
- Git URLs (HTTPS and SSH)

`json/gists/index.json` lists all owned gists.

**Secret gists.**  GitHub's public gist listing for a user never contains
secret gists.  When the token belongs to the account being backed up (the login
is compared case-insensitively), `GET /gists` is used and secret gists are
included.  For any other user, without a token, or with a token that cannot
call `GET /user` (GitHub App tokens), only public gists are backed up.

### Backup Starred Gists

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup --starred-gists
```

Records the gists starred by the **authenticated user** (not necessarily `octocat`).  This requires the `gist` OAuth scope.

Only the gist **metadata** is saved (`json/gists/<gist-id>.starred.json` and
`json/gists/starred_index.json`); starred gists are **not cloned**, so their file
contents are not part of the backup.

### Combined

```bash
github-backup octocat --token $GITHUB_TOKEN --output /backup \
  --gists --starred-gists
```

### Gist update behaviour

On subsequent runs, gists are updated in place with `git fetch --all --prune` (without `--prune` if `--no-prune` is set), exactly like mirror clones of repositories.  Gists are always mirror clones; `--clone-type` does not apply to them.  A gist that cannot be cloned or updated is recorded as a failure of the run (`gist <id>`) while the other gists continue.

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
