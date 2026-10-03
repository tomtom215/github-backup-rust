// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub Classic Projects (v1) backup.
//!
//! # Currently not functional against github.com
//!
//! GitHub sunset Classic Projects; the REST routes this module calls
//! (`/repos/{owner}/{repo}/projects`, `/projects/{id}/columns`) are not part
//! of the published API description of github.com, GHEC or GHES 3.17-3.19 and
//! answer 404/410, so **nothing is backed up** there.  That is reported with
//! one warning per run instead of passing silently; no project data is
//! written.  (Projects v2 have different routes and are not implemented.)
//! `projects.json` and per-project column files are only written if a server
//! still implements the classic routes.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use tracing::{debug, info, warn};

use github_backup_client::BackupClient;
use github_backup_types::config::BackupOptions;

use crate::{error::CoreError, storage::Storage};

/// Set once the "not supported" warning has been logged in this process.
static WARNED: AtomicBool = AtomicBool::new(false);

/// Reports that `repo` has no classic-projects endpoint: one `warn!` per
/// process, a `debug!` for every further repository.  Returns `true` for the
/// call that warned.
fn report_unsupported(warned: &AtomicBool, repo: &str) -> bool {
    if warned.swap(true, Ordering::Relaxed) {
        debug!(repo, "classic projects not backed up (no REST endpoint)");
        false
    } else {
        warn!(
            repo,
            "--projects is not supported by the GitHub REST API: Classic Projects \
             were sunset and Projects v2 are not implemented, so no projects are \
             backed up (this warning is shown once per run)"
        );
        true
    }
}

/// Backs up Classic Projects for a single repository.
///
/// When `opts.projects` is enabled:
/// - Writes `meta_dir/projects.json` with all project objects.
/// - For each project, writes `meta_dir/project_columns_<id>.json` with the
///   columns and their cards.
///
/// Returns the number of projects backed up.
///
/// # Errors
///
/// Propagates [`CoreError`] from storage writes or non-403/404 API errors.
pub async fn backup_projects(
    client: &impl BackupClient,
    owner: &str,
    repo_name: &str,
    opts: &BackupOptions,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<u64, CoreError> {
    if !opts.projects {
        return Ok(0);
    }

    let projects = match client.list_repo_projects(owner, repo_name).await {
        Ok(p) => p,
        Err(github_backup_client::ClientError::ApiError {
            status: 404 | 410, ..
        }) => {
            report_unsupported(&WARNED, &format!("{owner}/{repo_name}"));
            return Ok(0);
        }
        Err(github_backup_client::ClientError::ApiError { status: 403, .. }) => {
            info!(
                repo = format!("{owner}/{repo_name}"),
                "skipping classic projects (insufficient permissions)"
            );
            return Ok(0);
        }
        Err(e) => return Err(e.into()),
    };

    let count = projects.len() as u64;
    info!(
        owner,
        repo = repo_name,
        count,
        "backing up classic projects"
    );
    storage.write_json(&meta_dir.join("projects.json"), &projects)?;

    // Back up columns for each project.
    for project in &projects {
        let columns = match client.list_project_columns(project.id).await {
            Ok(c) => c,
            Err(github_backup_client::ClientError::ApiError {
                status: 403 | 404 | 410,
                ..
            }) => {
                info!(
                    project_id = project.id,
                    "skipping project columns (not available)"
                );
                continue;
            }
            Err(e) => return Err(e.into()),
        };

        let filename = format!("project_columns_{}.json", project.id);
        storage.write_json(&meta_dir.join(&filename), &columns)?;
        info!(
            owner,
            repo = repo_name,
            project_id = project.id,
            column_count = columns.len(),
            "saved project columns"
        );
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use github_backup_types::config::BackupOptions;
    use github_backup_types::project::{ClassicProject, ProjectColumn};
    use github_backup_types::user::User;
    use std::path::PathBuf;

    fn make_user() -> User {
        User {
            login: "octocat".to_string(),
            id: 1,
            avatar_url: "https://github.com/images/error/octocat_happy.gif".to_string(),
            html_url: "https://github.com/octocat".to_string(),
            user_type: "User".to_string(),
        }
    }

    fn make_project(id: u64, name: &str) -> ClassicProject {
        ClassicProject {
            id,
            number: id,
            name: name.to_string(),
            body: Some("Project body".to_string()),
            state: "open".to_string(),
            creator: make_user(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            html_url: format!("https://github.com/octocat/repo/projects/{id}"),
            open_issues_count: Some(0),
        }
    }

    fn make_column(id: u64, name: &str) -> ProjectColumn {
        ProjectColumn {
            id,
            name: name.to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            cards: vec![],
        }
    }

    #[test]
    fn unsupported_warning_is_given_once_per_process() {
        let flag = AtomicBool::new(false);

        assert!(report_unsupported(&flag, "o/a"), "first repository warns");
        assert!(!report_unsupported(&flag, "o/b"), "later ones stay quiet");
    }

    #[tokio::test]
    async fn backup_projects_disabled_writes_nothing() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        let opts = BackupOptions::default(); // projects: false

        let count = backup_projects(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_projects");

        assert_eq!(count, 0);
        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn backup_projects_enabled_writes_json() {
        let p = make_project(1, "Roadmap");
        let client = MockBackupClient::new()
            .with_repo_projects(vec![p])
            .with_project_columns(vec![make_column(10, "To Do"), make_column(11, "Done")]);
        let storage = MemStorage::default();
        let opts = BackupOptions {
            projects: true,
            ..Default::default()
        };

        let count = backup_projects(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_projects");

        assert_eq!(count, 1);
        assert!(
            storage.get(&PathBuf::from("/meta/projects.json")).is_some(),
            "projects.json should be written"
        );
        assert!(
            storage
                .get(&PathBuf::from("/meta/project_columns_1.json"))
                .is_some(),
            "project_columns_1.json should be written"
        );
    }

    #[tokio::test]
    async fn backup_projects_empty_list_writes_file() {
        let client = MockBackupClient::new(); // returns empty projects
        let storage = MemStorage::default();
        let opts = BackupOptions {
            projects: true,
            ..Default::default()
        };

        let count = backup_projects(
            &client,
            "octocat",
            "repo",
            &opts,
            &PathBuf::from("/meta"),
            &storage,
        )
        .await
        .expect("backup_projects");

        assert_eq!(count, 0);
        assert!(
            storage.get(&PathBuf::from("/meta/projects.json")).is_some(),
            "projects.json should still be written for empty list"
        );
    }
}
