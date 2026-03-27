// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository label type.

use serde::{Deserialize, Serialize};

/// A label that can be applied to issues and pull requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    /// Numeric label identifier.
    pub id: u64,
    /// Label name.
    pub name: String,
    /// Hex colour string without the leading `#` (e.g. `"e11d48"`).
    pub color: String,
    /// Optional description of the label's meaning.
    pub description: Option<String>,
    /// Whether this is a default label created automatically by GitHub.
    pub default: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_deserialise_returns_correct_fields() {
        let json = r#"{
            "id": 208045946,
            "name": "bug",
            "color": "f29513",
            "description": "Something isn't working",
            "default": true
        }"#;

        let label: Label = serde_json::from_str(json).expect("deserialise");
        assert_eq!(label.name, "bug");
        assert_eq!(label.color, "f29513");
        assert!(label.default);
    }
}
