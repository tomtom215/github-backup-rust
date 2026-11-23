// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub Projects (v2) backup.
//!
//! Projects v2 belong to a user or an organisation, not to a repository, so
//! this is owner-level data.  Through the REST API
//! (`/users/{username}/projectsV2`, `/orgs/{org}/projectsV2`) it saves, under
//! `<owner>/json/projects/`:
//!
//! - `projects.json` – every project;
//! - `<number>/fields.json` – the project's fields (columns), built-in and
//!   custom, with their options;
//! - `<number>/items.json` – its items (issues, pull requests, draft issues)
//!   with the value of every field.
//!
//! All three accumulate: they are merged by `id` with what earlier runs
//! stored, so a deleted project, field or item stays in the backup.  Classic
//! projects (v1) were sunset by GitHub and are not backed up.

use std::path::Path;

use tracing::{info, warn};

use github_backup_client::BackupClient;
use github_backup_types::config::{BackupOptions, BackupTarget};

use crate::{
    backup::{
        all_json, describe_unparsed, is_forbidden, is_gone, json_u64, merge::merge_and_write,
        refused, Outcome,
    },
    error::CoreError,
    storage::Storage,
};

/// The most field ids one item listing can ask for (GitHub's limit).
const MAX_FIELDS: usize = 50;

/// What a classic token needs to read projects.
const NEEDS: &str = "the `read:project` scope";

