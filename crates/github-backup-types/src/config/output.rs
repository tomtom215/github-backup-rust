// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Output directory layout configuration.

use std::path::PathBuf;

/// Root output path and per-owner subdirectory layout.
///
/// All paths returned by this struct follow the convention:
/// ```text
/// <root>/
///   <owner>/
///     git/
///       repos/       ← bare git mirrors
///       wikis/       ← wiki git clones
///       gists/       ← gist git clones
///       starred/     ← starred-repo git clones
///     json/
///       repos/
///         <repo>/    ← per-repo JSON metadata
///       gists/       ← gist metadata
///       *.json       ← owner-level data (starred, watched, …)
/// ```
#[derive(Debug, Clone)]
pub struct OutputConfig {
    /// Root backup directory supplied by the user.
    pub root: PathBuf,
}

impl OutputConfig {
    /// Creates an [`OutputConfig`] rooted at `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { root: path.into() }
    }

    /// Returns the directory for bare git clones: `<root>/<owner>/git/repos/`.
    #[must_use]
    pub fn repos_dir(&self, owner: &str) -> PathBuf {
        self.root.join(owner).join("git").join("repos")
    }

    /// Returns the directory for wiki git clones: `<root>/<owner>/git/wikis/`.
    #[must_use]
    pub fn wikis_dir(&self, owner: &str) -> PathBuf {
        self.root.join(owner).join("git").join("wikis")
    }

    /// Returns the directory for gist git clones: `<root>/<owner>/git/gists/`.
    #[must_use]
    pub fn gists_git_dir(&self, owner: &str) -> PathBuf {
        self.root.join(owner).join("git").join("gists")
    }

    /// Returns the JSON metadata directory for a repository:
    /// `<root>/<owner>/json/repos/<repo>/`.
    #[must_use]
    pub fn repo_meta_dir(&self, owner: &str, repo: &str) -> PathBuf {
        self.root.join(owner).join("json").join("repos").join(repo)
    }

    /// Returns the JSON metadata directory for gists:
    /// `<root>/<owner>/json/gists/`.
    #[must_use]
    pub fn gists_meta_dir(&self, owner: &str) -> PathBuf {
        self.root.join(owner).join("json").join("gists")
    }

    /// Returns the top-level JSON directory for an owner:
    /// `<root>/<owner>/json/`.
    #[must_use]
    pub fn owner_json_dir(&self, owner: &str) -> PathBuf {
        self.root.join(owner).join("json")
    }

    /// Returns the root directory for starred-repository git clones:
    /// `<root>/<owner>/git/starred/`.
    ///
    /// Individual repos are cloned into subdirectories:
    /// `<root>/<owner>/git/starred/<upstream-owner>/<repo>.git`.
    #[must_use]
    pub fn starred_repos_dir(&self, owner: &str) -> PathBuf {
        self.root.join(owner).join("git").join("starred")
    }

    /// Returns the path to the starred-repos clone queue file:
    /// `<root>/<owner>/json/starred_clone_queue.json`.
    #[must_use]
    pub fn starred_queue_path(&self, owner: &str) -> PathBuf {
        self.root
            .join(owner)
            .join("json")
            .join("starred_clone_queue.json")
    }

    /// Returns the path for a top-level owner JSON file (followers, starred…):
    /// `<root>/<owner>/json/<filename>`.
    #[must_use]
    pub fn owner_json(&self, owner: &str, filename: &str) -> PathBuf {
        self.root.join(owner).join("json").join(filename)
    }

    /// Returns the path to the backup history file:
    /// `<root>/<owner>/json/backup_history.json`.
    ///
    /// The history file is a rolling log of the last N backup runs,
    /// written after every successful run.
    #[must_use]
    pub fn backup_history_path(&self, owner: &str) -> PathBuf {
        self.root
            .join(owner)
            .join("json")
            .join("backup_history.json")
    }

    /// Returns the path to the backup state file:
    /// `<root>/<owner>/json/backup_state.json`.
    ///
    /// The state file records the timestamp of the last *successful* backup run
    /// so that subsequent runs can auto-populate `--since` for incremental
    /// operation without the user having to track the timestamp manually.
    #[must_use]
    pub fn backup_state_path(&self, owner: &str) -> PathBuf {
        self.root.join(owner).join("json").join("backup_state.json")
    }

    /// Returns the path to the backup checkpoint file:
    /// `<root>/<owner>/json/backup_checkpoint.json`.
    ///
    /// The checkpoint file lists every repository that has been fully backed
    /// up in the current run, enabling resumption after an interrupted backup
    /// without re-processing already-completed repositories.
    #[must_use]
    pub fn backup_checkpoint_path(&self, owner: &str) -> PathBuf {
        self.root
            .join(owner)
            .join("json")
            .join("backup_checkpoint.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn cfg() -> OutputConfig {
        OutputConfig::new("/var/backup")
    }

    #[test]
    fn new_stores_root_verbatim() {
        assert_eq!(OutputConfig::new("/foo").root, PathBuf::from("/foo"));
        assert_eq!(
            OutputConfig::new(PathBuf::from("/bar")).root,
            PathBuf::from("/bar")
        );
    }

    #[test]
    fn repos_dir_is_root_owner_git_repos() {
        assert_eq!(
            cfg().repos_dir("octocat"),
            Path::new("/var/backup/octocat/git/repos")
        );
    }

    #[test]
    fn wikis_dir_is_root_owner_git_wikis() {
        assert_eq!(
            cfg().wikis_dir("octocat"),
            Path::new("/var/backup/octocat/git/wikis")
        );
    }

    #[test]
    fn gists_git_dir_is_root_owner_git_gists() {
        assert_eq!(
            cfg().gists_git_dir("octocat"),
            Path::new("/var/backup/octocat/git/gists")
        );
    }

    #[test]
    fn repo_meta_dir_includes_repo_name() {
        assert_eq!(
            cfg().repo_meta_dir("octocat", "Hello-World"),
            Path::new("/var/backup/octocat/json/repos/Hello-World")
        );
    }

    #[test]
    fn gists_meta_dir_is_owner_json_gists() {
        assert_eq!(
            cfg().gists_meta_dir("octocat"),
            Path::new("/var/backup/octocat/json/gists")
        );
    }

    #[test]
    fn owner_json_dir_is_root_owner_json() {
        assert_eq!(
            cfg().owner_json_dir("octocat"),
            Path::new("/var/backup/octocat/json")
        );
    }

    #[test]
    fn starred_repos_dir_is_root_owner_git_starred() {
        assert_eq!(
            cfg().starred_repos_dir("octocat"),
            Path::new("/var/backup/octocat/git/starred")
        );
    }

    #[test]
    fn starred_queue_path_filename() {
        assert_eq!(
            cfg().starred_queue_path("octocat"),
            Path::new("/var/backup/octocat/json/starred_clone_queue.json")
        );
    }

    #[test]
    fn owner_json_joins_filename() {
        assert_eq!(
            cfg().owner_json("octocat", "starred.json"),
            Path::new("/var/backup/octocat/json/starred.json")
        );
        assert_eq!(
            cfg().owner_json("octocat", "followers.json"),
            Path::new("/var/backup/octocat/json/followers.json")
        );
    }

    #[test]
    fn backup_history_path_filename() {
        assert_eq!(
            cfg().backup_history_path("octocat"),
            Path::new("/var/backup/octocat/json/backup_history.json")
        );
    }

    #[test]
    fn backup_state_path_filename() {
        assert_eq!(
            cfg().backup_state_path("octocat"),
            Path::new("/var/backup/octocat/json/backup_state.json")
        );
    }

    #[test]
    fn backup_checkpoint_path_filename() {
        assert_eq!(
            cfg().backup_checkpoint_path("octocat"),
            Path::new("/var/backup/octocat/json/backup_checkpoint.json")
        );
    }

    // ── Cross-method invariants ─────────────────────────────────────────
    //
    // These tests pin down the path-segment structure so a mutant that
    // swaps `git` for `json` (or repos for wikis) is observable, even
    // when individual methods would otherwise pass via constant-folding.

    #[test]
    fn git_dirs_are_distinct_from_json_dirs() {
        let c = cfg();
        assert_ne!(c.repos_dir("o"), c.repo_meta_dir("o", "repos"));
        assert_ne!(c.wikis_dir("o"), c.owner_json_dir("o"));
        assert_ne!(c.gists_git_dir("o"), c.gists_meta_dir("o"));
        assert_ne!(c.starred_repos_dir("o"), c.starred_queue_path("o"));
    }

    #[test]
    fn distinct_owners_get_distinct_paths() {
        let c = cfg();
        assert_ne!(c.repos_dir("alice"), c.repos_dir("bob"));
        assert_ne!(c.owner_json_dir("alice"), c.owner_json_dir("bob"));
    }

    #[test]
    fn paths_are_anchored_at_configured_root() {
        let c = OutputConfig::new("/srv/data");
        assert!(c.repos_dir("u").starts_with("/srv/data"));
        assert!(c.owner_json_dir("u").starts_with("/srv/data"));
        assert!(c.backup_state_path("u").starts_with("/srv/data"));
    }
}
