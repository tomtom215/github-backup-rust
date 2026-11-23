# Browsing a Backup (HTML Viewer)

A backup is GitHub's JSON plus git clones: complete, but not something you can
read.  `--render-html` turns it into a static website that works offline: open
`index.html` in any browser, from the disk or from any static web server.  No
GitHub account, no network and no server are needed, so the backup stays
readable after the original repositories, or the account, are gone.

```bash
# Render the backup of octocat under /var/backup into /var/backup/octocat/site
github-backup octocat --output /var/backup --render-html

# Or into another directory (note the `=`)
github-backup octocat --output /var/backup --render-html=/srv/www/octocat
```

`--render-html` is a mode of its own, like `--verify`: it needs OWNER, reads
`<output>/<owner>/json/` and the clones under `<output>/<owner>/git/`, makes
no backup and does not contact GitHub.  Without a directory the site goes to
`<output>/<owner>/site/`.  The directory is given with `=` only, so
`--render-html octocat` still means "render the backup of octocat".

Run it after each backup (for example in the same cron job or systemd unit)
to keep the site current; it reads files that the backup writes atomically,
so it can also run while a backup is in progress.

## What the site shows

| Page | Content |
|------|---------|
| Index | Every repository in the backup: description, visibility, archived, fork and template badges, language, stars, open and total issues and pull requests, releases, last push; links to the owner's other pages |
| Repository overview | The fields of `info.json`, topics, the paths of the local clones (repository, wiki) with a `git clone` command to check them out, and links to every JSON file of the repository |
| Issues | Open and closed lists; one page per issue with its text, labels, assignees, milestone, and a timeline of comments and events (labelled, renamed, closed, ...) in time order |
| Pull requests | Open and closed lists (merged and draft ones are marked); one page per pull request with the conversation, submitted reviews, inline review comments grouped by file and thread under the diff they were made on, and the commits |
| Releases | Every release, newest first, with its notes and assets; an asset downloaded with `--release-assets` links to the local file, others to GitHub |
| Labels and milestones | All labels and milestones, with descriptions and progress |
| Discussions | Only when the backup holds `discussions.json` (with `discussion_comments/<n>.json`); laid out like issues |
| Gists | Own and starred gists; one page per gist with its files (their content when the backup has it) and comments, and the path of the local clone |
| Projects | Each project (v2) with its items and their field values as a table |
| Starred, watched, followers, following, members, teams | The owner-level lists, when they were backed up |

Markdown is rendered as on GitHub: tables, task lists, strikethrough,
footnotes, alerts, bare URLs and `#123` references (which link to the local
page of that issue or pull request when the backup has it, as do links to
the repository's own issues on github.com).

A page shows what the backup holds.  A category that was not backed up is
simply absent (an empty list, or no tab), and a file that cannot be read is
skipped: the run logs a warning naming it and exits with status `3`; the
rest of the site is still written.

### Images and attachments

A page never loads anything from the internet.  An image or file in an
issue, comment or release note is taken from the backup when it has a local
copy: the repository's `json/repos/<repo>/attachments/index.json` maps each
attachment URL to its file in that `attachments/` directory.  Images with a
local copy are shown, links to attachments lead to the local file, and every
other image is shown as a link to the original.

## Safety

Issue and comment text is written by anyone who can open an issue, so the
site treats it as hostile:

- Every page carries a Content-Security-Policy that forbids scripts and
  allows nothing to be loaded except the site's own style sheet and local
  images (`default-src 'none'; img-src 'self' data:; style-src 'self'`).
  The site contains no JavaScript.
- Raw HTML in Markdown is never passed through: it is shown as text (an
  HTML block as code; `<br>` becomes a line break; HTML comments, which
  GitHub does not show either, are dropped).
- Links are kept only for `http`, `https` and `mailto` URLs; `javascript:`,
  `data:`, `vbscript:` and every other scheme become plain text.  Relative
  links are resolved against the issue's address on GitHub.
- Every title, name, label and description is escaped.

## The site directory

The site is built in a hidden directory next to the target
(`.<name>.rendering`) and then renamed into place, replacing an earlier render
as a whole, so a page that no longer exists does not linger.  A directory is
only ever replaced when it is empty or holds the `.github-backup-site` file
that every render writes; any other existing directory is refused (exit
status `1`), so a mistyped path cannot delete anything.  The site may not be
placed inside `json/`, where `--manifest`, `--verify` and the S3 sync would
take it for backup data.

Links between pages and to the backup files are relative, so the site works
from `file://` and on any static host.  To publish it, copy the site and the
backup's `json/` (and `git/`, for the clone links) together, keeping their
relative positions; or render straight into the web server's directory.
Note that the site shows private repositories and secret gists if the backup
has them.

Two renders of the same backup are byte-identical (stable ordering, no
timestamps), so a site kept under version control only changes where the
backup did.

Rendering is fast: a repository with 10 000 issues and 10 000 pull requests
(three comments each) renders to about 97 MB of HTML in under two seconds on
four cores.

## Exit status

| Code | Meaning |
|------|---------|
| `0` | The site was written and every backup file could be read. |
| `1` | No site was written: no backup at `<output>/<owner>/json`, the directory is in use for something else, or it could not be written. |
| `3` | The site was written, but some backup files could not be read (each is logged); their pages show what else there is. |
