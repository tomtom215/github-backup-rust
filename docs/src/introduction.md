# github-backup

**github-backup** is a GitHub backup tool written in Rust.  It backs up the
repositories, issues, pull requests, releases, gists, wikis and relationship
data of a GitHub user or organisation to a local directory, optionally copies the
JSON metadata to S3-compatible storage and the git data to another git host.  TLS
is `rustls` (no OpenSSL), the S3 client is a pure-Rust SigV4 implementation (no
AWS SDK), and an optional full-screen TUI is available with `--tui`.

> **Documentation version.**  These pages describe the current development
> branch (`main`).  The latest release, v0.3.2, predates many of the behaviours
> described here (exit status `3`, per-repository incremental state, lossless
> JSON, `--doctor`, restore changes, and more); the `Unreleased` section of the
> [changelog](development/changelog.md) lists them.  See
> [Installation](getting-started/installation.md) for how to get a matching build.

## Feature Highlights

| Feature | Details |
|---------|---------|
| Repositories | Mirror, bare, full or shallow clones, kept up to date with `git fetch`; optional Git LFS objects |
| Issues and pull requests | The lists, comments, events, commits and reviews, saved as GitHub's complete JSON responses |
| Releases | Metadata and, optionally, the assets (streamed, size and checksum verified) |
| Gists and wikis | Owned gists and repository wikis cloned; starred gists as metadata |
| Metadata | Topics, branches and protection rules, labels, milestones, hooks, deploy keys, collaborators, security advisories, Actions workflows, runs and environments, package metadata |
| User and organisation data | Starred, watched, followers, following, organisation members and teams; optional clone of every starred repository |
| Private data | A user's own private repositories and secret gists are included when the token belongs to that user |
| Filters | `--include-repos` / `--exclude-repos` glob patterns on repository names |
| Incremental runs | A watermark per repository; lists are always fetched in full and merged, so a run never loses data; `--full` forces a complete refresh |
| Honest outcome | Failures never stop the rest of the run; exit status `3` means "finished but incomplete"; report, metrics, webhook and history agree |
| S3 sync | The JSON metadata (not the clones) to AWS S3, Backblaze B2, MinIO, Cloudflare R2, Spaces or Wasabi, with content-digest skipping |
| At-rest encryption | AES-256-GCM of the S3 uploads (`--encrypt-key`) |
| Push mirrors | Branches and tags to Gitea, Forgejo, Codeberg or GitLab; repositories created private by default; never overwrites a repository it did not create |
| Restore | Labels, milestones and issues back into an organisation, from the local backup (`--restore`) |
| Authentication | Personal access token, OAuth device flow, or anonymous (public data only) |
| GitHub Enterprise Server | `--api-url` and `--clone-host` |
| Config file | TOML; see [precedence](configuration/config-file.md#precedence) for how it combines with the command line |
| Monitoring | JSON report, Prometheus textfile metrics, webhook, run history |
| Safety | `--dry-run` writes nothing; OS file locks (no stale locks); atomic writes; atomic clones |
| Interactive TUI | Full-screen terminal interface with live progress (`--tui`) |
| Containers | Alpine-based multi-arch image, Compose profiles, Unraid template |

## What It Does Not Back Up

- **Discussions and Projects**: GitHub's REST API has no endpoints for them
  (`--discussions` and `--projects` are accepted but do nothing).
- Actions secrets and variables, code-scanning and Dependabot alerts, rulesets,
  issue attachments, commit comments and statuses.
- **Git LFS objects** unless you pass `--lfs` (requires `git-lfs`).
- Repository **clones are not uploaded to S3**, only the JSON metadata.
- It cannot see what the token cannot see: other users' private data, or
  categories the token lacks permission for (a 403/404 is skipped with an `INFO`
  line and is not counted as a failure).
- Pull requests, comments and reactions cannot be restored; `--restore` handles
  labels, milestones and issues only.

See [What Is Not Restored, or Not Backed Up](restore.md#what-is-not-restored-or-not-backed-up).

## Design Principles

- **No OpenSSL, no reqwest, no AWS SDK**: TLS via `rustls`, HTTP via `hyper`.
- **No `unsafe` in the workspace's own source.**
- **Trait-based**: `BackupClient`, `Storage` and `GitRunner` make the engine
  testable without network or filesystem.
- **Credentials stay out of argument lists, URLs and files**: git receives the
  token through an environment variable and a host-scoped credential helper.
- **Rate-limit aware**: rate-limit responses are waited out and retried.
- **A backup that is incomplete says so**: see
  [Monitoring](monitoring.md).

## Quick Links

- [Installation](getting-started/installation.md)
- [Quick Start](getting-started/quick-start.md)
- [Interactive TUI](tui.md)
- [CLI Reference](configuration/cli-reference.md)
- [Config File (TOML)](configuration/config-file.md)
- [Docker](docker.md)
- [Architecture](development/architecture.md)
- [GitHub Repository](https://github.com/tomtom215/github-backup-rust)
