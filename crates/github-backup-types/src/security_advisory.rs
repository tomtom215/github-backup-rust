// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Repository security advisory type.
//!
//! Modelled on the `repository-advisory` schema of GitHub's OpenAPI
//! description (`GET /repos/{owner}/{repo}/security-advisories`).  Draft
//! advisories legitimately carry `null` for the severity, the timestamps and
//! the vulnerability list, so those are optional here.

use serde::{Deserialize, Serialize};

/// A repository security advisory (published, draft, closed or withdrawn).
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
    /// Severity: `"critical"`, `"high"`, `"medium"`, `"low"`; `None` for a
    /// draft that has none yet.
    pub severity: Option<String>,
    /// Advisory state: `"published"`, `"closed"`, `"withdrawn"`, `"draft"`,
    /// `"triage"`.
    pub state: String,
    /// Vulnerable package references; `None` when GitHub sends `null`.
    pub vulnerabilities: Option<Vec<Vulnerability>>,
    /// ISO 8601 creation timestamp (`null` on some drafts).
    pub created_at: Option<String>,
    /// ISO 8601 last-update timestamp (`null` on some drafts).
    pub updated_at: Option<String>,
    /// ISO 8601 publication timestamp, or `None` if not published.
    pub published_at: Option<String>,
    /// URL of the advisory on GitHub.
    pub html_url: String,
}

/// A specific package version range affected by a security advisory.
///
/// Exactly the four properties GitHub defines for an entry of
/// `vulnerabilities`; there is no per-vulnerability severity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vulnerability {
    /// Affected package; `None` when GitHub sends `null`.
    pub package: Option<VulnerablePackage>,
    /// Version range string (e.g. `">= 1.0.0, < 1.2.3"`), or `None`.
    pub vulnerable_version_range: Option<String>,
    /// Patched version(s), or `None` if no patch exists.
    pub patched_versions: Option<String>,
    /// Names of the vulnerable functions, or `None` if unspecified.
    pub vulnerable_functions: Option<Vec<String>>,
}

/// Package identifier within a [`Vulnerability`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VulnerablePackage {
    /// Package ecosystem (e.g. `"npm"`, `"pip"`, `"rust"`).
    pub ecosystem: String,
    /// Package name within the ecosystem; `None` when GitHub sends `null`.
    pub name: Option<String>,
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
                "patched_versions": "1.2.3",
                "vulnerable_functions": ["parse"]
            }],
            "created_at": "2023-01-01T00:00:00Z",
            "updated_at": "2023-01-02T00:00:00Z",
            "published_at": "2023-01-01T12:00:00Z",
            "html_url": "https://github.com/advisories/GHSA-1234-5678-9abc"
        });

        let advisory: SecurityAdvisory = serde_json::from_value(json).expect("deserialise");
        assert_eq!(advisory.ghsa_id, "GHSA-1234-5678-9abc");
        assert_eq!(advisory.severity.as_deref(), Some("critical"));
        let vulns = advisory.vulnerabilities.expect("vulnerabilities");
        assert_eq!(vulns.len(), 1);
        assert_eq!(vulns[0].patched_versions.as_deref(), Some("1.2.3"));
        assert_eq!(
            vulns[0].vulnerable_functions.as_deref(),
            Some(&["parse".to_string()][..])
        );
    }

    #[test]
    fn vulnerability_has_no_severity_and_parses_the_documented_shape() {
        // The shape of the spec's own `repository-advisory` example: four
        // properties and no `severity`, which the previous model required.
        let json = serde_json::json!({
            "package": { "ecosystem": "npm", "name": "a-package" },
            "vulnerable_version_range": ">= 1.0.0, < 1.0.1",
            "patched_versions": "1.0.1",
            "vulnerable_functions": ["important_function"]
        });

        let vuln: Vulnerability = serde_json::from_value(json).expect("deserialise");

        assert_eq!(
            vuln.package.and_then(|p| p.name).as_deref(),
            Some("a-package")
        );
    }

    #[test]
    fn draft_advisory_with_null_severity_timestamps_and_vulnerabilities_parses() {
        let json = serde_json::json!({
            "ghsa_id": "GHSA-aaaa-bbbb-cccc",
            "cve_id": null,
            "summary": "Draft",
            "description": null,
            "severity": null,
            "state": "draft",
            "vulnerabilities": null,
            "created_at": null,
            "updated_at": null,
            "published_at": null,
            "html_url": "https://github.com/o/r/security/advisories/GHSA-aaaa-bbbb-cccc"
        });

        let advisory: SecurityAdvisory = serde_json::from_value(json).expect("deserialise");

        assert!(advisory.severity.is_none());
        assert!(advisory.vulnerabilities.is_none());
        assert!(advisory.created_at.is_none() && advisory.updated_at.is_none());
    }

    #[test]
    fn vulnerability_accepts_null_package_and_null_package_name() {
        let null_package: Vulnerability = serde_json::from_value(serde_json::json!({
            "package": null,
            "vulnerable_version_range": null,
            "patched_versions": null,
            "vulnerable_functions": null
        }))
        .expect("null package");
        let null_name: Vulnerability = serde_json::from_value(serde_json::json!({
            "package": { "ecosystem": "pip", "name": null }
        }))
        .expect("null name");

        assert!(null_package.package.is_none());
        assert_eq!(null_name.package.map(|p| p.name), Some(None));
    }
}
