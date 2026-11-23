// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub Discussions backup.
//!
//! GitHub serves Discussions through its GraphQL API only (the REST API has
//! no route for them).  The client returns the GraphQL nodes as GitHub sent
//! them, every connection followed to its last page, and this module stores
//! them the way the issue backup stores issues: a list file merged by `id`
//! and one comment file per discussion.

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;
use tracing::debug;

use github_backup_client::BackupClient;
use github_backup_types::{config::BackupOptions, Raw, Repository};

use crate::{
    backup::{
        describe_unparsed, json_str, json_u64,
        merge::{merge_and_write, unchanged_since},
        Outcome,
    },
    error::CoreError,
    storage::Storage,
};

/// Backs up the discussions of a repository with their comments and replies,
/// and its discussion categories.
///
/// Writes, in `meta_dir`:
/// - `discussion_categories.json` – the categories, merged by `id`;
/// - `discussions.json` – the discussions, merged by `id` with what earlier
///   runs stored, so a discussion deleted on GitHub stays in the backup;
/// - `discussion_comments/<number>.json` – the comments of discussion
///   `number`, oldest first, each with all its replies in `replies.nodes`;
///   merged by comment `id`, and a reply an earlier run stored that GitHub no
///   longer returns is kept after the current ones.
///
/// Nothing is fetched when `--discussions` is off, or when the repository
/// object says `"has_discussions": false`.
///
/// Discussions are listed most recently updated first.  `since` is the
/// repository's incremental watermark: with it (and an earlier
/// `discussions.json` to merge into) the listing stops at the first
/// discussion not updated since the watermark, and only the discussions
/// listed get their comments fetched again.  Without it every discussion
/// and its comments are fetched.
///
/// A comment list that cannot be fetched is recorded in
/// [`Outcome::failed`] and the other discussions are still backed up; one
/// whose discussion no longer exists (GraphQL `NOT_FOUND`) is skipped.
///
/// Returns the number of discussions fetched.
///
/// # Errors
///
/// Propagates [`CoreError`] from the category or discussion listing (a
/// GraphQL `NOT_FOUND` or `FORBIDDEN` there is read by the engine as an
/// absent or refused category) and from storage writes, and any fatal error.
pub async fn backup_discussions(
    client: &impl BackupClient,
    owner: &str,
    repo: &Raw<Repository>,
    opts: &BackupOptions,
    since: Option<&str>,
    meta_dir: &Path,
    storage: &impl Storage,
) -> Result<Outcome, CoreError> {
    let mut outcome = Outcome::default();
    if !opts.discussions {
        return Ok(outcome);
    }
    let name = repo.name.as_str();
    if repo.json().get("has_discussions").and_then(Value::as_bool) == Some(false) {
        debug!(owner, repo = name, "discussions are not enabled; skipping");
        return Ok(outcome);
    }

    let categories = client.list_discussion_categories(owner, name).await?;
    merge_and_write(
        storage,
        &meta_dir.join("discussion_categories.json"),
        &categories,
        "id",
    )?;

    let list_path = meta_dir.join("discussions.json");
    // Stopping at the watermark is only safe when the older discussions are
    // already stored.
    let since = since.filter(|_| holds_list(storage, &list_path));
    let fresh = list_changed(client, owner, name, since).await?;
    merge_and_write(storage, &list_path, &fresh, "id")?;
    outcome.count = fresh.len() as u64;

    for (index, discussion) in fresh.iter().enumerate() {
        let (Some(id), Some(number)) = (json_str(discussion, "id"), json_u64(discussion, "number"))
        else {
            outcome.fail(
                format!(
                    "discussion at position {index} ({})",
                    describe_unparsed(discussion)
                ),
                "the GraphQL node has no `id` or `number`, so its comments cannot be fetched",
            );
            continue;
        };
        match client.list_discussion_comments(id).await {
            Ok(comments) => {
                let path = meta_dir
                    .join("discussion_comments")
                    .join(format!("{number}.json"));
                write_comments(storage, &path, comments)?;
            }
            Err(e) => outcome.item_error(format!("discussion #{number} comments"), e)?,
        }
    }
    Ok(outcome)
}

