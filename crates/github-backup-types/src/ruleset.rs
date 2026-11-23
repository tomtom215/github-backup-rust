// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository ruleset type.

use serde::{Deserialize, Serialize};

/// A ruleset as `GET /repos/{owner}/{repo}/rulesets` lists it: a summary
/// without the rules themselves, which the backup fetches by `id`.
///
/// The listing includes the rulesets the repository inherits from its
/// organisation (`source_type` `"Organization"`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ruleset {
    /// Unique ruleset identifier.
    pub id: u64,
    /// Ruleset name (empty when omitted).
    #[serde(default)]
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruleset_summary_parses() {
        let json = serde_json::json!({
            "id": 42, "name": "super cool ruleset", "source_type": "Repository",
            "source": "monalisa/my-repo", "enforcement": "enabled",
            "node_id": "RRS_lACkVXNlcgQB", "_links": {}
        });
        let ruleset: Ruleset = serde_json::from_value(json).expect("ruleset");
        assert_eq!(ruleset.id, 42);
        assert_eq!(ruleset.name, "super cool ruleset");
    }
}
