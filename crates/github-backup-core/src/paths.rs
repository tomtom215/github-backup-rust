// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Turning names that come from the API into safe path components.
//!
//! Repository names, release tags, asset names and gist ids arrive from a remote
//! service and end up as parts of file paths under the output directory.
//! github.com sanitises most of them, but GitHub Enterprise Server and any
//! API-compatible proxy are less strict, and a backup tool must never let a
//! hostile or buggy response write outside its output tree.

use std::path::PathBuf;

/// `true` if `name` can be used verbatim as one path component: non-empty, not
/// `.` or `..`, and free of path separators and NUL.
#[must_use]
pub(crate) fn is_safe_component(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

/// Reduces `name` to a single safe file name: any directory part is dropped, and
/// a name that is empty or only dots becomes `_`.
///
/// `../../etc/passwd` becomes `passwd`; `..` becomes `_`.
#[must_use]
pub(crate) fn file_name(name: &str) -> String {
    let last = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .replace('\0', "_");
    if is_safe_component(&last) {
        last
    } else {
        "_".to_string()
    }
}

/// Builds a relative path from a name that may legitimately contain `/` (a git
/// tag such as `release/v1`), replacing every segment that is empty, `.` or `..`
/// with `_` so the result can never leave the directory it is joined to.
#[must_use]
pub(crate) fn nested(name: &str) -> PathBuf {
    let mut path = PathBuf::new();
    for segment in name.split(['/', '\\']) {
        let segment = segment.replace('\0', "_");
        path.push(if is_safe_component(&segment) {
            segment
        } else {
            "_".to_string()
        });
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn components_are_checked() {
        for ok in ["repo", "my-repo.git", "a.b", ".hidden", "ünï"] {
            assert!(is_safe_component(ok), "{ok}");
        }
        for bad in ["", ".", "..", "a/b", "a\\b", "../x", "x\0y"] {
            assert!(!is_safe_component(bad), "{bad:?}");
        }
    }

    #[test]
    fn file_name_drops_directories_and_neutralises_dots() {
        assert_eq!(file_name("app-1.0.tar.gz"), "app-1.0.tar.gz");
        assert_eq!(file_name("../../../../outside/pwned.txt"), "pwned.txt");
        assert_eq!(file_name("/etc/passwd"), "passwd");
        assert_eq!(file_name("..\\..\\win.ini"), "win.ini");
        assert_eq!(file_name(".."), "_");
        assert_eq!(file_name(""), "_");
        assert_eq!(file_name("dir/"), "_");
    }

    #[test]
    fn nested_keeps_legitimate_slashes_but_cannot_escape() {
        assert_eq!(nested("v1.2.3"), Path::new("v1.2.3"));
        assert_eq!(nested("release/v1"), Path::new("release").join("v1"));
        let escaped = nested("../../tagdir");
        assert_eq!(escaped, Path::new("_").join("_").join("tagdir"));
        assert!(!escaped
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir)));
        assert_eq!(nested("/abs/path"), Path::new("_").join("abs").join("path"));
    }
}
