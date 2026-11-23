// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Reading the backup's JSON files, which hold GitHub's responses verbatim.
//!
//! Nothing here is fatal: a file that does not exist is simply not there (the
//! category was not backed up), and a file that cannot be read or parsed is
//! noted in [`Warnings`] and treated as absent, so a page renders what exists.

use std::path::Path;
use std::sync::Mutex;

use serde_json::Value;

/// Files that could not be read, collected while rendering.
#[derive(Debug, Default)]
pub(crate) struct Warnings(Mutex<Vec<String>>);

impl Warnings {
    /// Records one problem.
    pub(crate) fn push(&self, message: String) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(message);
    }

    /// Every problem, sorted (rendering may run on several threads).
    pub(crate) fn into_sorted(self) -> Vec<String> {
        let mut all = self
            .0
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        all.sort();
        all
    }
}

/// Reads and parses `path`; `None` when it does not exist or cannot be used.
pub(crate) fn read(path: &Path, warnings: &Warnings) -> Option<Value> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            warnings.push(format!("{}: cannot be read: {e}", path.display()));
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(v) => Some(v),
        Err(e) => {
            warnings.push(format!("{}: not valid JSON: {e}", path.display()));
            None
        }
    }
}

/// Reads a file that holds a JSON array; its objects (anything else in the
/// array is skipped).  Empty when the file is missing or is not an array.
pub(crate) fn read_list(path: &Path, warnings: &Warnings) -> Vec<Value> {
    match read(path, warnings) {
        None => Vec::new(),
        Some(Value::Array(items)) => items.into_iter().filter(Value::is_object).collect(),
        Some(_) => {
            warnings.push(format!("{}: expected a JSON array", path.display()));
            Vec::new()
        }
    }
}

/// The string at `pointer` (`/user/login`), if it is one.
pub(crate) fn s<'a>(v: &'a Value, pointer: &str) -> Option<&'a str> {
    v.pointer(pointer).and_then(Value::as_str)
}

/// The string at `pointer`, or `""`.
pub(crate) fn st<'a>(v: &'a Value, pointer: &str) -> &'a str {
    s(v, pointer).unwrap_or_default()
}

/// The unsigned number at `pointer`.
pub(crate) fn u(v: &Value, pointer: &str) -> Option<u64> {
    v.pointer(pointer).and_then(Value::as_u64)
}

/// `true` when `pointer` holds `true`.
pub(crate) fn flag(v: &Value, pointer: &str) -> bool {
    v.pointer(pointer).and_then(Value::as_bool).unwrap_or(false)
}

/// Who wrote an item: `user.login` (REST), `author.login` (GraphQL) or
/// `actor.login` (events); `ghost` (GitHub's name for a deleted account).
pub(crate) fn login(v: &Value) -> &str {
    s(v, "/user/login")
        .or_else(|| s(v, "/author/login"))
        .or_else(|| s(v, "/actor/login"))
        .unwrap_or("ghost")
}

/// A short, human-readable text for any JSON value (a project field value:
/// a string, a number, `{"name": ...}`, a list of users, ...).
pub(crate) fn text_of(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(items) => items
            .iter()
            .map(text_of)
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Value::Object(map) => ["name", "title", "login", "text", "date", "number", "value"]
            .iter()
            .find_map(|k| map.get(*k).map(text_of).filter(|t| !t.is_empty()))
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_files_are_silent_and_broken_ones_are_noted() {
        let dir = tempfile::tempdir().unwrap();
        let w = Warnings::default();
        assert!(read(&dir.path().join("absent.json"), &w).is_none());
        std::fs::write(dir.path().join("bad.json"), b"{not json").unwrap();
        assert!(read(&dir.path().join("bad.json"), &w).is_none());
        std::fs::write(dir.path().join("obj.json"), b"{}").unwrap();
        assert!(read_list(&dir.path().join("obj.json"), &w).is_empty());
        std::fs::write(dir.path().join("list.json"), b"[{\"a\":1}, null, 3]").unwrap();
        assert_eq!(read_list(&dir.path().join("list.json"), &w).len(), 1);
        let warnings = w.into_sorted();
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("bad.json"));
        assert!(warnings[1].contains("obj.json"));
    }

    #[test]
    fn values_become_text() {
        assert_eq!(text_of(&json!({"name": "Done", "id": 1})), "Done");
        assert_eq!(text_of(&json!([{"login": "a"}, {"login": "b"}])), "a, b");
        assert_eq!(text_of(&json!(3)), "3");
        assert_eq!(login(&json!({"author": {"login": "x"}})), "x");
        assert_eq!(login(&json!({"user": null})), "ghost");
    }
}
