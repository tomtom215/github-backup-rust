# Changelog

All notable changes to `github-backup` are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

Changes on `main` since v0.3.2 (2026-04-12).  Nothing here is in a published
release yet; `github-backup --version` prints `0.3.2` for both until the version
is bumped.

### Breaking changes and upgrade notes

- **With no category selected, a run backs up everything** (as `--all` does).
  Before, `github-backup OWNER` backed up nothing and still reported success.
  `--skip <categories>` (or the `skip` config key) leaves categories out; any
  category flag still selects just that subset.
- **OWNER is optional**: it defaults to the account the token belongs to, and an
  organisation is recognised without `--org`.  The token is also taken from
  `GH_TOKEN` or the GitHub CLI (`gh auth token`) when no other source is set, and
  `~/.config/github-backup/config.toml` (`%APPDATA%` on Windows) is loaded when
  `--config` is not given.
- **`--restore` needs `--restore-target-org`** naming an account other than the
  source (or the explicit `--restore-into-source`): restoring into the
  repositories that were backed up duplicated every issue there.
- **Mirror destinations keep refs deleted on GitHub** unless `--prune` is given,
  as the local clones do; repositories whose backup failed in the run, shallow
  clones and clones without refs are no longer pushed.
- **Encrypted S3 objects use a new streamed format** (see Security).  Objects in
  the old format still decrypt; every encrypted object is uploaded once more
  because its size changes.
- **Release checksum sidecars moved** from `release_assets/<tag>/<asset>.sha256`
  to `release_checksums/<tag>/`, so a real `<asset>.sha256` published by the
  project is no longer overwritten; old sidecars are migrated.
- **More files accumulate history** instead of being overwritten: releases,
  labels, milestones, security advisories, workflows and their runs, gists and
  their comments, packages, projects, commit comments and every per-issue and
  per-pull-request file.  `--restore` therefore also recreates labels and
  milestones that were deleted on GitHub.  Access and configuration files
  (hooks, deploy keys, collaborators, environments, branches, protection,
  rulesets, topics, followers and the like) remain snapshots.
- **`info.json` is the full repository object** (`GET /repos/{owner}/{repo}`:
  fork parent, template, merge settings, ...) instead of the shorter listing
  entry; `topics.json` is taken from it.
- **A repository deleted and recreated under the same name** no longer mixes two
  repositories' data: the old backup is set aside as `<name>@<old id>`.
- **Access refused (403) for a category you selected by flag is a failure**
  naming the missing permission (exit `3`); under `--all` and the default
  selection it is one warning per category per run.  A category that does not
  exist for a repository (404/410) is not a failure.
- **Minimum Rust version 1.89** (building from source only).  CI and the release
  build with the toolchain pinned in `rust-toolchain.toml` (1.100.0).
- **Container users:** `:latest` is still v0.3.2.  Compose and the Unraid
  template use the new `:edge` image (built from `main`) until the next release.

- **Exit status `3` is new.**  A run that finishes but could not back up
  everything now exits `3` (it used to exit `0` and report success).  Scripts and
  unit files that treated every non-zero status as "could not run" need no change
  (a systemd unit simply fails); scripts that only checked for `1` should accept
  `3` as "incomplete".  The other statuses are `0` ok, `1` fatal, `2` usage,
  `130`/`143` interrupted.
- **Saved JSON is GitHub's complete response**, with GitHub's key order, instead
  of a typed subset.  Files are roughly four times larger and now contain
  properties that were dropped before (reactions, `node_id`, `_links`, review
  comment anchors, merge commit SHAs, ...).  Pull requests also get
  `issue_comments/<n>.json` and `issue_events/<n>.json`.
- **S3 object keys now include the owner**: `<prefix>/<owner>/json/<path>` (they
  were `<prefix>/<path>`).  The first run after upgrading uploads everything again
  under the new keys; the old objects are **not** removed (`--s3-delete-stale`
  only looks under the new `<prefix>/<owner>/json/` root), so delete them yourself
  once you no longer need them.  Objects are now skipped by SHA-256 digest plus
  size instead of size alone; an object without a stored digest is uploaded once
  more.  With `--encrypt-key` the digest is keyed, so changing the key re-uploads
  everything.
- **`--mirror-to` pushes branches and tags only** (`git push --prune` with
  explicit refspecs, no `--mirror`), creates every repository **private** unless
  `--mirror-public` is given **and** the source is known to be public (a private
  source is never published), and **refuses to push into an existing repository
  that it did not create** (unless that repository is empty).  Gitea repositories
  for an organisation owner are created through the organisation endpoint.
  Working-tree clones (`--clone-type full`) are not pushed.
- **Git clones keep what GitHub deleted.** Updates no longer prune: a branch or tag
  deleted on GitHub stays in the clone (force-pushed branches are still
  overwritten).  Pass `--prune` (config key `prune`) for the old behaviour.
  `--no-prune` and the config key `no_prune` are still accepted and ignored; they
  cannot be combined with `--prune`.  If a deleted branch `foo` is replaced by
  `foo/bar`, git cannot hold both: that update prunes once, logs a warning and
  continues.  The TUI's "No Prune" toggle became "Prune deleted refs".
- **`--restore` is restore-only**: it no longer runs a backup first and works from
  the local backup.  It asks for confirmation first, skips issues it restored
  before, and exits `3` when items could not be restored.  Target repositories must
  exist.
- **`--keep-last` and `--max-age-days` are deprecated and ignored** (a warning is
  logged).  They deleted directories by name pattern and could not work with the
  tool's own layout.  Rotate copies with your backup tool.
- **`--since` changed meaning.**  Issue and pull request lists are always fetched
  in full and merged into the stored files; `--since DATE` (now also `YYYY-MM-DD`)
  is an expert override that is never stored.  v0.3.2 applied the previous run's
  timestamp automatically and could overwrite `issues.json` with only the changed
  issues.  `backup_state.json` now holds a watermark per repository; an old file
  only costs one full fetch.
- **`--lfs` is now a mirror clone plus `git lfs fetch --all`** (it used to run
  `git lfs clone`, which created a non-bare checkout named `<repo>.git` whose refs
  never updated, and it overrode `--clone-type`).  Delete and re-clone existing
  `--lfs` repositories to get a proper mirror.
