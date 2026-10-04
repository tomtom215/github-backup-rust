# Local Filesystem Storage

By default `github-backup` writes everything to the local filesystem.  The root
directory is set with `--output` (or `output` in the config file) and defaults
to the current directory.

## Output Directory Layout

Each owner gets its own directory below `--output`:

```
<output>/
├── .github-backup.lock
└── <owner>/
    ├── git/                ← clones: repos/, wikis/, gists/, starred/
    └── json/               ← JSON metadata and the bookkeeping files
```

The complete tree, with the file each flag produces, is on the
[Output Directory Layout](../configuration/output-layout.md) page.  It is the
only place that lists every file; this page does not repeat it.

## Incremental Updates

`github-backup` is designed to be run repeatedly into the same directory.  On
later runs:

- **Git repositories** are updated in place (`git fetch --all` for mirrors)
  instead of being cloned again.  Deleted branches and tags are kept unless
  `--prune` is given; force-pushed branches are overwritten.
- **JSON lists** (`issues.json`, `pulls.json`, `releases.json`, ...) are fetched
  in full and written again.  `issues.json` and `pulls.json` are **merged** with
  what is already stored, so an item that has since disappeared from GitHub's
  listing stays in the backup.
- **Per-item files** (`issue_comments/<n>.json`, ...) are fetched again only for
  items that changed since the repository's watermark; see
  [Incremental runs](../monitoring.md#incremental-runs-and-the-state-file).
- **Release assets** are kept when they are complete (size and checksum match)
  and downloaded again otherwise.

Every file is written to a temporary name and renamed into place, and a fresh
clone is made in a hidden staging directory and renamed when it is complete, so
an interrupted run does not leave a truncated file or a half-written clone
under its real name.

## Disk Space

Plan for the repositories plus their metadata.  A rough guide:

| Content | Typical size |
|---------|-------------|
| Mirror clone, small repository | 1 to 100 MB |
| Mirror clone, large repository | 100 MB to 10 GB |
| `issues.json` for 1 000 issues | several MB: the files hold GitHub's full responses, roughly four times the size of a summary |
| Release assets | highly variable; they usually dominate with `--release-assets` |

A mirror also keeps `refs/pull/*`, which makes it larger than a plain clone.
The tool does not check free space before a run (`--doctor` does not either);
measure with `du -sh <output>/<owner>` after a trial run.  A full disk stops the
run with exit status `1`.

## Permissions

The process needs read and write access to `--output` and must be able to run
`git` (and `git-lfs` with `--lfs`).  New files get the mode implied by the
process `umask`.  Backups can contain private repository code, webhook
configuration and security advisories, so restrict the directory
(`chmod 700`) or encrypt the volume.

For unattended runs create a dedicated user:

```bash
sudo useradd -r -m -d /var/backup/github github-backup
sudo -u github-backup env GITHUB_TOKEN="$GITHUB_TOKEN" \
  github-backup octocat --output /var/backup/github --all
```

When the tool runs `git` in a repository it trusts exactly that path
(`-c safe.directory=<path>`), so a backup directory that was created by another
user, or chown-ed by a file manager or NAS tool, still updates.  Your **own**
`git` commands in such a directory may stop with "detected dubious ownership";
fix the ownership (`chown -R`) or add the path with
`git config --global --add safe.directory <path>`.

## Getting the Code Back

A mirror clone is a valid git repository:

```bash
# Clone from the backup
git clone /var/backup/github/octocat/git/repos/Hello-World.git hello-world
```

To push it to a new remote, see [Restore](../restore.md#git-data): do not use
`git push --mirror` against GitHub, which rejects the `refs/pull/*` refs a
mirror contains.
