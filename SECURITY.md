# Security Policy

## Supported versions

Only the latest release, and the `main` branch, receive security fixes.

## Reporting a vulnerability

**Please do not put the details of a vulnerability in a public GitHub issue,
pull request or discussion.**

1. **Private vulnerability reporting.**  Open
   [github.com/tomtom215/github-backup-rust/security/advisories/new](https://github.com/tomtom215/github-backup-rust/security/advisories/new)
   (needs a GitHub account).  If that page offers a form, use it: only the
   maintainer can read the report.
2. **If that page does not offer a form** (private reporting is not enabled for
   the repository), open a
   [public issue](https://github.com/tomtom215/github-backup-rust/issues/new)
   titled "Security contact request" that says only that you have a security
   report and how to reach you privately.  Do **not** include any detail of the
   vulnerability, the affected component or a proof of concept.  The maintainer
   will contact you and agree a private channel.

Include in the private report:
- A description of the vulnerability and its potential impact.
- Steps to reproduce or a proof-of-concept (if safe to share).
- The version (`github-backup --version`; say whether it is a release, the
  `edge` image or a build from `main`).
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
- Dependency vulnerabilities (RustSec advisories are checked by `cargo deny` in
  CI and daily against `main`)
- The release and container-image supply chain (workflows, attestations)

Out-of-scope:
- Vulnerabilities that require physical access to the machine running the tool
- Issues in dependencies that have no published fix and no known exploit