- **Report, metrics, webhook and history gained fields**: see Added.  `success` in
  the report now means "no failures" (it meant `repos_errored == 0`).
- **Linux release binaries are static musl builds**; the v0.3.2 x86_64 binary was
  glibc-linked and needed glibc 2.39 or newer.

### Added

- **Proxy settings** (`HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`, `NO_PROXY`) now
  also apply to S3 uploads and to the Gitea/GitLab API calls of `--mirror-to`;
  loopback hosts are always reached directly.
- **TUI:** an empty Owner backs up the token's account; Commit Comments and
  Attachments toggles; `A` selects exactly what `--all` selects.
- **Docker:** with only `GITHUB_TOKEN` set, the container backs up the token's
  account instead of printing help.

- **`--render-html[=DIR]`** turns a backup into a static website you can read
  offline (default `<output>/<owner>/site`): repositories, issues and pull
  requests with comments, events, reviews, review comments and commits,
  releases linked to their downloaded assets, labels, milestones, discussions,
  gists, projects and the account lists.  No JavaScript, a strict
  Content-Security-Policy and escaped HTML make the pages safe to open; a
  re-render of the same backup is byte-identical.  New crate
  `github-backup-viewer`.
- **Discussions** (`--discussions`, part of the default backup) through the
  GraphQL API: every discussion with labels, poll, reactions and answer, every
  comment with all its replies, and the categories (`discussions.json`,
  `discussion_comments/<n>.json`, `discussion_categories.json`).
- **Attachments** (`--attachments`, part of the default backup): files and
  images attached to issues, pull requests, discussions, comments, reviews,
  commit comments and release notes are downloaded to
  `repos/<repo>/attachments/<sha256>.<ext>` with an `attachments/index.json`.
  Only GitHub's attachment hosts are contacted, files are capped at 100 MiB and
  the stored JSON is left as GitHub sent it.  Attachments of private
  repositories need a classic token; fine-grained and App tokens are reported to
  get 404, which is recorded as missing and retried after 30 days or with
  `--full`.
- **GitHub App authentication** (`--app-id`, `--app-key`,
  `--app-installation-id`, the `GITHUB_APP_*` variables and `app_*` config keys)
  for unattended organisation backups.  Installation tokens are renewed before
  they expire, once for all workers, for API requests and every git process;
  `--doctor` checks the App, its installation and its permissions.

- **Projects (v2)** of the owner, with their fields and every item's field
  values (`json/projects/`); **repository rulesets** (`rulesets.json`);
  **commit comments** (`--commit-comments`, `commit_comments.json`); **gist
  comments**.  All are part of the default backup.
- **Pull requests with more than 250 commits** are listed in full through the
  compare API (the commits endpoint stops at 250).
- **`--every <interval>`** runs the backup on a schedule from a long-running
  process or container (minimum one minute), with each run's report, metrics and
  history as before.
- **`--verify-git`** checks every repository with `git fsck` after each clone and
  update (a fresh clone that fails is discarded); **`--git-timeout`** sets how long
  git may stay silent before it is stopped (default 600 s).
- **`--notify-format json|slack|discord|healthchecks`** for the webhook; an
  interrupted run is notified as `interrupted`.
- **`--skip`**, the `mirror_type`, `skip`, `verify_git` and `git_timeout` config
  keys, and `--restore-into-source`.
- **Doctor** (`--doctor`, `--check`) validates the configuration exactly as a run
  would and checks the owner, the selected categories and the mirror and S3
  settings; mirror and S3 settings are checked before a backup starts.
- **Install scripts** (`install.sh`, `install.ps1`) that verify checksums,
  cargo-binstall metadata, an `:edge` container image from `main`, and container
  images with an SBOM, provenance and a GitHub attestation.
- **GraphQL support in the client** (used by Discussions), API version pinning
  (`X-GitHub-Api-Version: 2022-11-28`, which keeps fields that `2026-03-10`
  dropped) and a once-per-endpoint warning for GitHub `Deprecation`/`Sunset`
  notices.

- `--prune` and `--mirror-public` (and the config keys `prune`, `mirror_public`); see the
  upgrade notes above for what they change.
- **Failure isolation and honest reporting.**  Every repository and every category
  is an isolated step; a failure is recorded (scope, step, message) and the rest of
  the run continues.  Only credentials rejected (401), an exhausted rate-limit
  budget, a full disk and cancellation stop a run.  The summary banner, the
  `--report` JSON (`schema_version`, `finished_at`, `failure_count`, `failures[]`),
  the Prometheus metrics (`github_backup_failures`, `github_backup_success`,
  `github_backup_last_run_timestamp_seconds`; `..._last_success_timestamp_seconds`
  is carried over from the previous file when a run fails), the run history
  (`failures`, and failed runs are recorded too) and the webhook (`status`
  `success`/`partial`/`failure`, `failure_count`, `failed[]`) all read the same
  list.  A run that fails early still writes report, metrics, history and webhook.
  S3 upload failures, mirror push failures and a manifest that cannot be written
  count as failures.
- **Per-repository incremental watermarks** (`backup_state.json`): per-item files
  (issue and pull request comments, events, commits, reviews) are re-fetched only
  for items changed since the watermark, which advances only for repositories whose
  every step succeeded and is trusted only for the categories it was recorded with.
  **`--full`** ignores watermarks.
- **`repos.json`** is written (only the repositories the options include, merged
  with earlier listings) and used by `--diff-with`, which now actually compares
  repository names.
- **A user's own private repositories and secret gists** are listed (through
  `GET /user/repos` and `GET /gists`) when the token belongs to the owner; before
  they were silently missing.
- **Lossless JSON** via `Raw<T>`/`Page<T>`; an element the typed model cannot read
  is written verbatim and logged instead of aborting the list; nullable API fields
  are modelled as `Option`; golden tests use GitHub's own example payloads.
- **Release assets are streamed**, verified against the size and `sha256:` digest
  GitHub reports, written through a `.part` file, and skipped on later runs only
  when complete; the `Authorization` header is never sent to another origin on a
  redirect and an HTTPS to HTTP redirect is refused.
- **`--doctor`, `--check`, `--list-scopes`, `--print-config-template`**.  `--doctor`
  checks the `git` binary, the output directory, the kind of credential, API
  reachability (through the real client: API URL validation, proxy, CA bundle) and
  that GitHub accepts the token (`GET /rate_limit`); it does not check scopes or
  free space.  `--check` adds the resolved configuration.  `--list-scopes` prints
  suggested classic scopes (it does not expand `--all`).
