// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Persistence and management for the starred-repository clone queue.
//!
//! The queue is serialised as pretty-printed JSON and written **atomically**:
//! a sibling `.tmp` file is written first, then renamed into place.  This
//! guarantees that a crash or Ctrl+C can never leave the queue file in a
//! truncated or partially-written state.
//!
//! # Resuming
//!
//! On the next run, [`load_or_create`] reloads the file and merges any newly
//! starred repositories (by repository ID).  Items already in the queue are
//! left unchanged — so `Done` and `Failed` items are never reprocessed unless
//! the user manually edits the file and resets their state to `"pending"`.

use std::collections::HashSet;
use std::path::Path;

use chrono::Utc;

use github_backup_types::starred_queue::{CloneState, StarredCloneQueue, StarredQueueItem};
use github_backup_types::{Raw, Repository};

use crate::error::CoreError;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Schema version written to every newly created queue file.
pub const QUEUE_VERSION: u32 = 1;

/// Maximum total clone attempts before an item is marked [`CloneState::Failed`].
///
/// Includes the initial attempt plus all retries.  With `MAX_ATTEMPTS = 4`
/// and three inter-attempt delays the total worst-case wait is ~2 m 35 s.
pub const MAX_ATTEMPTS: u32 = 4;

/// Backoff delay in seconds before each successive retry attempt.
///
/// Index 0 is the delay before retry 1 (after the initial failure), index 1
/// before retry 2, and index 2 before retry 3.
pub const RETRY_DELAYS_SECS: [u64; 3] = [5, 30, 120];

// ── Public types ──────────────────────────────────────────────────────────────

/// Aggregate statistics derived from the current queue state.
#[derive(Debug, Clone, Copy, Default)]
pub struct QueueStats {
    /// Total items in the queue (all states).
    pub total: usize,
    /// Items that have been successfully cloned.
    pub done: usize,
    /// Items where all retry attempts were exhausted.
    pub failed: usize,
    /// Items not yet processed (or pending retry).
    pub pending: usize,
}

// ── Public functions ──────────────────────────────────────────────────────────

/// Loads the queue from `path` if the file exists, or creates a new empty
/// queue.
///
/// New repositories from `starred` are appended in [`CloneState::Pending`].
/// Repositories already present in the queue (matched by GitHub repo ID) are
/// left untouched — their state is preserved.  (See [`begin_pass`] for how a
/// finished queue is refreshed on the next run.)
///
/// # Errors
///
/// Returns [`CoreError`] if the file exists but cannot be read or parsed.
pub fn load_or_create(
    path: &Path,
    owner: &str,
    starred: &[Raw<Repository>],
) -> Result<StarredCloneQueue, CoreError> {
    let mut queue = if path.exists() {
        let data = std::fs::read_to_string(path)
            .map_err(|e| CoreError::io(path.display().to_string(), e))?;
        serde_json::from_str::<StarredCloneQueue>(&data)?
    } else {
        let now = Utc::now().to_rfc3339();
        StarredCloneQueue {
            version: QUEUE_VERSION,
            owner: owner.to_string(),
            created_at: now.clone(),
            updated_at: now,
            items: vec![],
        }
    };

    merge_starred(&mut queue, starred);
    Ok(queue)
}

/// Starts a new pass over the queue when the previous one is complete.
///
/// While a pass is **in progress** (some item is `Pending`) nothing changes, so
/// an interrupted run resumes where it stopped.  Once nothing is `Pending`,
/// every item goes back to `Pending` with its retry budget reset: each run
/// refreshes every starred mirror and retries earlier failures, instead of
/// cloning once and never updating.
pub fn begin_pass(queue: &mut StarredCloneQueue) {
    if queue.items.is_empty() || queue.items.iter().any(|i| i.state == CloneState::Pending) {
        return;
    }
    for item in &mut queue.items {
        item.state = CloneState::Pending;
        item.retries = 0;
        item.last_error = None;
        item.finished_at = None;
    }
}

