// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository security advisory type.

use serde::{Deserialize, Serialize};

/// A published repository security advisory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityAdvisory {
    /// GitHub Security Advisory identifier (e.g. `"GHSA-xxxx-xxxx-xxxx"`).
    pub ghsa_id: String,
    /// CVE identifier, or `None` if not assigned.
    pub cve_id: Option<String>,
    /// Advisory title.
    pub summary: String,
    /// Advisory description (Markdown).
    pub description: Option<String>,
    /// Severity: `"critical"`, `"high"`, `"medium"`, `"low"`.
    pub severity: String,
    /// Advisory state: `"published"`, `"withdrawn"`, `"draft"`.
    pub state: String,
    /// Vulnerable package references.
    pub vulnerabilities: Vec<Vulnerability>,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 last-update timestamp.
    pub updated_at: String,
    /// ISO 8601 publication timestamp, or `None` if not published.
    pub published_at: Option<String>,
    /// URL of the advisory on GitHub.
    pub html_url: String,
}

/// A specific package version range affected by a security advisory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vulnerability {
    /// Affected package.
    pub package: VulnerablePackage,
    /// Version range string (e.g. `">= 1.0.0, < 1.2.3"`), or `None`.
    pub vulnerable_version_range: Option<String>,
    /// First patched version, or `None` if no patch exists.
    pub first_patched_version: Option<String>,
    /// Severity of this specific vulnerability.
    pub severity: String,
}

/// Package identifier within a [`Vulnerability`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VulnerablePackage {
    /// Package ecosystem (e.g. `"npm"`, `"pip"`, `"cargo"`).
    pub ecosystem: String,
    /// Package name within the ecosystem.
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_advisory_deserialise_succeeds() {
        let json = serde_json::json!({
            "ghsa_id": "GHSA-1234-5678-9abc",
            "cve_id": "CVE-2023-12345",
            "summary": "Critical vulnerability in example-pkg",
            "description": "A critical vulnerability was found.",
            "severity": "critical",
            "state": "published",
            "vulnerabilities": [{
                "package": { "ecosystem": "npm", "name": "example-pkg" },
                "vulnerable_version_range": "< 1.2.3",
                "first_patched_version": "1.2.3",
                "severity": "critical"
            }],
            "created_at": "2023-01-01T00:00:00Z",
            "updated_at": "2023-01-02T00:00:00Z",
            "published_at": "2023-01-01T12:00:00Z",
            "html_url": "https://github.com/advisories/GHSA-1234-5678-9abc"
        });

        let advisory: SecurityAdvisory = serde_json::from_value(json).expect("deserialise");
        assert_eq!(advisory.ghsa_id, "GHSA-1234-5678-9abc");
        assert_eq!(advisory.severity, "critical");
        assert_eq!(advisory.vulnerabilities.len(), 1);
    }
}
