// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Push-mirror runner: discovers the local bare clones and pushes each one to
//! a remote Git host (Gitea, Codeberg, Forgejo or GitLab).

use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};

use tracing::{info, warn};

use crate::error::MirrorError;
use crate::push::{has_branches_or_tags, is_shallow, push_refs, wants_private};

/// A mirror destination: creates repositories through its API and says where
/// and how to push.  Implemented by [`GiteaClient`](crate::GiteaClient) and
/// [`GitLabClient`](crate::GitLabClient).
pub trait MirrorDestination: Sync {
    /// Ensures the repository `name` exists and was created by this tool
    /// (see the implementations).
    fn ensure_repo_exists(
        &self,
        name: &str,
        description: &str,
        private: bool,
    ) -> impl Future<Output = Result<(), MirrorError>> + Send;

    /// The HTTPS URL `git push` uses for repository `name`.
    fn repo_clone_url(&self, name: &str) -> String;

    /// The user name `git` presents with the token.
    fn push_username(&self) -> &'static str;

    /// The API token, also used for `git push`.
    fn token(&self) -> &str;

    /// Every repository is created private (see the configurations).
    fn force_private(&self) -> bool;

    /// Delete branches and tags that no longer exist locally.
    fn prune(&self) -> bool;

    /// The destination's base URL, for log lines.
    fn base_url(&self) -> &str;
}

/// What to mirror.
#[derive(Debug, Clone, Copy)]
pub struct MirrorPlan<'a> {
    /// Directory holding the bare clones (`<name>.git`).
    pub repos_dir: &'a Path,
    /// Description given to created repositories, followed by the repository
    /// name; it marks a destination repository as created by this tool.
    pub description_prefix: &'a str,
    /// Repositories known to be public at the source.
    pub public_repos: &'a HashSet<String>,
    /// Repositories whose backup failed in this run: their local clone may be
    /// incomplete or stale, so they are not pushed.
    pub failed_repos: &'a HashSet<String>,
}

/// Statistics from a mirror push run.
#[derive(Debug, Default, Clone)]
pub struct MirrorStats {
    /// Number of repositories successfully pushed.
    pub pushed: usize,
    /// Number of repositories that failed to push.
    pub errored: usize,
    /// One `name: error` line per failed repository.
    pub failures: Vec<String>,
    /// One `name: reason` line per repository that was deliberately not
    /// pushed (its backup failed, it is a shallow clone, or it has no
    /// branches or tags).
    pub skipped: Vec<String>,
}

/// Pushes every bare clone in `plan.repos_dir` to `dest`.
///
/// For each `*.git` directory directly under the directory:
/// 1. Skip it if its backup failed in this run, if it is a shallow clone
///    (Git servers refuse pushes from those), or if it has no branch or tag
///    (nothing to push; with pruning it would empty the mirror).
/// 2. Ensure the repository exists at the destination (creating it if
///    needed); one that this tool did not create is never pushed into.
/// 3. Push branches and tags, deleting those gone locally only when
///    [`MirrorDestination::prune`] is set.
///
/// A repository is created private unless the source is listed in
/// `plan.public_repos` and the destination does not force privacy.
///
/// Errors per repository are logged and collected in
/// [`MirrorStats::failures`]; the remaining repositories are still pushed and
/// the caller decides how a non-empty failure list ends the run.
pub async fn push_mirrors<D: MirrorDestination>(dest: &D, plan: &MirrorPlan<'_>) -> MirrorStats {
    let mut stats = MirrorStats::default();
    let repos = discover_git_repos(plan.repos_dir);
    if repos.is_empty() {
        info!(dir = %plan.repos_dir.display(), "no git repositories found to mirror");
        return stats;
    }
    info!(count = repos.len(), dest = %dest.base_url(), "pushing mirrors");

    let mut not_backed_up = Vec::new();
    let mut shallow = Vec::new();
    for (repo_path, repo_name) in &repos {
        if plan.failed_repos.contains(repo_name) {
            not_backed_up.push(repo_name.as_str());
            stats
                .skipped
                .push(format!("{repo_name}: its backup failed in this run"));
            continue;
        }
        if is_shallow(repo_path) {
            shallow.push(repo_name.as_str());
            stats.skipped.push(format!(
                "{repo_name}: shallow clones cannot be push-mirrored"
            ));
            continue;
        }
        match push_one(dest, plan, repo_path, repo_name).await {
            Ok(true) => {
                stats.pushed += 1;
                info!(repo = %repo_name, "mirror pushed successfully");
            }
            Ok(false) => {
                warn!(repo = %repo_name, "no branches or tags locally; nothing pushed");
                stats
                    .skipped
                    .push(format!("{repo_name}: no branches or tags locally"));
            }
            Err(e) => {
                stats.errored += 1;
                stats.failures.push(format!("{repo_name}: {e}"));
                warn!(repo = %repo_name, error = %e, "mirror push failed, continuing");
            }
        }
    }
    if !not_backed_up.is_empty() {
        warn!(
            repos = %not_backed_up.join(", "),
            "not mirrored because their backup failed in this run; their mirrors keep their \
             previous state"
        );
    }
    if !shallow.is_empty() {
        warn!(
            repos = %shallow.join(", "),
            "shallow clones are not mirrored: Git servers refuse pushes from a shallow \
             repository; use --clone-type mirror or bare for repositories you mirror"
        );
    }
    stats
}