- **Friendly quickstart** when run without arguments, a pre-run plan banner and an
  end-of-run summary banner (which lists what failed), `--help` examples, actionable
  hints for common errors, and token-format recognition in `--doctor`.
- **`GITHUB_BACKUP_RESTORE_YES=1`** as an alternative to `--restore-yes`; restored
  issues carry the original author and date and a hidden source marker so a repeated
  restore does not duplicate them.
- **Proxy support**: `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY` and `NO_PROXY` (hosts,
  domains, ports, IPs, CIDR blocks); HTTP proxies only.  Shared by the API client,
  release downloads, the webhook and `--doctor`.  S3 and the mirror APIs do not use
  a proxy.
- **S3**: `--s3-session-token` / `AWS_SESSION_TOKEN`, credential validation before
  the first request, content-digest skipping, retries with back-off and an idle
  timeout (a request is abandoned only when no data moved for 60 s), a guarded `--s3-delete-stale` (nothing is deleted after a failed
  run, an unreadable or empty local tree, or a failed upload; only under
  `<prefix>/<owner>/json/`), and an in-process SigV4-verifying fake S3 server in the
  tests.
- **Mirror**: a destination-side marker (`GitHub mirror of <owner>/<repo>`) and
  per-repository failure reporting.
- **Checkpoint resume is limited to 6 hours** after an interrupted run's last
  activity; `--clone-starred` mirrors are refreshed on every pass and earlier
  failures are retried automatically.
- **`--all` honours explicit `--clone-starred` and `--action-runs`**; a config
  file accepts `clone_type = "shallow:3"` (and `{ shallow = 3 }`); a
  `--clone-type` on the command line beats the config file.
- **`--decrypt` needs no OWNER.**
- **Docker, Compose and Unraid**: see Packaging below.

### Changed

- **`--decrypt`** streams objects of the current format with bounded memory and
  writes the output only after the last segment authenticates; a truncated,
  tampered or wrong-key object leaves no output file.
- The owner and organisation look-ups before a run use the client's retry and
  rate-limit handling; hints cover device-flow refusals, pagination and GraphQL
  errors, and `--doctor` recognises an answer that is not the GitHub API.

- **Repeat runs ask only for what changed.**  Issues are listed with `since`
  and pull requests newest-updated first until older ones, and both are merged
  into the stored lists; `git fetch` is skipped for a repository nothing was
  pushed to since its last clean backup when its clone is on disk and was
  fetched within the last 7 days (wikis are always fetched).  An unchanged
  repository costs about 18 requests whatever its size, instead of 15 plus one
  per 100 issues and pull requests plus a fetch.  `--full` disables every
  shortcut; the first run after upgrading lists and fetches everything once.

- **Dependencies:** `chrono` (unmaintained) replaced by `jiff`; `fslock` replaced
  by the standard library's file locks.
- **Rate limits pause every worker** together instead of only the refused request.
- **Every file is written durably**: data is flushed to disk before it replaces
  the previous file, with temporary names unique per write; leftovers of a
  killed run are removed at the next start.
- **Full clones** update their checkout, local branches and tags on every run;
  **shallow clones** hold every branch.
- **A stopped git process** gets SIGTERM and two seconds to clean up before
  SIGKILL, and SSH runs with `BatchMode=yes` unless you set your own SSH command.
- **S3:** large files are uploaded part by part (memory no longer grows with file
  size); a file that changes during upload is not completed; a lost
  `CompleteMultipartUpload` answer is confirmed with HEAD.
- **Mirror API calls** retry server errors; GitLab and Gitea refusals are reported
  with the server's message instead of being treated as "already exists".
- **Projects use the REST Projects (v2) API**; packages of an organisation use the
  organisation endpoints and the token owner's packages `/user/packages`.
- **`--device-auth`** works behind a proxy and on GitHub Enterprise Server, and its
  default scopes cover every default category except webhooks.
- **`--mirror-type`** accepts only `gitea` or `gitlab`; a typo is a usage error.
- The deprecated `--keep-last`, `--max-age-days` and `--no-prune` are hidden from
  `--help` and warn when used.

- **Minimum Rust version is 1.89** (was 1.88): the output-directory and per-owner
  locks now use the standard library's `File::try_lock`, and the `fslock` crate is
  gone.  The locks are released explicitly on drop, so a child process started on
  another thread at that moment cannot keep them held.
- **Dependencies:** the RustCrypto crates move together to their current generation
  (`aes-gcm` 0.11, `aes` 0.9 with `zeroize` still enabled, `hmac` 0.13, `sha2` 0.11);
  nonces and retry jitter come from `getrandom` directly.  `toml` 1, `nix` 0.32,
  `criterion` 0.8.  Unused dependencies were removed from four crates.

- **Rate limits** (403 or 429 with `Retry-After`, `X-RateLimit-Remaining: 0` or a
  rate-limit message) are waited out and retried (up to 6 times, about an hour in
  total per request, at least a minute and doubling when GitHub gives no hint)
  instead of being treated as "no access" and skipped; GET requests retry
  500/502/503/504, dropped connections, timeouts and stalled bodies three times.
- **Git subprocesses**: a **stall** timeout (600 s without output; was a 600 s
  wall-clock limit) with `--progress`, no pipe deadlock for large outputs, async
  runner on the blocking pool, process-group kill on cancellation, `git fetch --all
  --prune` instead of `git remote update --prune` (pruning has since become opt-in,
  see above), bare clones update with explicit
  refspecs (they never advanced before), fresh clones go to a staging directory and
  are renamed when complete.  Git runs with stdin closed, `GIT_TERMINAL_PROMPT=0`
  and `LC_ALL=C`.
- **Cancellation** (`SIGINT`/`SIGTERM`) abandons in-flight API requests (also the
  repository listing), kills git and releases the lock; exit `130`/`143` within a few seconds, with an exit
  watchdog behind it.
- **Locking** is an operating-system lock (`flock`/`LockFileEx`) in the output
  directory and in `<owner>/json/`, released by the kernel when the process dies; the
  lock files stay on disk.
- **`--dry-run` writes nothing**: no lock, state, report, manifest, metrics, history,
  webhook, S3 upload or mirror push; no git, no owner data and no gists.
