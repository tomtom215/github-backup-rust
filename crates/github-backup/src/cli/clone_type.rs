// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! CLI representation of the `--clone-type` flag.

use std::str::FromStr;

/// CLI representation of `--clone-type`.
///
/// Parses the human-friendly strings accepted by the `--clone-type` flag and
/// converts them to the canonical [`github_backup_types::config::CloneType`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CliCloneType {
    /// `git clone --mirror` — complete backup with all refs (default).
    #[default]
    Mirror,
    /// `git clone --bare` — bare clone without remote-tracking refs.
    Bare,
    /// `git clone` — full working-tree clone.
    Full,
    /// `git clone --depth <n>` — shallow clone with limited history.
    Shallow(u32),
}

impl CliCloneType {
    /// Converts to the corresponding [`github_backup_types::config::CloneType`].
    #[must_use]
    pub fn into_clone_type(self) -> github_backup_types::config::CloneType {
        use github_backup_types::config::CloneType;
        match self {
            Self::Mirror => CloneType::Mirror,
            Self::Bare => CloneType::Bare,
            Self::Full => CloneType::Full,
            Self::Shallow(d) => CloneType::Shallow(d),
        }
    }
}

impl FromStr for CliCloneType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mirror" => Ok(Self::Mirror),
            "bare" => Ok(Self::Bare),
            "full" => Ok(Self::Full),
            s if s.starts_with("shallow:") => {
                let depth_str = &s["shallow:".len()..];
                let depth: u32 = depth_str.parse().map_err(|_| {
                    format!("invalid depth '{depth_str}' in '{s}'; expected e.g. 'shallow:10'")
                })?;
                if depth == 0 {
                    return Err("shallow depth must be at least 1".to_string());
                }
                Ok(Self::Shallow(depth))
            }
            _ => Err(format!(
                "unknown clone type '{s}'; valid values: mirror, bare, full, shallow:<depth>"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use github_backup_types::config::CloneType;

    #[test]
    fn parse_mirror() {
        assert_eq!(
            "mirror".parse::<CliCloneType>().unwrap(),
            CliCloneType::Mirror
        );
    }

    #[test]
    fn parse_bare() {
        assert_eq!("bare".parse::<CliCloneType>().unwrap(), CliCloneType::Bare);
    }

    #[test]
    fn parse_full() {
        assert_eq!("full".parse::<CliCloneType>().unwrap(), CliCloneType::Full);
    }

    #[test]
    fn parse_shallow_valid() {
        assert_eq!(
            "shallow:10".parse::<CliCloneType>().unwrap(),
            CliCloneType::Shallow(10)
        );
    }

    #[test]
    fn parse_shallow_one() {
        assert_eq!(
            "shallow:1".parse::<CliCloneType>().unwrap(),
            CliCloneType::Shallow(1)
        );
    }

    #[test]
    fn parse_shallow_zero_is_error() {
        assert!("shallow:0".parse::<CliCloneType>().is_err());
    }

    #[test]
    fn parse_invalid_is_error() {
        assert!("invalid".parse::<CliCloneType>().is_err());
        assert!("shallow:abc".parse::<CliCloneType>().is_err());
        assert!("shallow:".parse::<CliCloneType>().is_err());
    }

    #[test]
    fn into_clone_type_mirror() {
        assert_eq!(CliCloneType::Mirror.into_clone_type(), CloneType::Mirror);
    }

    #[test]
    fn into_clone_type_bare() {
        assert_eq!(CliCloneType::Bare.into_clone_type(), CloneType::Bare);
    }

    #[test]
    fn into_clone_type_full() {
        assert_eq!(CliCloneType::Full.into_clone_type(), CloneType::Full);
    }

    #[test]
    fn into_clone_type_shallow() {
        assert_eq!(
            CliCloneType::Shallow(5).into_clone_type(),
            CloneType::Shallow(5)
        );
    }

    #[test]
    fn default_is_mirror() {
        assert_eq!(CliCloneType::default(), CliCloneType::Mirror);
    }
}
