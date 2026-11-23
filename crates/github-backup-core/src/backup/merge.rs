// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Helpers for incremental runs.
//!
//! # Why the lists are never fetched incrementally
//!
//! `issues.json` and `pulls.json` are single files holding every issue or pull
//! request of a repository.  GitHub's `since` filter returns *only the items
//! that changed*, so writing that response over the file would replace the
//! whole backup with the delta — silently, on every run after the first.
//!
//! Instead the engine always fetches the **complete** listing (one request per
//! 100 items), merges it into what an earlier run stored (so an item GitHub
//! no longer returns, for example a deleted issue, stays in the backup), and
//! uses the incremental watermark only for the genuinely expensive part: the
//! per-item comment / event / commit / review requests, which are skipped for
//! items that have not changed since the last complete run.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::DateTime;
use serde::Serialize;
use serde_json::Value;
use tracing::warn;

use crate::{error::CoreError, storage::Storage};

/// Merges `fresh` (the current API listing) into the JSON array stored at
/// `path`, matching items on their `key` field (`number` for issues and pull
/// requests, `id` for most other lists).
///
/// * an item present in both is replaced by the fresh copy;
/// * an item only in the stored file is **kept** — the backup never forgets
///   something it has captured, even after it is deleted on GitHub;
/// * an item only in the fresh listing is added;
/// * an item without the key cannot be matched: a stored one is kept, and a
///   fresh one is added unless an identical item is already stored, so it is
///   never duplicated run after run;
/// * items with the key come first, ordered by `key` ascending when every
///   such key is a non-negative integer (otherwise stored order, then new
///   items); items without the key follow, stored ones first.  Output is
///   therefore deterministic.
///
/// Matching goes through a key-to-position map, so a merge costs time linear
/// in the two lists plus the final sort, not quadratic.
///
/// A missing stored file is simply "nothing to merge".  A stored file that is
/// unreadable or not a JSON array is reported with a warning and treated the
/// same way: it was already unusable, and refusing to write would make a
/// damaged file permanent.
///
/// # Errors
///
/// Fails if `fresh` does not serialise to a JSON array or the stored file
/// cannot be read for a reason other than not existing.
pub(crate) fn merge_list<S, T>(
    storage: &S,
    path: &Path,
    fresh: &T,
    key: &str,
) -> Result<Vec<Value>, CoreError>
where
    S: Storage,
    T: Serialize + ?Sized,
{
    let fresh = match serde_json::to_value(fresh)? {
        Value::Array(items) => items,
        other => {
            return Err(CoreError::Json(serde::de::Error::custom(format!(
                "expected a JSON array to merge into {}, got {}",
                path.display(),
                type_name(&other)
            ))))
        }
    };

    let stored: Vec<Value> = match storage.read(path)? {
        None => Vec::new(),
        Some(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Array(items)) => items,
            Ok(other) => {
                warn!(
                    path = %path.display(),
                    found = type_name(&other),
                    "existing backup file is not a JSON array; replacing it"
                );
                Vec::new()
            }
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "existing backup file is not valid JSON; replacing it"
                );
                Vec::new()
            }
        },
    };

    let mut keyed: Vec<Value> = Vec::with_capacity(stored.len() + fresh.len());
    let mut keyless: Vec<Value> = Vec::new();
    // Key, as its JSON text (keys are scalars), to its position in `keyed`.
    let mut index: HashMap<String, usize> = HashMap::with_capacity(keyed.capacity());
    for item in stored {
        match item.get(key).map(Value::to_string) {
            Some(k) => {
                // A key that a damaged file holds twice: the first is matched.
                index.entry(k).or_insert(keyed.len());
                keyed.push(item);
            }
            None => keyless.push(item),
        }
    }
    let stored_keyless: HashSet<String> = keyless.iter().map(Value::to_string).collect();

    for item in fresh {
        match item.get(key).map(Value::to_string) {
            Some(k) => match index.get(&k) {
                Some(&i) => keyed[i] = item,
                None => {
                    index.insert(k, keyed.len());
                    keyed.push(item);
                }
            },
            None => {
                if !stored_keyless.contains(&item.to_string()) {
                    keyless.push(item);
                }
            }
        }
    }

    if keyed.iter().all(|m| m.get(key).is_some_and(Value::is_u64)) {
        // Stable, so equal keys keep their order.
        keyed.sort_by_key(|m| m.get(key).and_then(Value::as_u64));
    }
    keyed.append(&mut keyless);
    Ok(keyed)
}

