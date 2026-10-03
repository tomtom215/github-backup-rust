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
///
/// Deserialisation additionally accepts the command-line spelling
/// `"shallow:<depth>"`, so a config file can say `clone_type = "shallow:3"`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
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

impl<'de> Deserialize<'de> for CloneType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{Error, MapAccess, Visitor};

        struct V;

        impl<'de> Visitor<'de> for V {
            type Value = CloneType;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(
                    "\"mirror\", \"bare\", \"full\", \"shallow:<depth>\" or { shallow = <depth> }",
                )
            }

            fn visit_str<E: Error>(self, v: &str) -> Result<CloneType, E> {
                match v {
                    "mirror" => Ok(CloneType::Mirror),
                    "bare" => Ok(CloneType::Bare),
                    "full" => Ok(CloneType::Full),
                    other => {
                        let depth = other
                            .strip_prefix("shallow:")
                            .and_then(|d| d.parse::<u32>().ok())
                            .filter(|d| *d >= 1)
                            .ok_or_else(|| {
                                E::custom(format!(
                                    "unknown clone type '{other}'; valid values: mirror, bare, \
                                     full, shallow:<depth> (depth >= 1)"
                                ))
                            })?;
                        Ok(CloneType::Shallow(depth))
                    }
                }
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<CloneType, A::Error> {
                let key: String = map
                    .next_key()?
                    .ok_or_else(|| A::Error::custom("expected a `shallow` key"))?;
                if key != "shallow" {
                    return Err(A::Error::unknown_field(&key, &["shallow"]));
                }
                let depth: u32 = map.next_value()?;
                if depth == 0 {
                    return Err(A::Error::custom("shallow depth must be at least 1"));
                }
                Ok(CloneType::Shallow(depth))
            }
        }

        deserializer.deserialize_any(V)
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[derive(Deserialize)]
    struct Wrap {
        clone_type: CloneType,
    }

    fn toml_value(src: &str) -> Result<CloneType, toml::de::Error> {
        toml::from_str::<Wrap>(src).map(|w| w.clone_type)
    }

    #[test]
    fn every_documented_spelling_is_accepted_in_a_config_file() {
        assert_eq!(
            toml_value(r#"clone_type = "mirror""#).unwrap(),
            CloneType::Mirror
        );
        assert_eq!(
            toml_value(r#"clone_type = "bare""#).unwrap(),
            CloneType::Bare
        );
        assert_eq!(
            toml_value(r#"clone_type = "full""#).unwrap(),
            CloneType::Full
        );
        assert_eq!(
            toml_value(r#"clone_type = "shallow:3""#).unwrap(),
            CloneType::Shallow(3)
        );
        assert_eq!(
            toml_value("clone_type = { shallow = 7 }").unwrap(),
            CloneType::Shallow(7)
        );
    }

    #[test]
    fn bad_values_say_what_is_valid() {
        for bad in [
            r#"clone_type = "shallow:0""#,
            r#"clone_type = "shallow:x""#,
            r#"clone_type = "deep""#,
            "clone_type = { shallow = 0 }",
        ] {
            assert!(toml_value(bad).is_err(), "{bad}");
        }
        let msg = toml_value(r#"clone_type = "deep""#)
            .unwrap_err()
            .to_string();
        assert!(msg.contains("shallow:<depth>"), "{msg}");
    }

    #[test]
    fn json_round_trip_of_the_serialised_form_still_works() {
        for ct in [
            CloneType::Mirror,
            CloneType::Bare,
            CloneType::Full,
            CloneType::Shallow(5),
        ] {
            let json = serde_json::to_string(&ct).unwrap();
            assert_eq!(
                serde_json::from_str::<CloneType>(&json).unwrap(),
                ct,
                "{json}"
            );
        }
    }
}
