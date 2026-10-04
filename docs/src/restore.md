# Restore Guide

This guide covers getting data back: the automated `--restore` mode (labels,
milestones, issues), the manual procedures for everything else, and decrypting
S3 objects.  Read [What Is Not Backed Up](#what-is-not-restored-or-not-backed-up)
before you rely on any of it.

---

## Automated Restore (`--restore`)

`--restore` re-creates **labels**, **milestones** and **issues** from the JSON
backup in the repositories of a target organisation (or user) through the
GitHub REST API.

It is a mode of its own:

- It reads the **local backup** under `--output`; it makes **no backup first**
  and does **not contact the source account**, so it works when the source
  repository or account has been deleted.
- The **target repositories must already exist**, with the same names as in the
  backup.  Restore never creates repositories; create them first (code with the
  [git procedure](#git-data) below).
- It asks for confirmation before it does anything.

### Basic usage

```bash
export GITHUB_TOKEN=ghp_dummy_write_token     # a token with write access
github-backup octocat \
  --output /var/backup/github \
  --restore \
  --restore-target-org new-org
```

`OWNER` (`octocat` here) names the backup to read; `--restore-target-org`
names where to write (default: the same name as `OWNER`; do not restore into the
repositories you backed up).  The tool prints a warning banner and asks you to
type `yes`.  When stdin is not a terminal, confirm with the flag or the
environment variable:

```bash
github-backup octocat --output /var/backup/github \
  --restore --restore-target-org new-org --restore-yes
# or
GITHUB_BACKUP_RESTORE_YES=1 github-backup octocat --output ... --restore ...
```

### Dry run

`--dry-run` shows what would be created without asking for confirmation and
**without any API call**:

```bash
github-backup octocat --output /var/backup/github \
  --restore --restore-target-org new-org --dry-run
```

### What is restored

| Artefact | Source JSON | Endpoint |
|----------|-------------|----------|
| Labels | `json/repos/<repo>/labels.json` | `POST /repos/{org}/{repo}/labels` |
| Milestones | `json/repos/<repo>/milestones.json` | `POST /repos/{org}/{repo}/milestones` |
| Issues | `json/repos/<repo>/issues.json` | `POST /repos/{org}/{repo}/issues` |

Details and limits:

- **Issues** are created with their title, body and labels.  The body gets a
  footer saying where it came from (`Restored by github-backup from
  octocat/repo#12 (opened by @alice on 2024-03-01T...)`) because GitHub sets the
  author and date to the restoring token and the time of restore.  **Closed
  issues come back open**; assignees, milestone links, comments, reactions and
  timeline events are **not** restored.  Issue numbers are not preserved.
- **Pull requests** (entries of `issues.json` that carry a `pull_request`
  property) are skipped; GitHub has no API to import them.
- **Milestones** keep title, description, state and due date; **labels** keep
  name, colour and description.
- Within each repository the order is labels, then milestones, then issues, so
  the labels exist when the issues that use them are created.

### Repeating a restore

Labels and milestones that already exist (HTTP 422) are counted as *skipped*.
Every restored issue carries a hidden marker naming its source
(`<!-- github-backup-restore:octocat/repo#12 -->`); before restoring a
repository's issues the tool lists the target's issues (open and closed) and
skips any whose marker it finds.  So running the same restore twice does not
duplicate anything.  The marker lives in the issue body: an issue whose body was
edited to remove it, or that was deleted, is created again.

### Outcome and exit status

A repository that cannot be restored (target missing, no write access) is
logged and counted; the others continue.  The final line shows
`labels: N created, N skipped, N errored | milestones: ... | issues: ...`.
Exit status: `0` all fine, `3` some items could not be restored, `1` the restore
could not start (no backup found, confirmation refused).

### Token requirements

| Token | Needs |
|-------|-------|
| Classic PAT | `repo` (private repositories) or `public_repo` (public only) |
| Fine-grained PAT | Issues: read and write on the target repositories (labels and milestones are covered by the same permission) |

The fine-grained row is taken from GitHub's documentation and has not been
exercised against a live account by the project.

---

## What Is Not Restored, or Not Backed Up

Be clear about what a restore cannot give back:

- **Pull requests**, issue and PR **comments**, **reviews**, **reactions** and
  timeline events: saved in the JSON files, but there is no importer.
- **Discussions** and **Projects**: not backed up at all (GitHub's REST API has
  no endpoints for them; `--discussions` / `--projects` do nothing).
- **Issue attachments and images**: the JSON holds the links; the files are
  not downloaded.
- **Actions secrets and variables, code-scanning and Dependabot alerts,
  rulesets, team permissions, commit comments and statuses, wiki attachments,
  stargazers of your repositories**: not collected.
- **Git LFS objects**: only with `--lfs` (needs `git-lfs`); without it the
  clone holds LFS pointers only.
- **Starred gists**: metadata only (no content).
- **Repository clones in S3**: S3 receives the JSON only.  If the local backup is
  lost, the git data is gone unless you have a mirror (`--mirror-to`) or a copy
  of `<output>/<owner>/git/`.
- Everything needs a token that could **see** it in the first place:
  private data of other users or organisations is never visible.

---

## Git Data

The clones under `git/repos/<repo>.git` are normal bare repositories.  The manual
procedures in the rest of this page use standard git and GitHub behaviour; the
project has not run them against github.com.

> **Do not use `git push --mirror` to push to GitHub.**  A mirror clone of a
> GitHub repository contains `refs/pull/*`, which GitHub does not accept
> ("deny updating a hidden ref"), so the push fails.  Push branches and tags:

```bash
# 1. create the empty target repository first (the push does not create it)
gh repo create new-org/my-repo --private

# 2. push branches and tags
git -C /backup/octocat/git/repos/my-repo.git push --prune \
    https://github.com/new-org/my-repo.git \
    '+refs/heads/*:refs/heads/*' '+refs/tags/*:refs/tags/*'
```

`--prune` deletes branches and tags at the destination that are not in the
backup; leave it out when pushing into a non-empty repository.  Pull request
refs and the pull requests themselves do not come back.

### Wikis

The wiki repository only exists once the wiki has a first page (create one in
the web UI), then:

```bash
git -C /backup/octocat/git/wikis/my-repo.wiki.git push \
    https://github.com/new-org/my-repo.wiki.git '+refs/heads/*:refs/heads/*'
```

### Gists

Gist git data lives in `git/gists/<gist-id>.git/`; the description and
visibility are in `json/gists/<gist-id>.json`.  Create a new gist in the web UI
(or with `gh gist create`), then push the content:

```bash
git -C /backup/octocat/git/gists/abc123.git push --force \
    https://gist.github.com/<new-gist-id>.git 'refs/heads/*:refs/heads/*'
```

### Releases and Assets

Assets are under `json/repos/<repo>/release_assets/<tag>/<file>`:

```bash
gh release create v1.0.0 \
  /backup/octocat/json/repos/my-repo/release_assets/v1.0.0/my-binary \
  --title "v1.0.0" --notes "Restored from backup" \
  --repo new-org/my-repo
```

`releases.json` has the release notes, draft and prerelease flags.

### Labels via `curl`

If you prefer scripting over `--restore`:

```bash
jq -c '.[]' /backup/octocat/json/repos/my-repo/labels.json | \
while read -r label; do
  name=$(jq -r '.name' <<< "$label")
  color=$(jq -r '.color' <<< "$label")
  desc=$(jq -r '.description // ""' <<< "$label")
  curl -s -X POST \
    -H "Authorization: Bearer $GITHUB_TOKEN" \
    -H "Content-Type: application/json" \
    -d "$(jq -n --arg n "$name" --arg c "$color" --arg d "$desc" '{name:$n,color:$c,description:$d}')" \
    "https://api.github.com/repos/new-org/my-repo/labels"
done
```

### Branch Protection, Deploy Keys, Collaborators

`branch_protections.json` (an object keyed by branch name), `deploy_keys.json`
and `collaborators.json` are records.  There is no automated path back: the
GitHub endpoint that sets branch protection (`PUT .../branches/<b>/protection`)
expects a different request shape than the response that was saved, so the
rules have to be translated by hand.  Deploy keys can be re-added from the
public key material; collaborators have to be re-invited.

### Starred, Followed and Organisation Data

These files are reference archives (useful for auditing or rebuilding the
social graph by hand); nothing restores them automatically.

---

## Decrypting S3 Objects

If you used `--encrypt-key`, objects in the bucket end in `.enc`.  Download one
with your provider's tool, then:

```bash
export BACKUP_ENCRYPT_KEY=...        # the 64-hex-character key used for the upload
github-backup --decrypt \
  --decrypt-input issues.json.enc --decrypt-output issues.json
```

`--decrypt` needs no `OWNER` and no network.  The wire format is
`[12-byte random nonce][ciphertext + 16-byte GCM tag]`.  `openssl enc` cannot
decrypt it (it does not support AES-GCM); use the tool, or any AES-GCM library,
as shown in [At-Rest Encryption](storage/encryption.md#decrypting).

---

## Layout Reference

Where each file lives is on one page: [Output Directory Layout](configuration/output-layout.md).
The implementation of `--restore` is described in [Restore Implementation](development/restore.md).
