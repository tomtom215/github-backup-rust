# Security Policy

## Supported versions

Only the latest release on the `main` branch receives security fixes.

## Reporting a vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

Send a report to the maintainer via the
[GitHub private vulnerability reporting](https://github.com/tomtom215/github-backup-rust/security/advisories/new)
feature (requires a GitHub account).

Include:
- A description of the vulnerability and its potential impact.
- Steps to reproduce or a proof-of-concept (if safe to share).
- Any suggested mitigations you have identified.

You will receive an acknowledgement within 72 hours. We aim to release a fix
within 14 days for critical issues and 90 days for lower-severity findings.
We will coordinate a disclosure date with you before publishing a CVE or
security advisory.

## Scope

In-scope vulnerabilities include:
- Credential leakage (tokens written to disk or logged in plaintext)
- Path traversal or arbitrary file write during backup/restore
- Command injection in git subprocess invocations
- Dependency vulnerabilities (tracked automatically by `cargo audit` in CI)

Out-of-scope:
- Vulnerabilities that require physical access to the machine running the tool
- Issues in dependencies that have no published fix and no known exploit