- **Manifest and `--verify`** ignore the history, state, checkpoint and lock files.
- **`NO_COLOR`** follows no-color.org literally (set and non-empty disables ANSI).
- **Report and Prometheus files** are written atomically (temporary file and rename);
  a response body is capped at 16 MiB.
- **Empty environment variables are treated as unset**, and whitespace around
  option values is trimmed.
- **Dependencies** updated to clear five RustSec advisories (rustls, h2, lru,
  anyhow, crossbeam-epoch).
- **`main.rs` was split** into focused modules (`run`, `modes`, `setup`, `ui`,
  `metrics`, `notify`, `errors`, `shutdown`).
- **TUI**: failures are reported honestly, the terminal is restored on every exit
  (including a closed terminal), small screens and narrow layouts fit, and a
  cancelled run shows no misleading counters.

### Fixed

- Docker: a blank `BACKUP_NOTIFY_FORMAT` aborted every run.

- **Refs that should be kept were deleted**: a `fetch.prune` setting in the
  user's own git configuration pruned the backup, and a stale `.lock` file left
  by a killed git triggered a pruning retry that then failed on every run.
- **Silent data loss:** repository objects, issues, releases and gists that the
  data model could not read were dropped (with their comments and assets) and the
  run still exited `0`; repositories with Issues turned off lost every pull
  request conversation; one failing item stopped the rest of its category.
- **Checkpoints** from a narrower run made a later full run skip categories, and
  `--dry-run` deleted the checkpoint.
- **GitLab mirrors into a group** were created in the token user's namespace (and
  could be public there).
- **Quadratic merging** of large issue and pull request lists (40,000 items took
  minutes per run).
- **Memory:** the manifest and S3 sync read whole release assets into memory.
- **Hangs:** a stalled error response from an asset download hung the run, and a
  self-referencing pagination link looped forever.
- **Renamed or transferred repositories** failed with HTTP 301 instead of being
  followed.
- **Ctrl+C and SIGTERM** during the mirror push, S3 sync or notification were
  ignored, and an interrupted run wrote no report, metrics or history.
- Fatal errors did not stop work already in flight; a full disk or quota
  reported by git was not fatal.
- Branch and package names were not URL-encoded (`release#2`).
- Starred repositories: names were not path-checked, renames were ignored,
  `--clone-host` was not applied and a corrupt queue failed every run.
- Filtered repositories stayed "running" in the TUI; `--tui` ignored the config
  file; the plan showed "Concurrency 0"; a closed output pipe panicked.
- `--clone-host host:port`, `--prefer-ssh` for wikis, and the issue count (which
  included pull requests).
- Windows: `safe.directory` never matched, and reserved device names
  (`con`, `aux`, ...) are refused for repositories instead of failing obscurely.
- A flaky test ("Text file busy").

- The CLI exited with a usage error whenever `AWS_ACCESS_KEY_ID`,
  `AWS_SECRET_ACCESS_KEY`, `MIRROR_TOKEN` or `GITHUB_OAUTH_CLIENT_ID` existed in the
  environment, even empty (every Compose, Unraid and Kubernetes launcher); the
  dependency rules now apply to flags typed on the command line only.
- Incremental runs lost data: the automatic `--since` overwrote `issues.json` with
  only the changed issues (see Breaking changes), and a dry run advanced the same
  state.
- A killed run could leave a half-initialised clone that later fetches treated as
  complete; a stale lock from PID reuse blocked runs in containers.
- `SIGTERM` during a slow call or a long clone could leave the process running until
  it was killed.
- Repository names, release tags, asset names and gist ids from the API can no
  longer place files outside the output tree; an unsafe repository name is a recorded
  failure.
- A wiki that vanished upstream keeps its copy and logs a warning instead of
  failing; "exit 128" is only treated as "no wiki" when git says the repository was
  not found.
- Pull requests were missing their conversation comments and events (they share
  the issue number space).
- `--discussions` and `--projects` called REST routes that do not exist and logged
  "feature not enabled" for every repository; they now log one clear warning per run
  (the flags still back up nothing; GitHub offers Discussions through GraphQL only and
  Classic Projects are retired).
- Webhook URLs are logged as `scheme://host` only.
- Docker and Compose: see Packaging below.

### Removed

- Classic Projects and the REST Discussions calls (neither worked), the extra
  `/topics` request per repository, `GitHubClient`'s unused HTTP/2 feature,
  the `rustsec/audit-check` CI job (cargo-deny covers advisories) and the
  Dependabot Docker ecosystem (versions are now build arguments).

- The `GIT_ASKPASS` script and the PID-file lock (and with it the workspace's only
  `unsafe` block).
- The effect of `--keep-last` and `--max-age-days` (deprecated, see above).
- Compose: the bundled `minio/minio` service and the default `config.toml` bind
  mount.

### Security

- **GitHub App keys** are signed with `aws-lc-rs` (already part of the build
  through rustls); the key and the minted tokens are wiped from memory when
  dropped and never logged.
- **Attachment downloads** send the token only to `https://github.com`, only
  from a client configured for github.com, and never to the storage host GitHub
  redirects to.

- **Encrypted S3 objects** use the STREAM construction over AES-256-GCM in 64 KiB
  segments with a per-object key from HKDF-SHA256 and a random salt (the layout of
  the `age` file format), so truncation and reordering are detected and memory
  is bounded.
- **Every configured secret** (GitHub token, mirror token, S3 keys, encryption
  key, webhook URL) is removed by value from logs, reports and notifications;
  tokens used as a URL's user name and GitLab `glpat-` tokens are redacted too.
- `--help` no longer prints the webhook URL from `BACKUP_NOTIFY_WEBHOOK`.
- `temp_clone_token`, a short-lived credential GitHub can embed in any
  repository object, is removed from every API response at every depth before
  anything is stored (GitHub's API description lists it on repository objects,
  so it could reach `repos.json`, `info.json` and any file
  holding a nested repository object, such as a pull request's `base.repo`); configuration and proxy `Debug` output redact secrets; a
  next-page link to another origin is not followed with the token; the mirror API
  refuses `http://` URLs and empty tokens.
- Starred repositories named like a path are refused instead of cloned outside
  the backup directory.
- **CI hardening:** every action pinned by commit SHA, least-privilege
  permissions, no persisted checkout credentials, no build cache in releases,
  a zizmor workflow lint, and Windows tests.

- **Git credentials** travel in an environment variable of the git child plus an
  inline credential helper bound to the clone URL's origin (after resetting
  `credential.helper`): never in an argument list, never on disk, never offered to
  another host, never persisted by a configured helper.  Mirror pushes use the same
  mechanism.  Commands in an existing repository trust exactly that path through
  `-c safe.directory=<path>`.
