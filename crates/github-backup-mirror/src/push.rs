// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! How a bare clone is pushed, with which credentials, and which safety checks
//! come first.
//!
//! # Credentials
//!
//! The token is passed in an environment variable of the `git` child and
//! answered by an inline credential helper that is URL-scoped to the
//! destination's origin (after `credential.helper=` reset), exactly as the
//! engine does for clones.  No file is created and the token is never in any
//! argument list.
//!
//! # What is pushed
//!
//! Branches and tags only, rather than `git push --mirror`: a mirror clone
//! also holds `refs/pull/*`, `refs/remotes/*` and similar namespaces that the
//! destination rejects (GitHub-style hidden refs), which used to fail the
//! whole push.  The refspecs are forced, so a branch that was force-pushed on
//! GitHub is overwritten in the mirror (as in the local backup).  Branches and
//! tags that no longer exist locally are deleted at the destination only with
//! pruning (`--prune`), matching the local backup, which keeps refs deleted
//! upstream unless `--prune` is given.

use std::collections::HashSet;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::MirrorError;

/// Environment variable carrying the token to the credential helper.
const TOKEN_ENV: &str = "GH_BACKUP_MIRROR_TOKEN";

/// Refspecs of a mirror push: everything under `refs/heads` and `refs/tags`.
const REFSPECS: [&str; 2] = ["+refs/heads/*:refs/heads/*", "+refs/tags/*:refs/tags/*"];

/// `scheme://host[:port]` of an HTTP(S) URL.
fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if host.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{}", host.to_ascii_lowercase()))
}

/// Whether a mirror of `name` is created private: always when the user asked
/// for it, and otherwise unless the source is *known* to be public.  A
/// repository whose visibility is unknown is never published.
pub(crate) fn wants_private(forced: bool, public_repos: &HashSet<String>, name: &str) -> bool {
    forced || !public_repos.contains(name)
}

/// Whether the clone at `repo_path` is shallow.  Git servers refuse pushes
/// that would make the receiving repository shallow
/// (`receive.shallowUpdate` is off by default, and Gitea, Forgejo and GitLab
/// do not turn it on), so such a clone cannot be mirrored.
pub(crate) fn is_shallow(repo_path: &Path) -> bool {
    repo_path.join("shallow").is_file() || repo_path.join(".git/shallow").is_file()
}

/// `-c safe.directory=<repo>`: trust exactly this repository.
fn trust_args(repo_path: &Path) -> Vec<String> {
    match repo_path.canonicalize() {
        Ok(real) => vec![
            "-c".to_owned(),
            format!("safe.directory={}", real.display()),
        ],
        Err(_) => Vec::new(),
    }
}

/// Global git options: trust exactly this repository and offer the token to
/// the destination only.
fn global_args(repo_path: &Path, remote_url: &str, username: &str, has_token: bool) -> Vec<String> {
    let mut args = trust_args(repo_path);
    if let (true, Some(origin)) = (has_token, origin(remote_url)) {
        let helper = format!(
            "!f() {{ test \"$1\" = get && printf 'username={username}\\npassword=%s\\n' \"${TOKEN_ENV}\"; }}; f"
        );
        args.extend([
            "-c".to_owned(),
            "credential.helper=".to_owned(),
            "-c".to_owned(),
            format!("credential.{origin}.helper={helper}"),
        ]);
    }
    args
}