/// Pushes one repository.  `Ok(false)` when it has nothing to push.
async fn push_one<D: MirrorDestination>(
    dest: &D,
    plan: &MirrorPlan<'_>,
    repo_path: &Path,
    repo_name: &str,
) -> Result<bool, MirrorError> {
    if !has_branches_or_tags(repo_path)? {
        return Ok(false);
    }
    let description = format!("{}{repo_name}", plan.description_prefix);
    let private = wants_private(dest.force_private(), plan.public_repos, repo_name);
    dest.ensure_repo_exists(repo_name, &description, private)
        .await?;
    info!(repo = %repo_name, remote = %dest.base_url(), "pushing branches and tags");
    push_refs(
        repo_path,
        &dest.repo_clone_url(repo_name),
        dest.token(),
        dest.push_username(),
        dest.prune(),
    )?;
    Ok(true)
}

/// Discovers all `*.git` directories directly under `dir`, sorted by name.
///
/// Returns a list of `(path, repo_name)` pairs where `repo_name` is the
/// directory name with the `.git` suffix stripped.
fn discover_git_repos(dir: &Path) -> Vec<(PathBuf, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut repos: Vec<(PathBuf, String)> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }
            let name = path.file_name()?.to_string_lossy().into_owned();
            let repo_name = name.strip_suffix(".git")?.to_string();
            Some((path, repo_name))
        })
        .collect();
    repos.sort_by(|a, b| a.1.cmp(&b.1));
    repos
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push::tests::{git, source_repo};
    use std::fs;
    use std::sync::Mutex;
    use tempfile::tempdir;

    #[test]
    fn discover_git_repos_finds_dot_git_directories() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("my-repo.git")).unwrap();
        fs::create_dir(dir.path().join("another.git")).unwrap();
        // A directory without .git should be ignored.
        fs::create_dir(dir.path().join("not-a-repo")).unwrap();
        // A file with .git should be ignored (must be dir).
        fs::write(dir.path().join("file.git"), b"").unwrap();

        let repos = discover_git_repos(dir.path());
        let names: Vec<&str> = repos.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(names, ["another", "my-repo"]);
    }

    #[test]
    fn discover_git_repos_returns_empty_for_missing_dir() {
        let repos = discover_git_repos(Path::new("/nonexistent/path/that/does/not/exist"));
        assert!(repos.is_empty());
    }

    /// A destination whose "remote" is a local bare repository per name.
    struct LocalDest {
        root: PathBuf,
        prune: bool,
        created: Mutex<Vec<String>>,
    }

    impl MirrorDestination for LocalDest {
        async fn ensure_repo_exists(
            &self,
            name: &str,
            _description: &str,
            _private: bool,
        ) -> Result<(), MirrorError> {
            let path = self.root.join(format!("{name}.git"));
            if !path.exists() {
                git(
                    &self.root,
                    &["init", "-q", "--bare", path.to_str().unwrap()],
                );
            }
            self.created.lock().unwrap().push(name.to_owned());
            Ok(())
        }
        fn repo_clone_url(&self, name: &str) -> String {
            self.root
                .join(format!("{name}.git"))
                .to_str()
                .unwrap()
                .to_owned()
        }
        fn push_username(&self) -> &'static str {
            "x"
        }
        fn token(&self) -> &str {
            ""
        }
        fn force_private(&self) -> bool {
            true
        }
        fn prune(&self) -> bool {
            self.prune
        }
        fn base_url(&self) -> &str {
            "local"
        }
    }

    fn dest(root: &Path, prune: bool) -> LocalDest {
        fs::create_dir_all(root).unwrap();
        LocalDest {
            root: root.to_owned(),
            prune,
            created: Mutex::default(),
        }
    }

    fn refs(repo: &Path) -> String {
        git(repo, &["for-each-ref", "--format=%(refname)"])
    }

    async fn run(dest: &LocalDest, repos_dir: &Path, failed: &[&str]) -> MirrorStats {
        let public = HashSet::new();
        let failed: HashSet<String> = failed.iter().map(|s| (*s).to_owned()).collect();
        push_mirrors(
            dest,
            &MirrorPlan {
                repos_dir,
                description_prefix: "GitHub mirror of o/",
                public_repos: &public,
                failed_repos: &failed,
            },
        )
        .await
    }

    /// A repository whose backup failed this run is not pushed: its clone may
    /// be stale or partial, and pushing it could rewind the mirror.
    #[tokio::test]
    async fn repositories_whose_backup_failed_are_not_pushed() {
        let dir = tempdir().unwrap();
        let repos = dir.path().join("repos");
        fs::create_dir(&repos).unwrap();
        source_repo(&repos, "good");
        source_repo(&repos, "broken");
        let dest = dest(&dir.path().join("dest"), false);
        let stats = run(&dest, &repos, &["broken"]).await;
        assert_eq!(stats.pushed, 1, "{stats:?}");
        assert_eq!(stats.errored, 0, "{stats:?}");
        assert_eq!(*dest.created.lock().unwrap(), ["good"]);
        assert!(
            stats.skipped[0].starts_with("broken: its backup failed"),
            "{stats:?}"
        );
    }

    /// A clone without branches or tags is never pushed (with `--prune` it
    /// would delete everything at the destination).
    #[tokio::test]
    async fn a_repository_without_refs_never_empties_the_mirror() {
        let dir = tempdir().unwrap();
        let repos = dir.path().join("repos");
        fs::create_dir(&repos).unwrap();
        let src = source_repo(&repos, "r");
        let dest = dest(&dir.path().join("dest"), true);
        assert_eq!(run(&dest, &repos, &[]).await.pushed, 1);
        let remote = dir.path().join("dest/r.git");
        assert!(refs(&remote).contains("refs/heads/main"));

        // Every local ref disappears (a broken or emptied clone).
        for r in refs(&src).lines() {
            git(&src, &["update-ref", "-d", r]);
        }
        let stats = run(&dest, &repos, &[]).await;
        assert_eq!((stats.pushed, stats.errored), (0, 0), "{stats:?}");
        assert!(
            stats.skipped[0].contains("no branches or tags"),
            "{stats:?}"
        );
        assert!(
            refs(&remote).contains("refs/heads/main"),
            "the mirror is intact"
        );
    }

    /// Shallow clones are skipped (with one warning), not failed every run.
    #[tokio::test]
    async fn shallow_clones_are_skipped_not_failed() {
        let dir = tempdir().unwrap();
        let repos = dir.path().join("repos");
        fs::create_dir(&repos).unwrap();
        let src = source_repo(&repos, "deep");
        // A real shallow mirror clone of it.
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
                repos.join("shallow.git").to_str().unwrap(),
            ],
        );
        assert!(is_shallow(&repos.join("shallow.git")));
        let dest = dest(&dir.path().join("dest"), false);
        let stats = run(&dest, &repos, &[]).await;
        assert_eq!((stats.pushed, stats.errored), (1, 0), "{stats:?}");
        assert!(
            stats.skipped[0].starts_with("shallow: shallow clones"),
            "{stats:?}"
        );
    }
}