/// Backs up the Projects (v2) of `owner` into `json_dir/projects/`.
///
/// Returns the number of projects.  A project whose fields or items cannot
/// be fetched is recorded in [`Outcome::failed`] and the others are still
/// backed up.  When the listing answers 404 (no projects API, e.g. an old
/// GitHub Enterprise Server) nothing is written.  A 403 refusal is handled as
/// described for [`BackupOptions::tolerate_forbidden`].
///
/// # Errors
///
/// Propagates [`CoreError`] from the project listing or storage writes, and
/// any fatal error.
pub async fn backup_projects(
    client: &impl BackupClient,
    owner: &str,
    opts: &BackupOptions,
    json_dir: &Path,
    storage: &impl Storage,
) -> Result<Outcome, CoreError> {
    let mut outcome = Outcome::default();
    if !opts.projects || opts.dry_run {
        return Ok(outcome);
    }
    let org = opts.target == BackupTarget::Org;

    let projects = match client.list_projects(owner, org).await {
        Ok(projects) => projects,
        Err(e) => {
            let e = CoreError::from(e);
            if is_gone(&e) {
                info!(owner, "no Projects (v2) API here; skipping projects");
                return Ok(outcome);
            }
            if is_forbidden(&e) {
                refused(opts, "projects", NEEDS, e, true)?;
                return Ok(outcome);
            }
            return Err(e);
        }
    };
    outcome.count = (projects.len() + projects.unparsed_count()) as u64;
    let dir = json_dir.join("projects");
    merge_and_write(storage, &dir.join("projects.json"), &projects, "id")?;

    for project in all_json(&projects) {
        let Some(number) = json_u64(project, "number") else {
            outcome.fail(
                format!("project ({})", describe_unparsed(project)),
                "the API object has no `number`, so its fields and items cannot be fetched",
            );
            continue;
        };
        let project_dir = dir.join(number.to_string());

        let fields = match client.list_project_fields(owner, org, number).await {
            Ok(fields) => fields,
            Err(e) => {
                outcome.item_error(format!("project {number} fields"), e)?;
                continue;
            }
        };
        merge_and_write(storage, &project_dir.join("fields.json"), &fields, "id")?;

        let mut field_ids: Vec<u64> = all_json(&fields)
            .filter_map(|f| json_u64(f, "id"))
            .collect();
        if field_ids.len() > MAX_FIELDS {
            warn!(
                owner,
                project = number,
                fields = field_ids.len(),
                "project has more fields than one item listing can return"
            );
            outcome.fail(
                format!("project {number} items"),
                format!(
                    "the project has {} fields and GitHub returns the values of at most \
                     {MAX_FIELDS}; the values of the others are not backed up",
                    field_ids.len()
                ),
            );
            field_ids.truncate(MAX_FIELDS);
        }
        match client
            .list_project_items(owner, org, number, &field_ids)
            .await
        {
            Ok(items) => merge_and_write(storage, &project_dir.join("items.json"), &items, "id")?,
            Err(e) => outcome.item_error(format!("project {number} items"), e)?,
        }
    }

    info!(owner, count = outcome.count, "backed up projects");
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::mock_client::MockBackupClient;
    use crate::storage::test_support::MemStorage;
    use serde_json::{json, Value};
    use std::path::PathBuf;

    fn opts() -> BackupOptions {
        BackupOptions {
            projects: true,
            ..Default::default()
        }
    }

    fn read(storage: &MemStorage, path: &str) -> Value {
        serde_json::from_slice(&storage.get(&PathBuf::from(path)).expect(path)).expect("json")
    }

    #[tokio::test]
    async fn disabled_writes_nothing() {
        let storage = MemStorage::default();
        let outcome = backup_projects(
            &MockBackupClient::new(),
            "o",
            &BackupOptions::default(),
            &PathBuf::from("/json"),
            &storage,
        )
        .await
        .expect("ok");
        assert_eq!(outcome, Outcome::default());
        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn projects_fields_and_items_with_values_are_stored_as_served() {
        let client = MockBackupClient::new()
            .with_raw(
                "projects",
                vec![json!({"id": 10, "number": 2, "title": "Roadmap"})],
            )
            .with_raw(
                "project_fields",
                vec![json!({"id": 100, "name": "Status", "data_type": "single_select"})],
            )
            .with_raw(
                "project_items",
                vec![json!({"id": 7, "content_type": "Issue",
                            "fields": [{"id": 100, "name": "Status", "value": "Done"}]})],
            );
        let storage = MemStorage::default();
        let outcome = backup_projects(&client, "o", &opts(), &PathBuf::from("/json"), &storage)
            .await
            .expect("ok");

        assert_eq!(outcome.count, 1);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert_eq!(
            read(&storage, "/json/projects/projects.json")[0]["title"],
            "Roadmap"
        );
        assert_eq!(read(&storage, "/json/projects/2/fields.json")[0]["id"], 100);
        assert_eq!(
            read(&storage, "/json/projects/2/items.json")[0]["fields"][0]["value"],
            "Done"
        );
        assert_eq!(
            client.project_item_field_ids(),
            vec![vec![100]],
            "items are asked for with every field's value"
        );
    }

    #[tokio::test]
    async fn a_refusal_fails_when_requested_and_is_skipped_under_all() {
        let client = MockBackupClient::new().with_error("projects", 403);
        let err = backup_projects(
            &client,
            "o",
            &opts(),
            &PathBuf::from("/json"),
            &MemStorage::default(),
        )
        .await
        .expect_err("explicit --projects: failure");
        assert!(err.to_string().contains("read:project"), "{err}");

        let all = BackupOptions {
            tolerate_forbidden: true,
            ..opts()
        };
        backup_projects(
            &client,
            "o",
            &all,
            &PathBuf::from("/json"),
            &MemStorage::default(),
        )
        .await
        .expect("--all: skipped");
    }

    #[tokio::test]
    async fn one_project_that_fails_does_not_stop_the_others() {
        let client = MockBackupClient::new()
            .with_raw(
                "projects",
                vec![
                    json!({"id": 1, "number": 1}),
                    json!({"id": 2, "number": 2}),
                    json!({"id": 3}),
                ],
            )
            .with_error("project_fields:1", 500);
        let storage = MemStorage::default();
        let outcome = backup_projects(&client, "o", &opts(), &PathBuf::from("/json"), &storage)
            .await
            .expect("ok");
        let whats: Vec<&str> = outcome.failed.iter().map(|(w, _)| w.as_str()).collect();
        assert_eq!(whats, ["project 1 fields", "project (id 3)"]);
        assert!(storage
            .get(&PathBuf::from("/json/projects/2/items.json"))
            .is_some());
    }
}