/// Whether the clone at `repo_path` has at least one branch or tag.
pub(crate) fn has_branches_or_tags(repo_path: &Path) -> Result<bool, MirrorError> {
    let args = ["for-each-ref", "--count=1", "refs/heads", "refs/tags"];
    let output = Command::new("git")
        .args(trust_args(repo_path))
        .arg("-C")
        .arg(repo_path)
        .args(args)
        .stdin(Stdio::null())
        .env("LC_ALL", "C")
        .output()
        .map_err(MirrorError::GitSpawn)?;
    if !output.status.success() {
        return Err(MirrorError::GitFailed {
            args: args.join(" "),
            code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(!output.stdout.is_empty())
}

/// Pushes the branches and tags of the bare clone at `repo_path` to
/// `remote_url`; with `prune`, also deletes remote branches and tags that no
/// longer exist locally.
pub(crate) fn push_refs(
    repo_path: &Path,
    remote_url: &str,
    token: &str,
    username: &str,
    prune: bool,
) -> Result<(), MirrorError> {
    push_refs_with("git", repo_path, remote_url, token, username, prune)
}

fn push_refs_with(
    program: &str,
    repo_path: &Path,
    remote_url: &str,
    token: &str,
    username: &str,
    prune: bool,
) -> Result<(), MirrorError> {
    let has_token = !token.is_empty();
    let mut cmd = Command::new(program);
    cmd.args(global_args(repo_path, remote_url, username, has_token))
        .arg("-C")
        .arg(repo_path)
        .arg("push");
    if prune {
        cmd.arg("--prune");
    }
    cmd.arg(remote_url)
        .args(REFSPECS)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C");
    if has_token && origin(remote_url).is_some() {
        cmd.env(TOKEN_ENV, token);
    }

    let output = cmd.output().map_err(MirrorError::GitSpawn)?;
    if output.status.success() {
        return Ok(());
    }
    let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !token.is_empty() {
        stderr = stderr.replace(token, "[redacted]");
    }
    Err(MirrorError::GitFailed {
        args: format!(
            "push{} {remote_url} <refs/heads, refs/tags>",
            if prune { " --prune" } else { "" }
        ),
        code: output.status.code().unwrap_or(-1),
        stderr,
    })
}

/// Refuses to push into a repository this tool did not create.
///
/// `description` is what the destination reports; `expected` is the marker
/// written when the mirror was created.  A repository that is still empty has
/// nothing to lose and is accepted.
pub(crate) fn verify_ours(
    repo: &str,
    description: Option<&str>,
    empty: bool,
    expected: &str,
) -> Result<(), MirrorError> {
    if empty || description.map(str::trim) == Some(expected) {
        return Ok(());
    }
    Err(MirrorError::ForeignRepository {
        repo: repo.to_owned(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    pub(crate) fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// A bare "mirror clone" `<dir>/<name>.git` holding two commits on
    /// `main`, a tag `v1` and a `refs/pull/1/head`.
    pub(crate) fn source_repo(dir: &Path, name: &str) -> PathBuf {
        let work = dir.join(format!("{name}-work"));
        std::fs::create_dir(&work).expect("mkdir");
        git(&work, &["init", "-q", "-b", "main"]);
        for message in ["one", "two"] {
            git(
                &work,
                &[
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@example.invalid",
                    "commit",
                    "-q",
                    "--allow-empty",
                    "-m",
                    message,
                ],
            );
        }
        git(&work, &["tag", "v1"]);
        git(&work, &["update-ref", "refs/pull/1/head", "HEAD"]);
        let bare = dir.join(format!("{name}.git"));
        let out = Command::new("git")
            .args(["clone", "-q", "--mirror"])
            .arg(&work)
            .arg(&bare)
            .output()
            .expect("clone");
        assert!(out.status.success());
        std::fs::remove_dir_all(&work).expect("rm work tree");
        bare
    }

    /// A destination that, like GitHub, refuses `refs/pull/*`.
    fn refusing_destination(root: &Path) -> PathBuf {
        let dest = root.join("dest.git");
        git(root, &["init", "-q", "--bare", dest.to_str().unwrap()]);
        let hook = dest.join("hooks/pre-receive");
        std::fs::write(
            &hook,
            "#!/bin/sh\nwhile read old new ref; do case \"$ref\" in refs/pull/*) echo \"deny $ref\" >&2; exit 1;; esac; done\n",
        )
        .expect("hook");
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        dest
    }

    #[test]
    fn pushes_branches_and_tags_but_not_pull_refs_and_prunes_only_when_asked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = source_repo(dir.path(), "src");
        assert!(git(&src, &["for-each-ref"]).contains("refs/pull/1/head"));
        let dest = refusing_destination(dir.path());
        let url = dest.to_str().unwrap();
        let refs = || git(&dest, &["for-each-ref", "--format=%(refname)"]);

        git(&src, &["tag", "stale"]);
        push_refs(&src, url, "", "x", false).expect("first push succeeds");
        git(&src, &["tag", "-d", "stale"]);

        // Without pruning, a tag deleted at the source stays in the mirror.
        push_refs(&src, url, "", "x", false).expect("second push succeeds");
        assert!(refs().contains("refs/tags/stale"), "{}", refs());

        // With pruning it goes.
        push_refs(&src, url, "", "x", true).expect("pruning push succeeds");
        let refs = refs();
        assert!(refs.contains("refs/heads/main"), "{refs}");
        assert!(refs.contains("refs/tags/v1"), "{refs}");
        assert!(!refs.contains("refs/pull"), "{refs}");
        assert!(!refs.contains("stale"), "stale tag must be pruned: {refs}");
    }

    /// The premise for skipping shallow clones: a push from one into a fresh
    /// repository is refused (`receive.shallowUpdate` is off by default).
    #[test]
    fn git_refuses_a_push_from_a_shallow_clone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = source_repo(dir.path(), "src");
        let shallow = dir.path().join("shallow.git");
        let url = format!("file://{}", src.display());
        git(
            dir.path(),
            &[
                "clone",
                "-q",
                "--mirror",
                "--depth",
                "1",
                &url,
                shallow.to_str().unwrap(),
            ],
        );
        assert!(is_shallow(&shallow));
        assert!(!is_shallow(&src));
        let dest = dir.path().join("dest.git");
        git(
            dir.path(),
            &["init", "-q", "--bare", dest.to_str().unwrap()],
        );
        let err = push_refs(&shallow, dest.to_str().unwrap(), "", "x", false)
            .expect_err("shallow push is refused");
        assert!(matches!(err, MirrorError::GitFailed { .. }), "{err:?}");
    }

    #[test]
    fn has_branches_or_tags_sees_only_branches_and_tags() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = source_repo(dir.path(), "src");
        assert!(has_branches_or_tags(&src).unwrap());
        git(&src, &["update-ref", "-d", "refs/heads/main"]);
        assert!(has_branches_or_tags(&src).unwrap(), "the tag is left");
        git(&src, &["update-ref", "-d", "refs/tags/v1"]);
        assert!(
            !has_branches_or_tags(&src).unwrap(),
            "refs/pull/* does not count"
        );
        assert!(has_branches_or_tags(&dir.path().join("missing")).is_err());
    }

    #[test]
    fn a_failed_push_is_an_error_without_the_token() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = source_repo(dir.path(), "src");
        let err = push_refs(
            &src,
            dir.path().join("missing.git").to_str().unwrap(),
            "tok",
            "x",
            false,
        )
        .expect_err("destination does not exist");
        assert!(matches!(err, MirrorError::GitFailed { .. }), "{err:?}");
    }

    #[test]
    fn token_is_in_the_environment_only_and_scoped_to_the_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let seen = dir.path().join("seen.txt");
        let script = dir.path().join("fake-git");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\n{{ printf '%s ' \"$@\"; echo; echo \"env=$GH_BACKUP_MIRROR_TOKEN\"; }} > '{}'\n",
                seen.display()
            ),
        )
        .expect("write");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        push_refs_with(
            script.to_str().unwrap(),
            dir.path(),
            "https://codeberg.example/alice/r.git",
            "DUMMY_MIRROR_TOKEN_123",
            "oauth2",
            false,
        )
        .expect("push");
        let text = std::fs::read_to_string(&seen).expect("seen");
        let (cmdline, env) = text.split_once('\n').expect("two lines");
        assert!(!cmdline.contains("DUMMY_MIRROR_TOKEN_123"), "{cmdline}");
        assert!(cmdline.contains("-c credential.helper= "), "{cmdline}");
        assert!(
            cmdline.contains("credential.https://codeberg.example.helper="),
            "{cmdline}"
        );
        assert!(cmdline.contains("username=oauth2"), "{cmdline}");
        assert!(cmdline.contains("safe.directory="), "{cmdline}");
        assert!(env.contains("env=DUMMY_MIRROR_TOKEN_123"), "{env}");
        assert!(!cmdline.contains("--mirror"), "{cmdline}");
        assert!(
            !cmdline.contains("--prune"),
            "no pruning by default: {cmdline}"
        );
    }

    #[test]
    fn visibility_defaults_to_private() {
        let public: HashSet<String> = ["open".to_owned()].into();
        assert!(
            wants_private(false, &public, "secret"),
            "unknown => private"
        );
        assert!(!wants_private(false, &public, "open"));
        assert!(
            wants_private(true, &public, "open"),
            "--mirror-private wins"
        );
    }

    #[test]
    fn only_our_own_or_empty_repositories_are_pushed_into() {
        let marker = "GitHub mirror of octo/r";
        assert!(verify_ours("r", Some(marker), false, marker).is_ok());
        assert!(verify_ours("r", Some("my own project"), true, marker).is_ok());
        assert!(verify_ours("r", Some("my own project"), false, marker).is_err());
        assert!(verify_ours("r", None, false, marker).is_err());
    }
}
