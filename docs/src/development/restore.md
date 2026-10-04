# Restore Implementation

How `--restore` works internally.  For using it, see the
[Restore Guide](../restore.md); this page does not repeat the procedures.

The code is `crates/github-backup/src/restore.rs` (the mode) and
`crates/github-backup-client/src/client/endpoints/write.rs` (the three write
calls: `create_label`, `create_milestone`, `create_issue`).

---

## Control Flow

1. `main` parses the arguments and resolves the credential.  With `--restore` it
   calls `run::execute_restore` **instead of** the backup: no engine, no
   post-processing, no lock, no contact with the source owner.
2. `confirm_restore` prints the warning banner and returns `true` if
   `--restore-yes` is set, `GITHUB_BACKUP_RESTORE_YES=1` is set, or the user
   typed `yes` on a terminal.  Without a terminal and without either escape
   hatch it prints both and the run ends with exit status `1`.  `--dry-run`
   skips the confirmation.
3. `run_restore` reads `<output>/<owner>/json/repos/`; if that directory does not
   exist the restore fails (exit `1`: "no backup of '<owner>' found").
4. For every repository directory, `restore_repo` restores labels, then
   milestones, then issues.  Errors are counted per resource kind and never
   abort the loop.
5. The totals are logged; exit status is `3` if any resource errored, else `0`.

## Per-Resource Rules

| Resource | Request | Already there | Notes |
|----------|---------|---------------|-------|
| Label | `POST /repos/{org}/{repo}/labels` (name, colour, description) | HTTP 422: counted as skipped | |
| Milestone | `POST /repos/{org}/{repo}/milestones` (title, description, state, due date) | HTTP 422: skipped | |
| Issue | `POST /repos/{org}/{repo}/issues` (title, body, label names) | marker found in the target: skipped | pull requests (`pull_request` set) are skipped and counted |

* **Marker.**  Before restoring a repository's issues the tool pages through the
  target's issues (`state=all`) and collects every
  `<!-- github-backup-restore:<owner>/<repo>#<number> -->` found in a body.  A
  backed-up issue whose marker is present is skipped.  Each created issue's body
  is the original text, a rule, an italic "Restored by github-backup from ..."
  line (original author and creation time) and the marker.
* **No restore of state.**  The create-issue request carries no state,
  assignees or milestone: closed issues are re-created open.  Issue numbers
  are assigned by GitHub and differ from the originals.
* **Missing target repository.**  Listing the target's issues fails, the
  repository's issues are counted as errored and skipped; its labels and
  milestones fail the same way, one request each.
* **Order.**  Labels before issues so that label names exist.  Label names are
  passed to the API as they are.
* **Dry run.**  Counts what would be created, makes no request and does not
  look for markers.

The JSON is read through the typed models (`Label`, `Milestone`, `Issue`);
the files are lossless copies of GitHub's responses, and these types ignore the
properties restore does not need.

## Why Pull Requests Are Not Restored

GitHub offers no API to create a pull request from historic data (authors,
timestamps, review state, merged status).  The data stays in `pulls.json`,
`pull_*` and `issue_*` files for reference.

## Tests

`restore.rs` has unit tests for the statistics, JSON loading, the body and marker
format (including finding markers again), the error total, and a dry run that
makes no API call.  There is no automated test of the live marker lookup or of
the exit statuses; those were exercised by hand against a local fake GitHub
server during the documentation review, which is not part of the repository.
