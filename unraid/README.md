# Unraid deployment

`github-backup-rust` ships an [Unraid](https://unraid.net) [Community
Applications](https://docs.unraid.net/unraid-os/using-unraid-to/run-docker-containers/community-applications/)
template so users can install and configure it from the Unraid WebUI
without touching the terminal.

```
unraid/
├── github-backup.xml     ← the template (this is what you submit to CA)
├── ca_profile.xml        ← developer profile picked up by CA
├── icon.png              ← 256×256 PNG referenced by <Icon> in the template
└── README.md             ← this file
```

The template targets **Unraid v7.0.0 and newer** (`<MinVer>7.0.0</MinVer>`)
and has been authored against the v7.2.x DockerMan / CA conventions
documented at <https://docs.unraid.net/unraid-os/using-unraid-to/run-docker-containers/>.

## What the template gives the user

| WebUI field                       | Variable / mount        | Default                          | Notes |
|-----------------------------------|--------------------------|----------------------------------|-------|
| Output Directory                  | `/backup` (Path)         | `/mnt/user/backups/github/`      | Required. Point at an Unraid share. |
| GitHub Owner                      | `GITHUB_OWNER`           | _(empty)_                        | Required. User / org to back up. |
| GitHub Token                      | `GITHUB_TOKEN`           | _(empty)_                        | Required, **masked**. |
| Run Mode                          | `BACKUP_MODE` (dropdown) | `--all`                          | `--doctor`, `--check`, `--list-scopes`, `--verify`, `--tui`, `--print-config-template` also available. |
| Extra CLI Flags                   | `BACKUP_FLAGS`           | _(empty)_                        | e.g. `--org --concurrency 8 --include-repos 'rust-*'`. Shell metacharacters refused. |
| GitHub API URL (GHES)             | `GITHUB_API_URL`         | _(empty)_                        | Advanced. |
| GitHub Clone Host (split GHES)    | `GITHUB_CLONE_HOST`      | _(empty)_                        | Advanced. |
| OAuth App Client ID               | `GITHUB_OAUTH_CLIENT_ID` | _(empty)_                        | Advanced. Pair with `--device-auth` in Extra CLI Flags. |
| At-Rest Encryption Key            | `BACKUP_ENCRYPT_KEY`     | _(empty)_                        | Advanced, **masked**. 32-byte hex; generate with `openssl rand -hex 32`. |
| Notification Webhook              | `BACKUP_NOTIFY_WEBHOOK`  | _(empty)_                        | Advanced. JSON POST on completion. |
| Log Level                         | `RUST_LOG`               | `info`                           | Advanced. `info`/`debug`/`trace`/`warn`/`error`. |
| HTTPS Proxy                       | `HTTPS_PROXY`            | _(empty)_                        | Advanced. Honoured by both the API client and git. |

## How the env-var workflow works

CLI / Compose / Kubernetes users invoke the binary directly:

    docker run --rm -e GITHUB_TOKEN=… ghcr.io/tomtom215/github-backup-rust:latest octocat --all

That continues to work unchanged.

Unraid CA, however, fills in env vars from a form — it does not let
the user supply positional arguments. The image therefore ships a tiny
POSIX shell wrapper (`docker/entrypoint.sh`) which behaves as follows:

- **If any positional arguments are supplied → exec verbatim.**
  The CLI / Compose / Kubernetes contract is preserved.
- **Otherwise → reconstruct argv from `GITHUB_OWNER`, `BACKUP_MODE`,
  and `BACKUP_FLAGS`.** This is the Unraid path.
- **Both empty → `github-backup --help`.**

`BACKUP_MODE` is restricted to a whitelist of known run modes plus
any flag starting with `--` (so future modes work without an image
rebuild); shell metacharacters are rejected up-front.

## First run

1. **Install** via Community Applications: search "github-backup" or
   add the template URL directly under *Settings → Community
   Applications → Settings → Add Container → Template URL*:
   `https://raw.githubusercontent.com/tomtom215/github-backup-rust/main/unraid/github-backup.xml`
2. **Fill the form**: at minimum `GitHub Owner`, `GitHub Token`, and
   keep `Run Mode = --doctor` for the first run.
3. **Start the container**. The pre-flight diagnostic runs in a few
   seconds; review the colour-coded output in the container's log
   (Docker tab → click the github-backup icon → Logs).
4. **Change `Run Mode` to `--all`** and start the container again.
   The backup runs to completion and exits.

The container is one-shot: it exits when the backup finishes. The
status in the Docker tab will show "exited (0)" on success.

## Scheduling recurring backups

Unraid does not currently support a built-in scheduler for docker
containers. The community-standard pattern is:

1. Install the **User Scripts** plugin (already in Community
   Applications: <https://forums.unraid.net/topic/48286-plugin-ca-user-scripts/>).
2. Create a new script named e.g. `github-backup-daily`:

   ```sh
   #!/bin/bash
   docker start github-backup
   ```

3. Set the schedule to a cron expression — daily at 02:00 is `0 2 * * *`.

The script returns immediately; the container runs in the background
and writes structured progress to its Docker log. A *successful* run
exits with code 0; *failure* exits with the error category's code
(usually 1) and is visible in the Docker tab.

## Restore

`--restore` is not exposed as a `BACKUP_MODE` option on purpose: it
*writes* to GitHub and we don't want a stray click to recreate
hundreds of issues against the wrong org. To run it, supply explicit
arguments via the *Post Arguments* field on the WebUI edit page (or
exec from the Console):

    --restore --restore-target-org my-other-org --restore-yes

Set `GITHUB_BACKUP_RESTORE_YES=1` if you'd prefer the env-var form.

## Verify a previous backup

Switch `Run Mode` to `--verify` and start the container. It reads the
SHA-256 manifest under the configured output directory and exits 0
when every file matches, non-zero when anything is missing, tampered,
or unexpected.

## Submission to Community Applications

The current (May 2026) CA submission flow:

1. **Open a support thread** at <https://forums.unraid.net/forum/53-docker-containers/>
   and copy its URL into the `<Support>` element of
   `github-backup.xml` (replace the `REPLACE_WITH_SUPPORT_THREAD_ID`
   placeholder). Templates without a working `<Support>` URL get
   blacklisted.
2. **Host this folder publicly** on GitHub — it already is, under
   `tomtom215/github-backup-rust/unraid/`.
3. **Submit** via the CA submission form linked from
   <https://docs.unraid.net/unraid-os/using-unraid-to/run-docker-containers/community-applications/>
   (an Asana form as of 2026; the old Google Form is retired).
   Moderation typically responds within ~48 h.

What the CA moderators check:

- `<Repository>` actually pulls from GHCR (the `release.yml` workflow
  publishes multi-arch images on every tagged release, so this is
  satisfied automatically).
- `<Project>` and `<Support>` URLs both resolve.
- No exotic XML formatting — the template is laid out in DockerMan's
  emitted style (single-line `<Config>` rows, the conventional
  element order).
- Application is open source (MIT, per `LICENSE`).

## Local testing without submitting

You can install the template directly from a local file without
touching the registry:

1. Copy the XML to your Unraid box's `/boot/config/plugins/dockerMan/templates-user/`
   (e.g. via `scp` or the SMB share).
2. Open *Docker → Add Container* in the WebUI; the template will
   appear under "User Templates".
3. Click **Apply**, fill in `GITHUB_TOKEN` + `GITHUB_OWNER`, and
   start.

Any subsequent edit you make in the WebUI is written back to the same
file, so a round-trip through DockerMan is a free linter — diff
against this version to ensure your manual edits do not deviate from
DockerMan's emit style (which is one of the things the CA parser
checks).

## Icon

`icon.png` should be a square PNG, **256×256**, with transparency.
The image is hot-linked from the template via its raw GitHub URL.
Replace the placeholder with a real icon before submitting to CA.

## References

- [Unraid Docs — Community Applications](https://docs.unraid.net/unraid-os/using-unraid-to/run-docker-containers/community-applications/)
- [Selfhosters — writing a CA-compatible template](https://selfhosters.net/docker/templating/templating/)
- [Unraid wiki — DockerTemplateSchema](https://wiki.unraid.net/DockerTemplateSchema)
- [Unraid 7.2 release notes](https://docs.unraid.net/unraid-os/release-notes/7.2.0/)
- Reference templates studied:
  [binhex-rclone](https://raw.githubusercontent.com/binhex/docker-templates/master/binhex/rclone.xml),
  [cmccambridge/mosquitto](https://raw.githubusercontent.com/cmccambridge/unraid-templates/master/cmccambridge/mosquitto-unraid.xml),
  [ibracorp/unraid-templates](https://github.com/ibracorp/unraid-templates)
