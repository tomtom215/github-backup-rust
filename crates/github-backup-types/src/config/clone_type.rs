// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Clone-depth strategy for repository mirroring.

use serde::{Deserialize, Serialize};

/// Selects how repositories are cloned during backup.
///
/// The default ([`CloneType::Mirror`]) produces a bare mirror suitable for
/// complete backups and restores.  Other modes trade completeness for clone
/// speed or working-tree access.
///
/// # Serialisation
///
/// Unit variants serialise as lowercase strings (`"mirror"`, `"bare"`,
/// `"full"`).  The shallow variant serialises as `{"shallow": <depth>}`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CloneType {
    /// `git clone --mirror` — fetches all refs (branches, tags, notes, …).
    ///
    /// The result is a bare repository that mirrors the remote exactly.
    /// This is the recommended choice for backup purposes because it captures
    /// the complete repository state and can be restored with `git clone`.
    #[default]
    Mirror,
    /// `git clone --bare` — bare clone without remote-tracking metadata.
    ///
    /// Similar to `Mirror` but does not set up remote-tracking refs.  Slightly
    /// smaller than a mirror.
    Bare,
    /// Standard `git clone` — creates a full working-tree clone.
    ///
    /// Use this if you need to browse or build the backed-up source code
    /// directly.  Requires more disk space than bare clones.
    Full,
    /// `git clone --depth <n>` — shallow clone with limited commit history.
    ///
    /// Significantly reduces disk usage at the cost of losing history beyond
    /// `depth` commits per branch.  Not suitable for archival backups.
    Shallow(u32),
}