/// The discussions updated since `since` (all of them without it), most
/// recently updated first, following the listing page by page.
async fn list_changed(
    client: &impl BackupClient,
    owner: &str,
    name: &str,
    since: Option<&str>,
) -> Result<Vec<Value>, CoreError> {
    let mut fresh = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let page = client
            .list_discussions_page(owner, name, after.as_deref())
            .await?;
        for node in page.nodes {
            if unchanged_since(json_str(&node, "updatedAt").unwrap_or_default(), since) {
                return Ok(fresh);
            }
            fresh.push(node);
        }
        match page.next {
            None => return Ok(fresh),
            Some(next) if after.as_deref() == Some(next.as_str()) => {
                return Err(CoreError::Json(serde::de::Error::custom(format!(
                    "the discussion listing of {owner}/{name} returned the cursor {next} twice"
                ))));
            }
            Some(next) => after = Some(next),
        }
    }
}

/// `true` if `path` holds a JSON array.
fn holds_list(storage: &impl Storage, path: &Path) -> bool {
    matches!(
        storage.read(path),
        Ok(Some(bytes)) if serde_json::from_slice::<Value>(&bytes).is_ok_and(|v| v.is_array())
    )
}

/// Merges `fresh` comments into the file at `path` by `id`, first adding to
/// each comment the replies the stored copy has and the fresh one lacks.
fn write_comments(
    storage: &impl Storage,
    path: &Path,
    mut fresh: Vec<Value>,
) -> Result<(), CoreError> {
    let stored: Vec<Value> = storage
        .read(path)?
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let mut stored_replies: HashMap<String, Vec<Value>> = stored
        .into_iter()
        .filter_map(|mut comment| {
            let id = json_str(&comment, "id")?.to_string();
            match comment.pointer_mut("/replies/nodes").map(Value::take) {
                Some(Value::Array(replies)) => Some((id, replies)),
                _ => None,
            }
        })
        .collect();
    for comment in &mut fresh {
        let Some(old) = json_str(comment, "id").and_then(|id| stored_replies.remove(id)) else {
            continue;
        };
        let Some(Value::Array(replies)) = comment.pointer_mut("/replies/nodes") else {
            continue;
        };
        for reply in old {
            let id = reply.get("id");
            if id.is_some() && !replies.iter().any(|r| r.get("id") == id) {
                replies.push(reply);
            }
        }
    }
    merge_and_write(storage, path, &fresh, "id")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{is_forbidden, is_gone, mock_client::MockBackupClient};
    use crate::storage::test_support::MemStorage;
    use serde_json::json;
    use std::path::PathBuf;

    fn opts() -> BackupOptions {
        BackupOptions {
            discussions: true,
            ..Default::default()
        }
    }

    fn repo(has_discussions: Option<bool>) -> Raw<Repository> {
        let mut value = json!({
            "id": 1, "name": "r", "full_name": "o/r",
            "owner": {"id": 1, "login": "o", "type": "User", "avatar_url": "", "html_url": ""},
            "private": false, "fork": false, "has_issues": true, "has_wiki": false,
            "clone_url": "", "ssh_url": "", "html_url": ""
        });
        if let Some(on) = has_discussions {
            value["has_discussions"] = json!(on);
        }
        Raw::from_value(value).expect("repository")
    }

    fn discussion(number: u64, updated_at: &str) -> Value {
        json!({
            "id": format!("D_{number}"), "number": number, "title": format!("#{number}"),
            "updatedAt": updated_at,
            "category": {"id": "C_q", "name": "Q&A"},
            "labels": {"totalCount": 0, "pageInfo": {"hasNextPage": false, "endCursor": null}, "nodes": []},
            "reactionGroups": [{"content": "THUMBS_UP", "reactors": {"totalCount": 2}}],
            "answer": null, "locked": false, "closed": false,
            "comments": {"totalCount": 1}
        })
    }

    fn meta() -> PathBuf {
        PathBuf::from("/meta")
    }

    fn read(storage: &MemStorage, path: &str) -> Option<Value> {
        storage
            .get(&meta().join(path))
            .map(|b| serde_json::from_slice(&b).expect("json"))
    }

    async fn run(
        client: &MockBackupClient,
        repo: &Raw<Repository>,
        since: Option<&str>,
        storage: &MemStorage,
    ) -> Result<Outcome, CoreError> {
        backup_discussions(client, "o", repo, &opts(), since, &meta(), storage).await
    }

    /// Every page of the listing is followed, every node is stored verbatim
    /// (properties the backup knows nothing about included), and each
    /// discussion gets its comments with their replies.
    #[tokio::test]
    async fn every_page_is_stored_losslessly_with_comments_and_categories() {
        let mut first = discussion(3, "2024-03-01T00:00:00Z");
        first["somethingNew"] = json!({"nested": [1, 2]});
        let comment = json!({
            "id": "DC_1", "body": "hi", "isAnswer": true,
            "replies": {"totalCount": 1, "pageInfo": {"hasNextPage": false, "endCursor": "x"},
                        "nodes": [{"id": "DC_2", "body": "reply"}]}
        });
        let client = MockBackupClient::new()
            .with_raw(
                "discussion_categories",
                vec![json!({"id": "C_q", "name": "Q&A"})],
            )
            .with_discussion_pages(vec![
                vec![first.clone(), discussion(2, "2024-02-01T00:00:00Z")],
                vec![discussion(1, "2024-01-01T00:00:00Z")],
            ])
            .with_discussion_comments("D_3", vec![comment.clone()]);
        let storage = MemStorage::default();

        let outcome = run(&client, &repo(Some(true)), None, &storage)
            .await
            .expect("ok");

        assert_eq!(
            outcome,
            Outcome {
                count: 3,
                failed: vec![]
            }
        );
        assert_eq!(
            client.calls()[..3],
            [
                "discussion_categories:",
                "discussions:",
                "discussions:page-1"
            ]
        );
        let list = read(&storage, "discussions.json").expect("list");
        assert_eq!(list.as_array().map(Vec::len), Some(3));
        assert_eq!(list[0], first, "stored exactly as GitHub sent it");
        assert_eq!(
            read(&storage, "discussion_comments/3.json"),
            Some(json!([comment]))
        );
        for n in [1, 2] {
            assert_eq!(
                read(&storage, &format!("discussion_comments/{n}.json")),
                Some(json!([]))
            );
        }
        assert_eq!(
            read(&storage, "discussion_categories.json"),
            Some(json!([{"id": "C_q", "name": "Q&A"}]))
        );
    }

    /// With a watermark the listing stops at the first discussion not
    /// updated since; the stored older discussions are kept and only the
    /// changed one gets its comments again.
    #[tokio::test]
    async fn an_incremental_run_stops_at_the_watermark_and_keeps_the_rest() {
        let storage = MemStorage::default();
        storage
            .write_json(
                &meta().join("discussions.json"),
                &json!([
                    discussion(1, "2024-01-01T00:00:00Z"),
                    discussion(2, "2024-01-02T00:00:00Z")
                ]),
            )
            .expect("seed");
        let client = MockBackupClient::new().with_discussion_pages(vec![
            vec![
                discussion(2, "2024-06-01T00:00:00Z"),
                discussion(1, "2024-01-01T00:00:00Z"),
            ],
            vec![discussion(0, "2023-01-01T00:00:00Z")],
        ]);

        let outcome = run(&client, &repo(None), Some("2024-03-01T00:00:00Z"), &storage)
            .await
            .expect("ok");

        assert_eq!(outcome.count, 1);
        let calls = client.calls();
        assert!(
            !calls.contains(&"discussions:page-1".to_string()),
            "{calls:?}"
        );
        assert!(calls.contains(&"discussion_comments:D_2".to_string()));
        assert!(!calls.contains(&"discussion_comments:D_1".to_string()));
        let list = read(&storage, "discussions.json").expect("list");
        let updated: Vec<&str> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["updatedAt"].as_str().unwrap())
            .collect();
        assert_eq!(updated, ["2024-01-01T00:00:00Z", "2024-06-01T00:00:00Z"]);
    }

    /// Without a stored list there is nothing to keep, so the watermark is
    /// ignored and every discussion is fetched (the category was switched on
    /// later, or the file was removed).
    #[tokio::test]
    async fn a_watermark_without_a_stored_list_fetches_everything() {
        let client = MockBackupClient::new().with_discussion_pages(vec![
            vec![discussion(2, "2024-06-01T00:00:00Z")],
            vec![discussion(1, "2020-01-01T00:00:00Z")],
        ]);
        let storage = MemStorage::default();
        let outcome = run(&client, &repo(None), Some("2024-03-01T00:00:00Z"), &storage)
            .await
            .expect("ok");
        assert_eq!(outcome.count, 2);
        assert!(read(&storage, "discussion_comments/1.json").is_some());
    }

    #[tokio::test]
    async fn a_repository_without_discussions_is_skipped_quietly() {
        let client = MockBackupClient::new()
            .with_discussion_pages(vec![vec![discussion(1, "2024-01-01T00:00:00Z")]]);
        let storage = MemStorage::default();
        let outcome = run(&client, &repo(Some(false)), None, &storage)
            .await
            .expect("ok");
        assert_eq!(outcome, Outcome::default());
        assert!(client.calls().is_empty());
        assert_eq!(storage.len(), 0);
    }

    #[tokio::test]
    async fn nothing_is_fetched_unless_requested() {
        let client = MockBackupClient::new();
        let storage = MemStorage::default();
        backup_discussions(
            &client,
            "o",
            &repo(Some(true)),
            &BackupOptions::default(),
            None,
            &meta(),
            &storage,
        )
        .await
        .expect("ok");
        assert!(client.calls().is_empty());
    }

    /// GraphQL `NOT_FOUND` and `FORBIDDEN` reach the engine as an absent and
    /// a refused category; any other GraphQL error is a plain failure.
    #[tokio::test]
    async fn graphql_errors_of_the_listing_are_returned_to_the_engine() {
        for (kind, gone, forbidden) in [
            ("NOT_FOUND", true, false),
            ("FORBIDDEN", false, true),
            ("INTERNAL", false, false),
        ] {
            let client = MockBackupClient::new().with_graphql_error("discussions", kind);
            let err = run(&client, &repo(Some(true)), None, &MemStorage::default())
                .await
                .expect_err(kind);
            assert_eq!(is_gone(&err), gone, "{kind}");
            assert_eq!(is_forbidden(&err), forbidden, "{kind}");
        }
    }

    /// One discussion's comments failing is recorded and the others are
    /// still backed up; a discussion deleted meanwhile is not a failure.
    #[tokio::test]
    async fn a_failing_comment_list_is_recorded_and_the_rest_continue() {
        let client = MockBackupClient::new()
            .with_discussion_pages(vec![vec![
                discussion(3, "2024-03-01T00:00:00Z"),
                discussion(2, "2024-02-01T00:00:00Z"),
                discussion(1, "2024-01-01T00:00:00Z"),
            ]])
            .with_graphql_error("discussion_comments:D_3", "INTERNAL")
            .with_graphql_error("discussion_comments:D_2", "NOT_FOUND");
        let storage = MemStorage::default();
        let outcome = run(&client, &repo(Some(true)), None, &storage)
            .await
            .expect("not fatal");
        let whats: Vec<&str> = outcome.failed.iter().map(|(w, _)| w.as_str()).collect();
        assert_eq!(whats, ["discussion #3 comments"]);
        assert!(read(&storage, "discussion_comments/1.json").is_some());
        assert!(read(&storage, "discussion_comments/3.json").is_none());
    }

    #[tokio::test]
    async fn a_node_without_id_or_number_is_recorded() {
        let client =
            MockBackupClient::new().with_discussion_pages(vec![vec![json!({"title": "odd"})]]);
        let outcome = run(&client, &repo(Some(true)), None, &MemStorage::default())
            .await
            .expect("ok");
        assert_eq!(outcome.failed.len(), 1);
    }

    /// Comments and replies GitHub no longer returns stay in the backup.
    #[tokio::test]
    async fn comments_and_replies_accumulate() {
        let storage = MemStorage::default();
        storage
            .write_json(
                &meta().join("discussion_comments/1.json"),
                &json!([
                    {"id": "DC_gone", "body": "deleted upstream"},
                    {"id": "DC_1", "replies": {"nodes": [{"id": "R_gone"}, {"id": "R_1", "body": "old"}]}}
                ]),
            )
            .expect("seed");
        let client = MockBackupClient::new()
            .with_discussion_pages(vec![vec![discussion(1, "2024-01-01T00:00:00Z")]])
            .with_discussion_comments(
                "D_1",
                vec![json!({"id": "DC_1", "replies": {"nodes": [{"id": "R_1", "body": "new"}]}})],
            );
        run(&client, &repo(Some(true)), None, &storage)
            .await
            .expect("ok");
        assert_eq!(
            read(&storage, "discussion_comments/1.json"),
            Some(json!([
                {"id": "DC_gone", "body": "deleted upstream"},
                {"id": "DC_1", "replies": {"nodes": [{"id": "R_1", "body": "new"}, {"id": "R_gone"}]}}
            ]))
        );
    }
}