/// [`merge_list`] followed by writing the result back to `path`.
///
/// # Errors
///
/// See [`merge_list`]; also fails if the file cannot be written.
pub(crate) fn merge_and_write<S, T>(
    storage: &S,
    path: &Path,
    fresh: &T,
    key: &str,
) -> Result<(), CoreError>
where
    S: Storage,
    T: Serialize + ?Sized,
{
    let merged = merge_list(storage, path, fresh, key)?;
    storage.write_json(path, &merged)
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Returns `true` if an item last updated at `updated_at` has not changed
/// since the watermark `since`, i.e. its sub-resources captured by the last
/// complete run are still current.
///
/// `None` (no watermark, so a full fetch) and any timestamp that cannot be
/// parsed both answer `false`: when in doubt, fetch.
pub(crate) fn unchanged_since(updated_at: &str, since: Option<&str>) -> bool {
    let Some(since) = since else {
        return false;
    };
    match (
        DateTime::parse_from_rfc3339(updated_at),
        DateTime::parse_from_rfc3339(since),
    ) {
        (Ok(updated), Ok(since)) => updated < since,
        _ => false,
    }
}

/// Returns `true` if the per-item files an earlier run stored for an item can
/// be kept instead of fetched again: the item has not changed since the
/// watermark ([`unchanged_since`]) **and** every file that would be written for
/// it still exists.
///
/// The check makes the shortcut safe against a backup directory that was
/// partially restored, moved or cleaned by hand, and against a category (say
/// issue events) that was switched on after the file set was last written.  A
/// file that exists but is empty or not valid JSON (a crash, a full disk, a
/// hand edit) counts as missing, so it is fetched and written again.
pub(crate) fn reusable<S: Storage>(
    storage: &S,
    updated_at: &str,
    since: Option<&str>,
    files: &[&Path],
) -> bool {
    unchanged_since(updated_at, since) && files.iter().all(|f| holds_json(storage, f))
}

/// `true` if `path` exists and holds a valid JSON document.
fn holds_json<S: Storage>(storage: &S, path: &Path) -> bool {
    matches!(
        storage.read(path),
        Ok(Some(bytes)) if serde_json::from_slice::<serde::de::IgnoredAny>(&bytes).is_ok()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::test_support::MemStorage;
    use serde_json::json;
    use std::path::PathBuf;

    fn path() -> PathBuf {
        PathBuf::from("/meta/issues.json")
    }

    fn seed(storage: &MemStorage, value: &Value) {
        storage
            .write_bytes(&path(), value.to_string().as_bytes())
            .expect("seed");
    }

    #[test]
    fn missing_file_yields_the_fresh_listing() {
        let storage = MemStorage::default();
        let merged = merge_list(
            &storage,
            &path(),
            &json!([{"number": 2}, {"number": 1}]),
            "number",
        )
        .expect("merge");
        assert_eq!(merged, vec![json!({"number": 1}), json!({"number": 2})]);
    }

    /// The regression behind the data-loss bug: a delta must not shrink the file.
    #[test]
    fn a_delta_keeps_every_earlier_item() {
        let storage = MemStorage::default();
        seed(
            &storage,
            &json!([
                {"number": 1, "title": "old one"},
                {"number": 2, "title": "old two"},
                {"number": 3, "title": "old three"}
            ]),
        );
        let merged = merge_list(
            &storage,
            &path(),
            &json!([{"number": 2, "title": "edited"}]),
            "number",
        )
        .expect("merge");
        assert_eq!(
            merged,
            vec![
                json!({"number": 1, "title": "old one"}),
                json!({"number": 2, "title": "edited"}),
                json!({"number": 3, "title": "old three"}),
            ]
        );
    }

    #[test]
    fn an_empty_fresh_listing_never_empties_the_file() {
        let storage = MemStorage::default();
        seed(&storage, &json!([{"number": 1}, {"number": 2}]));
        let merged = merge_list(&storage, &path(), &json!([]), "number").expect("merge");
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn items_github_no_longer_returns_are_kept() {
        let storage = MemStorage::default();
        seed(
            &storage,
            &json!([{"number": 5, "title": "deleted upstream"}]),
        );
        let merged =
            merge_list(&storage, &path(), &json!([{"number": 6}]), "number").expect("merge");
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0]["title"], "deleted upstream");
    }

    #[test]
    fn new_items_are_added_and_order_is_deterministic() {
        let storage = MemStorage::default();
        seed(&storage, &json!([{"number": 9}, {"number": 3}]));
        let merged = merge_list(
            &storage,
            &path(),
            &json!([{"number": 7}, {"number": 1}]),
            "number",
        )
        .expect("merge");
        let numbers: Vec<u64> = merged
            .iter()
            .map(|m| m["number"].as_u64().unwrap())
            .collect();
        assert_eq!(numbers, vec![1, 3, 7, 9]);
    }

    #[test]
    fn non_integer_keys_keep_stored_order_then_new() {
        let storage = MemStorage::default();
        seed(&storage, &json!([{"id": "b"}, {"id": "a"}]));
        let merged = merge_list(
            &storage,
            &path(),
            &json!([{"id": "c"}, {"id": "a", "x": 1}]),
            "id",
        )
        .expect("merge");
        let ids: Vec<&str> = merged.iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["b", "a", "c"]);
        assert_eq!(merged[1]["x"], 1);
    }

    #[test]
    fn corrupt_existing_file_is_replaced_not_fatal() {
        let storage = MemStorage::default();
        storage.write_bytes(&path(), b"{ truncated").expect("seed");
        let merged =
            merge_list(&storage, &path(), &json!([{"number": 1}]), "number").expect("merge");
        assert_eq!(merged, vec![json!({"number": 1})]);
    }

    #[test]
    fn existing_file_that_is_not_an_array_is_replaced() {
        let storage = MemStorage::default();
        seed(&storage, &json!({"not": "a list"}));
        let merged =
            merge_list(&storage, &path(), &json!([{"number": 1}]), "number").expect("merge");
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn a_non_array_fresh_value_is_an_error() {
        let storage = MemStorage::default();
        assert!(merge_list(&storage, &path(), &json!({"x": 1}), "number").is_err());
    }

    #[test]
    fn items_without_the_key_are_never_dropped() {
        let storage = MemStorage::default();
        seed(&storage, &json!([{"number": 1}]));
        let merged =
            merge_list(&storage, &path(), &json!([{"title": "odd"}]), "number").expect("merge");
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn items_without_the_key_are_not_duplicated_and_do_not_stop_the_sort() {
        let storage = MemStorage::default();
        seed(
            &storage,
            &json!([{"number": 9}, {"title": "odd"}, {"number": 2}]),
        );
        let fresh = json!([{"title": "odd"}, {"number": 5}, {"title": "new odd"}]);
        // Run twice: the second run must not grow the file.
        for _ in 0..2 {
            let merged = merge_list(&storage, &path(), &fresh, "number").expect("merge");
            storage.write_json(&path(), &merged).expect("write");
        }
        let merged: Value = serde_json::from_slice(&storage.get(&path()).expect("file")).unwrap();
        assert_eq!(
            merged,
            json!([
                {"number": 2}, {"number": 5}, {"number": 9},
                {"title": "odd"}, {"title": "new odd"}
            ])
        );
    }

    /// Pins the output order: numeric keys ascending (stable for duplicates),
    /// replaced items in place, then non-numeric keys in stored-then-new order.
    #[test]
    fn output_order_is_pinned() {
        let storage = MemStorage::default();
        seed(
            &storage,
            &json!([{"id": 30, "v": "a"}, {"id": 10, "v": "b"}, {"id": 30, "v": "dup"}]),
        );
        let merged = merge_list(
            &storage,
            &path(),
            &json!([{"id": 20}, {"id": 30, "v": "new"}, {"id": 5}]),
            "id",
        )
        .expect("merge");
        assert_eq!(
            merged,
            vec![
                json!({"id": 5}),
                json!({"id": 10, "v": "b"}),
                json!({"id": 20}),
                json!({"id": 30, "v": "new"}),
                json!({"id": 30, "v": "dup"}),
            ]
        );
    }

    /// The merge used to look every fresh item up with a linear scan, which
    /// took about a minute for 40 000 items.  Generous bound: a linear merge
    /// takes well under a second even in a debug build.
    #[test]
    fn merging_large_lists_is_not_quadratic() {
        let storage = MemStorage::default();
        let n = 40_000u64;
        let stored: Vec<Value> = (0..n).map(|i| json!({"number": i, "t": "old"})).collect();
        seed(&storage, &Value::Array(stored));
        let fresh: Vec<Value> = (0..n).rev().map(|i| json!({"number": i + n / 2})).collect();
        let started = std::time::Instant::now();
        let merged = merge_list(&storage, &path(), &fresh, "number").expect("merge");
        assert_eq!(merged.len() as u64, n + n / 2);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn merge_and_write_stores_the_merge() {
        let storage = MemStorage::default();
        seed(&storage, &json!([{"id": 1}]));
        merge_and_write(&storage, &path(), &json!([{"id": 2}]), "id").expect("merge");
        let on_disk: Value = serde_json::from_slice(&storage.get(&path()).expect("file")).unwrap();
        assert_eq!(on_disk, json!([{"id": 1}, {"id": 2}]));
    }

    #[test]
    fn reusable_treats_an_empty_or_corrupt_file_as_missing() {
        let storage = MemStorage::default();
        let a = PathBuf::from("/meta/issue_comments/1.json");
        let since = Some("2026-01-02T00:00:00Z");
        let old = "2026-01-01T00:00:00Z";
        for bad in [&b""[..], b"  ", b"[{\"id\": 1}"] {
            storage.write_bytes(&a, bad).expect("seed");
            assert!(!reusable(&storage, old, since, &[a.as_path()]), "{bad:?}");
        }
        storage.write_bytes(&a, b"[]").expect("seed");
        assert!(reusable(&storage, old, since, &[a.as_path()]));
    }

    #[test]
    fn unchanged_since_compares_instants_not_strings() {
        let since = Some("2026-01-02T00:00:00Z");
        assert!(unchanged_since("2026-01-01T23:59:59Z", since));
        assert!(
            !unchanged_since("2026-01-02T00:00:00Z", since),
            "equal means changed: fetch"
        );
        assert!(!unchanged_since("2026-01-02T00:00:01Z", since));
        // Same instant expressed with an offset: 2026-01-01T20:00-04:00 == 00:00Z.
        assert!(!unchanged_since("2026-01-01T20:00:00-04:00", since));
        assert!(unchanged_since("2026-01-01T19:59:59-04:00", since));
    }

    #[test]
    fn unchanged_since_fetches_when_in_doubt() {
        assert!(
            !unchanged_since("2026-01-01T00:00:00Z", None),
            "no watermark means full fetch"
        );
        assert!(!unchanged_since("not a date", Some("2026-01-02T00:00:00Z")));
        assert!(!unchanged_since("2026-01-01T00:00:00Z", Some("garbage")));
    }

    #[test]
    fn reusable_needs_an_old_item_and_every_file_present() {
        let storage = MemStorage::default();
        let a = PathBuf::from("/meta/issue_comments/1.json");
        let b = PathBuf::from("/meta/issue_events/1.json");
        let since = Some("2026-01-02T00:00:00Z");
        let old = "2026-01-01T00:00:00Z";
        let new = "2026-01-03T00:00:00Z";

        storage.write_bytes(&a, b"[]").expect("seed a");
        assert!(reusable(&storage, old, since, &[a.as_path()]));
        assert!(
            !reusable(&storage, old, since, &[a.as_path(), b.as_path()]),
            "a missing file forces a fetch even for an unchanged item"
        );
        storage.write_bytes(&b, b"[]").expect("seed b");
        assert!(reusable(&storage, old, since, &[a.as_path(), b.as_path()]));
        assert!(
            !reusable(&storage, new, since, &[a.as_path(), b.as_path()]),
            "a changed item is always fetched"
        );
        assert!(!reusable(&storage, old, None, &[a.as_path()]));
    }
}