- Failure messages, logs and reports are scrubbed of tokens and URL credentials.
- Release-asset downloads never forward the credential across origins.
- Mirrors are private by default (`--mirror-public` opts public sources in) and never
  overwrite a repository the tool did not create.
- S3: keyed (HMAC) content digests for encrypted uploads so the bucket learns no
  unkeyed hash of the plaintext; delete-stale guarded as described above; credentials
  validated and kept out of `Debug` output.
- `github-backup-core` is `#![forbid(unsafe_code)]`.
- Dependencies updated past five open advisories.
- **Lock file refreshed past five more advisories** that had turned the scheduled
  audit red: `rustls` 0.23.46 and `rustls-webpki` 0.103.17 (RUSTSEC-2026-0346,
  -0347, -0348, -0349 and -0382).  `chrono`, declared unmaintained
  (RUSTSEC-2026-0543), is replaced by `jiff`.  No advisory is ignored any more:
  the stale `RUSTSEC-2026-0097` entries in `.cargo/audit.toml` and the CI audit job
  no longer matched anything in `Cargo.lock` (reported in issue #37).
- **Token redaction** in error output (`<prefix>_<redacted>`), and the credential
  type's `Debug` output no longer prints the token.

### Packaging, CI and release

- **Release:** Linux binaries are static musl builds made by the Dockerfile's
  `export` stage (native on x86_64 and aarch64, no `cross`), checked with `file` and
  `ldd` and run with `--version` before upload; `--locked` everywhere; a pinned
  toolchain; a tag must point at a commit on `main`.  Each of the five binaries gets a
  signed GitHub build-provenance attestation (`actions/attest`; the checksum files,
  `SHA256SUMS.txt` and the container images are not attested; no SBOM).  CI builds and
  smoke-tests the Docker image; Dependabot and a daily advisory audit were added.
- **Image:** `alpine:3.23` runtime with `git`, `git-lfs`, `openssh-client`,
  `ca-certificates` and `tini`; numeric `USER 1000:1000` (uid 99 has a passwd entry
  for Unraid); `safe.directory` trusts `/backup` only; allow-list `.dockerignore`;
  `UMASK` support; the dependency layer compiles third-party crates.
- **Entrypoint wrapper** (`docker/entrypoint.sh`): arguments pass through verbatim;
  without arguments it builds them from `GITHUB_OWNER`, `BACKUP_MODE` and
  `BACKUP_FLAGS`; empty optional variables are dropped; shell metacharacters in
  `BACKUP_FLAGS` and `BACKUP_MODE` are refused (exit 64).
- **Compose:** profiles `doctor`, `tui`, `verify`, `s3`, `b2`, `minio` (an S3 server
  you run), `codeberg` and `gitlab`; every service inherits the full base
  environment (the B2/MinIO profiles previously uploaded unencrypted even with
  `BACKUP_ENCRYPT_KEY` set); fixed flags live in `entrypoint:` so `run SERVICE OWNER
  --all` keeps them; empty variables are no longer injected.
- **Unraid:** Community Applications template with every important option as a form
  field, runs as 99:100, no `--rm` (so `docker start` works for scheduled runs),
  corrected `Category` and `Support`, `ca_profile.xml` in the repository root.
  The template's first entry claimed it matched binary v0.3.2; the run modes
  `--doctor`, `--check`, `--list-scopes`, `--print-config-template` and the
  entrypoint wrapper are not in the v0.3.2 image.

### Documentation

The documentation was rewritten against the code (every statement checked by
running the binary or reading the source): CLI reference and exit codes, config
precedence, incremental runs and the state file, monitoring (report, metrics,
webhook), restore, S3 and encryption, mirroring, Docker and Unraid, security,
troubleshooting and the operations runbook.  Duplicated pages were merged (`DOCKER.md`,
`ARCHITECTURE.md` and `CONTRIBUTING.md` are now the single sources of their book
pages).  The README and the book describe `main`; a version note says how that
differs from v0.3.2.

---

## [0.3.2] — 2026-04-12

Maintenance release focused on the release pipeline, distribution
strategy, supply-chain hardening, mutation-testing coverage, and
CI/docs polish. No runtime behaviour changes for end users; all
existing configurations and command-line flags are unchanged.

### Changed

- **Release distribution: dropped crates.io; releases are now binary +
  Docker only.** Every workspace crate is marked `publish = false`,
  the `publish` / `publish-dry-run` / `package` jobs and the protected
  `crates-io` GitHub Environment have been removed from
  `.github/workflows/release.yml`, and `CARGO_REGISTRY_TOKEN` is no
  longer referenced. Users install via one of three methods, in
  recommended order:
    1. Pre-built binary from the GitHub Releases page (five targets:
       Linux x86_64/aarch64, macOS x86_64/aarch64, Windows x86_64),
       each with a `.sha256` checksum and SLSA Level 2 build
       provenance attestation (**correction added later:** this was wrong
       for the binaries.  The attestation step of the v0.3.2 workflow covered
       only the `.crate` archives, which were uploaded as workflow artifacts and
       not published; the v0.3.2 release binaries have no attestation).
    2. Multi-arch container image from GHCR
       (`ghcr.io/tomtom215/github-backup-rust:<tag>`), wired up with
       a Docker Compose file at the repo root that supports local,
       S3, B2, MinIO, and Codeberg profiles and reads secrets from a
       `.env` file (template: `compose.example.env`).
    3. `cargo install --git https://github.com/tomtom215/github-backup-rust --tag v0.3.2 github-backup`.
  The rationale is that `github-backup` is a CLI application, not a
  reusable library — the workspace split exists for internal code
  organisation, not for third-party consumption. Dropping crates.io
  removes the operational burden of seven name reservations, seven
  docs.rs builds, and per-release intra-workspace version-pin
  synchronisation.
- **Multi-stage release pipeline** (`.github/workflows/release.yml`):
  `validate → ci → security → binaries → github-release → docker`.
  The validate job enforces semver tag format, checks
  `[workspace.package].version` against the tag, verifies every
  intra-workspace dependency line (if any carry `version =` pins),
  and requires a matching `## [X.Y.Z]` CHANGELOG entry before any
  build runs.
- **Intra-workspace `version =` pins dropped** from the root
  `Cargo.toml`. With every member marked `publish = false`, only the
  `path =` source is used to resolve internal dependencies, so there
  is nothing to keep in lock-step every release.
- **Portfolio-grade docs, manifests, and CI polish** (PR #16): the
  root `README.md`, `ARCHITECTURE.md`, every per-crate `Cargo.toml`
  (descriptions, keywords, categories, documentation links), the full
  mdBook (`docs/src/**`), `Dockerfile`, `clippy.toml`, and `deny.toml`
  have been reviewed and aligned. `README.md` and
  `docs/src/getting-started/installation.md` have been rewritten
  around the new binary / Docker / source install story.
- **`docker-compose.yml` now pulls from GHCR by default** via a
  shared YAML anchor (`image: ghcr.io/tomtom215/github-backup-rust`),
  mounts `./backups` → `/backup` and `./config.toml` →
  `/etc/github-backup/config.toml`, and fails fast if required
  secrets are missing from `.env`. `build: .` remains available as a
  commented-out override for local development. A new
  `compose.example.env` template ships at the repo root.
- **`mdBook` pinned to `0.4.40`** in CI and the Pages workflow to
  match the version that the `mdbook-linkcheck` backend is known to
  be compatible with.
- **Pages deployment publishes `docs/book/html/`** instead of
  `docs/book/`. Enabling the `linkcheck` backend causes mdBook to
  emit each backend into its own subdirectory, so the HTML tree is
  now one level deeper.

### Added

- **SLSA Level 2 build provenance attestations** for every pre-built
  binary produced by the release pipeline, verifiable with
  `gh attestation verify` (**correction added later:** not true of the
  release binaries; see the note under *Changed* above.  Attestation of the
  binaries starts with the first release after v0.3.2).
- **Mutation-testing configuration** (`.cargo/mutants.toml`): curated
  exclude list for generated / panic-only / `#[cfg(...)]`-gated code
  paths so `cargo mutants` produces actionable survivors only. A
  `workflow_dispatch`-only CI job runs the full mutation suite
  on-demand without blocking every push to main.
- **Pagination malformed-URL tests** in `github-backup-client` and
  **rate-limit edge-case tests** to catch mutants that would
  otherwise silently degrade GitHub API retry/backoff behaviour.
- **`BackupRunHistory::push` regression tests** covering the
  deduplication and ordering mutants flagged by `cargo mutants`.
- **`cargo-audit` configuration** (`.cargo/audit.toml`) with an
  explicit ignore entry and rationale for the informational `rand`
  advisory that does not affect this project.

### Fixed

- **Release workflow `validate` job could never succeed**: the awk
  extractor for `[workspace.package].version` used the greedy regex
  `.*"` to strip the `version = "` prefix, which matched through the
  closing quote and left an empty string. The job then always
  reported "could not parse `[workspace.package].version`" and
  failed before comparing against the tag. Replaced with `^[^"]*"`
  so the strip stops at the first quote.
- **`Deploy Book to GitHub Pages` workflow failed with "The command
  `mdbook-linkcheck` wasn't found"**: `book.toml` enables
  `[output.linkcheck]`, but the Pages workflow only installed
  `mdbook` itself. The link checker is now installed alongside
  `mdBook`, matching the existing `ci.yml` step.
- **`is_process_alive` false positive on macOS**: `kill(0, 0)`
  returns success on BSDs for PID 0 (the kernel swapper), which
  caused the lockfile staleness check to treat a stale lock with
  PID 0 as still held. The check now rejects PID 0 explicitly.
- **macOS CI permissions test** that relied on `chmod 000` being
  honoured when the test runs as root.
- **`cargo-deny` `advisories` check now runs with `contents: read`
  permissions** in CI; the previous implicit default blocked the
  step on pull requests from forks.
- **`BackupEvent::RepoCompleted` missing error field** and related
  `dead_code` clippy errors surfaced by the mutation-testing work.
- **Rustdoc intra-doc link ambiguity** for the `write` module and
  several broken internal mdBook links that surfaced after the
  link-check backend was added.

### Security

- **Dependency audit**: all advisories reviewed and either fixed,
  upgraded away, or explicitly ignored with justification in
  `.cargo/audit.toml`. The release pipeline now runs
  `cargo-deny check licenses bans advisories sources` as a gating
  job before any binary or container image is produced.

---

## [0.3.1] — 2026-03-29

### Added

- **`--restore` mode (labels, milestones, and issues)**: the `--restore` flag
  reads every repository's `labels.json`, `milestones.json`, and `issues.json`
  from the backup and re-creates them in the target organisation via the
  GitHub REST API.  Pull requests embedded in `issues.json` are skipped.
  Existing resources (HTTP 422) are silently skipped.  Requires
  `--restore-target-org` and a token with repository write access; an
  interactive confirmation banner is printed unless `--restore-yes` is
  supplied.

- **AES-256-GCM at-rest encryption for S3** (`--encrypt-key`): provide a
  32-byte hex key (64 hex chars) and every file is encrypted with AES-256-GCM
  before upload.  The wire format is
  `[12-byte random nonce][ciphertext + 16-byte tag]`.  Encrypted objects
  receive a `.enc` suffix in S3.  The key may also be supplied via the
  `BACKUP_ENCRYPT_KEY` environment variable, and a `--decrypt` mode reverses
  the process locally.

- **`post_process` module**: mirror push, S3 sync, Prometheus metrics, diff,
  and retention logic live in a dedicated `post_process.rs` module in the
  main binary.

- **Write endpoints in `GitHubClient`**: `create_label()`,
  `create_milestone()`, and `create_issue()` use a shared `post_json` helper
  with the same rate-limit and 5xx retry behaviour as the GET path.

- **Interactive TUI** (`--tui`): full-screen terminal interface built with
  [Ratatui](https://ratatui.rs) 0.30.  Five screens — Dashboard, Configure,
  Run, Verify, Results — cover the end-to-end workflow without leaving the
  terminal.  A custom `tracing_subscriber::Layer` routes log lines to the
  Run screen's log panel; a `tokio::sync::oneshot` cancellation channel
  aborts a running backup on `Ctrl+C`.  The TUI crate ships unit tests that
  exercise the full state machine without a real terminal.

- **Config file now covers S3 and mirror settings**: `s3_bucket`,
  `s3_region`, `s3_prefix`, `s3_endpoint`, `s3_access_key`, `s3_secret_key`,
  `s3_include_assets`, `mirror_to`, `mirror_token`, `mirror_owner`, and
  `mirror_private` are valid TOML keys.  All values can be overridden by CLI
  flags.

- **Config file now covers clone behaviour**: `prefer_ssh`, `clone_type`,
  `lfs`, `no_prune`, and `report` are valid TOML keys.

### Changed

- **MSRV raised from 1.85 to 1.88**: `ratatui@0.30` and its transitive
  dependencies require Rust 1.88.  The workspace `rust-version` in
  `Cargo.toml` has been updated accordingly.

- **`deny.toml` allows the `Zlib` licence**: `foldhash@0.2` (a transitive
  dependency of `ratatui-core`) is Zlib-licensed.

- **`s3_region` / `s3_prefix` are now `Option<String>`** in `Args`,
  consistent with `concurrency`.  The defaults (`us-east-1` and `""`) are
  applied at `build_s3_config` time so a config file can supply the values
  when the CLI flags are absent.

### Fixed

- **`org` merge bug**: `merge_config` now applies `cfg.org` when the CLI
  `--org` flag was not passed.  Previously the config-file value was
  silently ignored.

### Internal

- **`repository.rs` split**: inline test module extracted to
  `repository_tests.rs` via the `#[path]` attribute, separating production
  code from its tests.

---

## [0.3.0] — 2026-03-29

### Added

- **`--clone-host <HOST>`** (`GITHUB_CLONE_HOST` env / `clone_host` config
  key): overrides the hostname in every git clone URL returned by the API.
  Intended for GitHub Enterprise Server deployments where the API endpoint
  and the git clone endpoint are on separate hosts.  Applied to repository,
  wiki, and gist clones.

- **`--concurrency` is now truly optional**: `Args::concurrency` is
  `Option<usize>`, so a config-file value such as `concurrency = 8` is no
  longer overridden by the implicit CLI default.

- **`BackupStats::add_gists(n)`**: batch increment replacing the previous
  per-item loop in the engine.

- **`repos_discovered` in `BackupStats::Display`**: the summary line now
  shows `N/M backed up` (backed-up / discovered) so operators can see at a
  glance whether any repositories were skipped or errored.

### Fixed

- **Dead code removed**: `FsStorage::write_bytes_owned` and an unused
  `use bytes::Bytes` import in `storage.rs`.

- **`run_git` signature simplified**: removed the `in_cwd: bool` parameter.
  Callers now pass the working directory directly.

### Internal

- **Module extraction**: inline metadata backup blocks in `engine.rs`
  (`labels`, `milestones`, `hooks`, `security_advisories`, `topics`,
  `branches`) split into dedicated modules under `backup/`.
- **`endpoints/` directory**: `client/endpoints.rs` split into eight focused
  submodules (`actions`, `issues`, `keys`, `org`, `pulls`, `repo_meta`,
  `repos`, `social`).
- **`api_client/` directory**: trait definition (`mod.rs`) split from the
  blanket `impl BackupClient for GitHubClient` (`impl_github.rs`).
- **`config/` directory**: `config.rs` split into `credential`, `output`,
  `clone_type`, `options`, and `file` submodules.
- **`report.rs`**: report-writing helpers extracted from `main.rs` into a
  dedicated module with unit tests.
- **Broken intra-doc link** fixed in `api_client/mod.rs`.

### Added

- **GitHub Actions workflow backup** (`--actions`, `--action-runs`): new
  `Workflow` and `WorkflowRun` types added to `github-backup-types`.  Two new
  client endpoints (`list_workflows`, `list_workflow_runs`) and a dedicated
  backup module (`backup/actions.rs`) in `github-backup-core`.  The engine
  writes `workflows.json` per repository when `--actions` is set, and optionally
  `workflow_runs_<id>.json` per workflow when `--action-runs` is also set.
  Both endpoints handle 403/404 gracefully (Actions disabled, token scope).
  `BackupStats` now tracks `workflows_fetched` and the JSON report includes the
  counter.  `--action-runs` is intentionally excluded from `--all` due to its
  potentially large output.

- **Deployment environment backup** (`--environments`): new `Environment`,
  `EnvironmentProtectionRule`, and `DeploymentBranchPolicy` types added to
  `github-backup-types`.  New client endpoint (`list_environments`) and backup
  module (`backup/environments.rs`) write `environments.json` per repository.
  404/403 responses (no environments or insufficient permissions) are logged
  and skipped gracefully.

- **TOML config file** (`--config` / `-c`): supply any backup option through a
  `config.toml` file; CLI flags always take precedence.  The new `ConfigFile`
  type in `github-backup-types` is parsed with the `toml` crate and merged into
  `Args` before the backup starts.
- **Backup summary report** (`--report <FILE>`): write a machine-readable JSON
  summary of the run to an arbitrary path after the backup completes.  The
  report now includes `tool_version`, `started_at` (ISO 8601), `duration_secs`,
  per-category counters, and a `success` boolean — useful for monitoring and
  alerting integrations.
- **Modular CLI**: `cli.rs` (724 lines) refactored into:
  - `cli/args.rs` — `Args` struct, `merge_config()`, `into_backup_options()`
  - `cli/clone_type.rs` — `CliCloneType` parser
  - `cli/mod.rs` — re-exports
- **Modular git runner**: `git.rs` (600 lines) refactored into:
  - `git/mod.rs` — `CloneOptions`, `GitRunner` trait, `ProcessGitRunner`
  - `git/askpass.rs` — `AskpassScript` RAII guard
  - `git/spy.rs` — `SpyGitRunner` test stub + tests
- **Repository name filters** (`--include-repos` / `--exclude-repos`): back
  up only a subset of repositories using glob patterns (`*` / `?`), matching
  is case-insensitive.  Patterns can be comma-separated or the flag can be
  repeated.  `--exclude-repos` takes precedence over `--include-repos`.
- **`--since <DATETIME>`**: limit issue and pull-request API calls to items
  updated at or after an ISO 8601 timestamp.  Enables efficient incremental
  backups — re-use `started_at` from the previous run's JSON report.
- **Topics backup** (`--topics`): write `topics.json` (repository tags) per
  repository.  Already had a `GitHubClient` endpoint; now wired end-to-end
  through the `BackupClient` trait and the engine.
- **Branch list backup** (`--branches`): write `branches.json` per repository
  containing all branch names, tip SHA-1s, and protection status.  New
  `Branch` / `BranchCommit` types added to `github-backup-types`.
- **`BackupStats::elapsed_secs()`**: wall-clock duration tracking using
  `std::time::Instant`; displayed in the `Display` output and included in the
  JSON report.
- **GitHub Pages deployment** (`pages.yml`): CI workflow that builds the
  mdBook and deploys it to the `github-pages` environment on every push to
  `main`.
- **Full mdBook documentation** in `docs/` covering installation, quick
  start, authentication, all backup categories, storage backends,
  configuration, deployment, monitoring, security, troubleshooting, and the
  workspace architecture.

- **GitHub Enterprise Server** support via `--api-url <URL>` (or
  `GITHUB_API_URL` environment variable / `api_url` config file key).  Pass
  the GHES API base URL (e.g. `https://github.example.com/api/v3`) and all API
  requests are directed there.  New `GitHubClient::with_api_url()` constructor
  added to `github-backup-client`.
- **Extended backup stats**: `BackupStats` now tracks `issues_fetched` and
  `prs_fetched` across all repositories.  Both counters appear in the log
  output, the `Display` summary, and the JSON report (`--report`).
- **`--since` format validation**: the ISO 8601 value is now validated before
  the backup starts, producing a clear error for malformed timestamps.
- **`dry_run` gap fixed**: `backup_gists` and `backup_user_data` now respect
  `opts.dry_run` and skip all I/O in dry-run mode (previously only
  per-repository operations were skipped).
- **Modular code**: `config.rs` split into `config.rs` + `glob.rs`; `args.rs`
  split into `args.rs` (struct) + `args_impl.rs` (`merge_config` / `into_backup_options`).

### Changed

- `owner` positional argument is now optional; it can be supplied via the
  `owner` key in the config file instead.
- `--output` flag now defaults to `.` when not specified via CLI or config.
- `BackupClient::list_issues` and `BackupClient::list_pull_requests` now
  accept an optional `since: Option<&str>` parameter (used by `--since`).
- `BackupOptions::all()` now also enables `topics` and `branches`.
- `BackupStats::Display` now includes elapsed time, issues fetched, and PRs
  fetched.
- `backup_issues` and `backup_pull_requests` return `u64` (count of items
  fetched) instead of `()`.  The engine uses these to populate `BackupStats`.

---

## [0.2.0] — 2026-01-15

### Added

- **OAuth device flow**: `--device-auth` + `--oauth-client-id` enable
  interactive authentication via GitHub's device authorisation flow without
  creating a long-lived PAT.
- **Gitea/Codeberg/Forgejo mirror push**: after the primary backup, push every
  cloned repository as a mirror to a Gitea-compatible instance using
  `--mirror-to`, `--mirror-token`, `--mirror-owner`, and `--mirror-private`.
- **S3-compatible storage sync**: `--s3-bucket` (plus region, prefix, endpoint,
  access-key, secret-key flags) syncs JSON metadata — and optionally binary
  release assets — to any S3-compatible object store.  Uses a pure-Rust SigV4
  implementation; no AWS SDK or OpenSSL required.
- **Incremental S3 sync**: `HeadObject` checks before each `PutObject` so
  already-uploaded objects are skipped on subsequent runs.
- **Shallow clone** support via `--clone-type shallow:<depth>`.
- **Git LFS** support via `--lfs`.
- **Docker**: multi-stage Alpine Dockerfile and `docker-compose.yml` with
  service profiles for S3/B2/MinIO/Codeberg.
- **`BackupStats`**: lock-free `AtomicU64` counters shared across concurrent
  repository backup tasks.
- `ARCHITECTURE.md` and `DOCKER.md` documentation.

### Changed

- `BackupEngine` is now generic over `Storage` and `GitRunner` for compile-time
  dispatch and zero-overhead testability.

---

## [0.1.0] — 2025-12-01

### Added

- Complete Rust rewrite of the Python `github-backup` reference implementation.
- **Repositories**: `mirror`, `bare`, and `full` clone modes.
- **Issues**: metadata, comments, timeline events.
- **Pull requests**: metadata, review comments, commit lists, reviews.
- **Releases**: metadata + optional binary asset download.
- **Gists**: owned and starred.
- **Wikis**: bare mirror clones.
- **User data**: starred repos, watched repos, followers, following.
- **Repository metadata**: labels, milestones, webhooks, security advisories.
- **Trait-based design**: `Storage`, `GitRunner`, and `BackupClient` traits with
  full in-memory test stubs (`MemStorage`, `SpyGitRunner`, `MockBackupClient`).
- **RAII credential cleanup**: `GIT_ASKPASS` temp scripts are deleted even on
  panic, ensuring no tokens are left on disk.
- **Rate-limit awareness**: automatic backoff on `X-RateLimit-Remaining: 0`.
- **Retry on 5xx**: up to 3 retries with exponential backoff.
- **Concurrent backup**: semaphore-based, configurable with `--concurrency`.
- **Dry-run mode**: `--dry-run` previews what would be backed up.
- **Shell completions**: bash, zsh, fish, PowerShell, elvish.
- **145 unit tests** covering all modules.
- **`proptest`** round-trip tests for all serialised types.
- CI: rustfmt, clippy (`-D warnings`), tests (Ubuntu + macOS), MSRV 1.85,
  `cargo-audit`, `cargo-deny`.
- Dependency policy in `deny.toml`: no OpenSSL, no reqwest, no native-tls.

[Unreleased]: https://github.com/tomtom215/github-backup-rust/compare/v0.3.2...HEAD
[0.3.2]: https://github.com/tomtom215/github-backup-rust/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/tomtom215/github-backup-rust/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/tomtom215/github-backup-rust/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/tomtom215/github-backup-rust/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/tomtom215/github-backup-rust/releases/tag/v0.1.0