/// Writes `queue` to `path` atomically.
///
/// The queue is serialised to a sibling `.tmp` file which is then renamed
/// over `path`.  Creates parent directories as needed.
///
/// # Errors
///
/// Returns [`CoreError`] on I/O or serialisation failure.
pub fn save(queue: &mut StarredCloneQueue, path: &Path) -> Result<(), CoreError> {
    queue.updated_at = Utc::now().to_rfc3339();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CoreError::io(parent.display().to_string(), e))?;
    }

    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(queue)?;
    std::fs::write(&tmp, &json).map_err(|e| CoreError::io(tmp.display().to_string(), e))?;
    std::fs::rename(&tmp, path).map_err(|e| CoreError::io(path.display().to_string(), e))?;
    Ok(())
}

/// Computes aggregate statistics from the current queue contents.
#[must_use]
pub fn compute_stats(queue: &StarredCloneQueue) -> QueueStats {
    let mut stats = QueueStats {
        total: queue.items.len(),
        ..Default::default()
    };
    for item in &queue.items {
        match item.state {
            CloneState::Done => stats.done += 1,
            CloneState::Failed => stats.failed += 1,
            CloneState::Pending => stats.pending += 1,
        }
    }
    stats
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Appends newly discovered starred repos to the queue.
///
/// Items already present (by numeric repo ID) are never modified.
fn merge_starred(queue: &mut StarredCloneQueue, repos: &[Raw<Repository>]) {
    let known_ids: HashSet<u64> = queue.items.iter().map(|i| i.id).collect();

    for repo in repos {
        if known_ids.contains(&repo.id) {
            continue;
        }
        queue.items.push(StarredQueueItem {
            id: repo.id,
            full_name: repo.full_name.clone(),
            clone_url: repo.clone_url.clone(),
            ssh_url: repo.ssh_url.clone(),
            size_kb: repo.size,
            state: CloneState::Pending,
            retries: 0,
            last_error: None,
            finished_at: None,
        });
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use github_backup_types::user::User;
    use tempfile::NamedTempFile;

    fn sample_user() -> User {
        User {
            id: 1,
            login: "octocat".to_string(),
            user_type: "User".to_string(),
            avatar_url: String::new(),
            html_url: String::new(),
        }
    }

    fn make_repo(id: u64, full_name: &str) -> Raw<Repository> {
        let name = full_name.split('/').nth(1).unwrap_or(full_name).to_string();
        Raw::from_typed(Repository {
            id,
            full_name: full_name.to_string(),
            name: name.clone(),
            owner: sample_user(),
            private: false,
            fork: false,
            archived: false,
            disabled: false,
            description: None,
            clone_url: format!("https://github.com/{full_name}.git"),
            ssh_url: format!("git@github.com:{full_name}.git"),
            default_branch: Some("main".to_string()),
            size: 1024,
            has_issues: true,
            has_wiki: false,
            created_at: Some("2024-01-01T00:00:00Z".to_string()),
            pushed_at: None,
            updated_at: Some("2024-01-01T00:00:00Z".to_string()),
            html_url: format!("https://github.com/{full_name}"),
        })
    }

    fn empty_queue(owner: &str) -> StarredCloneQueue {
        StarredCloneQueue {
            version: QUEUE_VERSION,
            owner: owner.to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            items: vec![],
        }
    }

    #[test]
    fn merge_adds_new_repos() {
        let repos = vec![make_repo(1, "a/one"), make_repo(2, "b/two")];
        let mut queue = empty_queue("octocat");
        merge_starred(&mut queue, &repos);
        assert_eq!(queue.items.len(), 2);
        assert!(queue.items.iter().all(|i| i.state == CloneState::Pending));
    }

    #[test]
    fn merge_preserves_existing_state() {
        let mut queue = empty_queue("octocat");
        // Pre-populate with a Done item.
        queue.items.push(StarredQueueItem {
            id: 1,
            full_name: "a/one".to_string(),
            clone_url: "https://github.com/a/one.git".to_string(),
            ssh_url: "git@github.com:a/one.git".to_string(),
            size_kb: 1024,
            state: CloneState::Done,
            retries: 0,
            last_error: None,
            finished_at: Some("2026-01-01T00:00:00Z".to_string()),
        });

        let repos = vec![make_repo(1, "a/one"), make_repo(2, "b/two")];
        merge_starred(&mut queue, &repos);

        assert_eq!(queue.items.len(), 2, "new repo should be appended");
        assert_eq!(
            queue.items[0].state,
            CloneState::Done,
            "existing Done state must not be overwritten"
        );
        assert_eq!(queue.items[1].state, CloneState::Pending);
    }

    #[test]
    fn merge_does_not_duplicate() {
        let repos = vec![make_repo(1, "a/one")];
        let mut queue = empty_queue("octocat");
        merge_starred(&mut queue, &repos);
        merge_starred(&mut queue, &repos); // second call
        assert_eq!(queue.items.len(), 1, "should not duplicate on second merge");
    }

    #[test]
    fn compute_stats_counts_all_states() {
        let mut queue = empty_queue("octocat");
        for (id, state) in [
            (1, CloneState::Done),
            (2, CloneState::Pending),
            (3, CloneState::Failed),
            (4, CloneState::Done),
        ] {
            queue.items.push(StarredQueueItem {
                id,
                full_name: format!("x/repo{id}"),
                clone_url: String::new(),
                ssh_url: String::new(),
                size_kb: 0,
                state,
                retries: 0,
                last_error: None,
                finished_at: None,
            });
        }

        let stats = compute_stats(&queue);
        assert_eq!(stats.total, 4);
        assert_eq!(stats.done, 2);
        assert_eq!(stats.pending, 1);
        assert_eq!(stats.failed, 1);
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let mut queue = empty_queue("octocat");
        queue.items.push(StarredQueueItem {
            id: 42,
            full_name: "x/y".to_string(),
            clone_url: "https://github.com/x/y.git".to_string(),
            ssh_url: "git@github.com:x/y.git".to_string(),
            size_kb: 512,
            state: CloneState::Done,
            retries: 0,
            last_error: None,
            finished_at: Some("2026-01-02T00:00:00Z".to_string()),
        });

        let tmp = NamedTempFile::new().expect("tempfile");
        save(&mut queue, tmp.path()).expect("save");

        let loaded = load_or_create(tmp.path(), "octocat", &[]).expect("load");
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].id, 42);
        assert_eq!(loaded.items[0].state, CloneState::Done);
    }

    #[test]
    fn load_or_create_creates_new_when_missing() {
        let path = std::path::PathBuf::from("/nonexistent/path/queue.json");
        // Path does not exist — should create a fresh queue and merge repos.
        let repos = vec![make_repo(99, "new/repo")];
        let queue = load_or_create(&path, "alice", &repos).expect("create");
        assert_eq!(queue.owner, "alice");
        assert_eq!(queue.items.len(), 1);
        assert_eq!(queue.items[0].id, 99);
    }

    fn qitem(id: u64, state: CloneState) -> github_backup_types::starred_queue::StarredQueueItem {
        github_backup_types::starred_queue::StarredQueueItem {
            id,
            full_name: format!("o/r{id}"),
            clone_url: String::new(),
            ssh_url: String::new(),
            size_kb: 0,
            state,
            retries: 3,
            last_error: Some("boom".into()),
            finished_at: Some("2026-01-01T00:00:00Z".into()),
        }
    }

    fn q(items: Vec<github_backup_types::starred_queue::StarredQueueItem>) -> StarredCloneQueue {
        StarredCloneQueue {
            version: QUEUE_VERSION,
            owner: "o".into(),
            created_at: String::new(),
            updated_at: String::new(),
            items,
        }
    }

    #[test]
    fn a_completed_pass_is_restarted_with_failures_retried() {
        let mut queue = q(vec![
            qitem(1, CloneState::Done),
            qitem(2, CloneState::Failed),
        ]);
        begin_pass(&mut queue);
        for item in &queue.items {
            assert_eq!(item.state, CloneState::Pending);
            assert_eq!(item.retries, 0);
            assert!(item.last_error.is_none() && item.finished_at.is_none());
        }
    }

    #[test]
    fn a_pass_in_progress_is_left_alone() {
        let mut queue = q(vec![
            qitem(1, CloneState::Done),
            qitem(2, CloneState::Pending),
        ]);
        begin_pass(&mut queue);
        assert_eq!(queue.items[0].state, CloneState::Done);
        assert_eq!(queue.items[1].state, CloneState::Pending);
    }

    #[test]
    fn an_empty_queue_stays_empty() {
        let mut queue = q(vec![]);
        begin_pass(&mut queue);
        assert!(queue.items.is_empty());
    }
}
