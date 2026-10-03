// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! [`Raw`]: a typed view of an API object that remembers the exact JSON it was
//! parsed from.
//!
//! # Why
//!
//! The model structs in this crate describe the *fields the backup logic needs*
//! (an issue's number, a release's assets, ...).  GitHub returns many more
//! (reactions, `node_id`s, URL templates, review threading, ...).  Writing a
//! re-serialised struct to disk silently drops everything the struct does not
//! name, which is not acceptable for a backup.
//!
//! [`Raw<T>`] keeps both: `T` for the code that needs to look at a few fields
//! (it [`Deref`]s to `T`) and the original [`serde_json::Value`] for the code
//! that writes the object to disk (it [`Serialize`]s as that value).  With the
//! `serde_json` `preserve_order` feature enabled workspace-wide, the value also
//! keeps GitHub's own key order, so the bytes written are deterministic.
//!
//! ```
//! use github_backup_types::{Label, Raw};
//!
//! let json = serde_json::json!({
//!     "id": 1, "node_id": "LA_1", "name": "bug", "color": "d73a4a",
//!     "description": null, "default": true,
//! });
//! let label: Raw<Label> = Raw::from_value(json.clone())?;
//! assert_eq!(label.name, "bug");                       // typed access
//! assert_eq!(serde_json::to_value(&label)?, json);     // `node_id` is not lost
//! # Ok::<(), serde_json::Error>(())
//! ```

use std::ops::Deref;

use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// A typed API object together with the exact JSON it was parsed from.
///
/// See the [module documentation](self) for the rationale.
///
/// Equality compares the JSON, i.e. two `Raw` values are equal when they would
/// be written to disk identically.
#[derive(Debug, Clone)]
pub struct Raw<T> {
    typed: T,
    json: Value,
}

impl<T> Raw<T> {
    /// Pairs an already-parsed `typed` view with the `json` it came from.
    pub(crate) fn from_parts(typed: T, json: Value) -> Self {
        Self { typed, json }
    }

    /// Returns the typed view.
    ///
    /// Usually unnecessary: `Raw<T>` dereferences to `T`.
    #[must_use]
    pub fn typed(&self) -> &T {
        &self.typed
    }

    /// Returns the original JSON, exactly as received.
    #[must_use]
    pub fn json(&self) -> &Value {
        &self.json
    }

    /// Discards the original JSON and returns the typed view.
    #[must_use]
    pub fn into_typed(self) -> T {
        self.typed
    }

    /// Discards the typed view and returns the original JSON.
    #[must_use]
    pub fn into_json(self) -> Value {
        self.json
    }
}

impl<T: DeserializeOwned> Raw<T> {
    /// Parses `json` into `T` and keeps `json` untouched next to it.
    ///
    /// # Errors
    ///
    /// Returns the deserialisation error when `json` does not fit `T`.  Use
    /// [`Page`](crate::Page) to keep objects that do not fit.
    pub fn from_value(json: Value) -> Result<Self, serde_json::Error> {
        let typed = T::deserialize(&json)?;
        Ok(Self::from_parts(typed, json))
    }
}

impl<T: Serialize> Raw<T> {
    /// Wraps a value that was built in code (tests, mocks) rather than parsed
    /// from an API response; the "original" JSON is what `typed` serialises to.
    ///
    /// # Panics
    ///
    /// Panics if `T`'s `Serialize` implementation fails.  The model types of
    /// this crate cannot fail to serialise.
    #[must_use]
    pub fn from_typed(typed: T) -> Self {
        let json = serde_json::to_value(&typed).expect("model types always serialise to JSON");
        Self::from_parts(typed, json)
    }
}

impl<T> Deref for Raw<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.typed
    }
}

impl<T> PartialEq for Raw<T> {
    fn eq(&self, other: &Self) -> bool {
        self.json == other.json
    }
}

impl<T> Eq for Raw<T> {}

impl<T> Serialize for Raw<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.json.serialize(serializer)
    }
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for Raw<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let json = Value::deserialize(deserializer)?;
        Self::from_value(json).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Item {
        id: u64,
        name: String,
        note: Option<String>,
    }

    fn sample() -> Value {
        serde_json::json!({
            "z_first": {"b": 2, "a": 1},
            "id": 7,
            "name": "seven",
            "note": null,
            "node_id": "I_7",
            "reactions": {"+1": 3},
            "labels": [{"name": "bug"}],
        })
    }

    #[test]
    fn serialises_the_original_json_including_fields_the_type_does_not_name() {
        let raw: Raw<Item> = Raw::from_value(sample()).expect("parses");

        assert_eq!(serde_json::to_value(&raw).expect("serialise"), sample());
        assert_eq!(raw.json(), &sample());
    }

    #[test]
    fn typed_view_is_reachable_through_deref() {
        let raw: Raw<Item> = Raw::from_value(sample()).expect("parses");

        assert_eq!(raw.id, 7);
        assert_eq!(raw.name, "seven");
        assert_eq!(raw.typed(), &Item { id: 7, name: "seven".into(), note: None });
        assert_eq!(raw.clone().into_typed().id, 7);
        assert_eq!(raw.into_json(), sample());
    }

    #[test]
    fn keeps_keys_in_the_order_they_arrived() {
        let text = r#"{"zeta":1,"id":7,"name":"n","alpha":{"y":1,"x":2}}"#;
        let raw: Raw<Item> = serde_json::from_str(text).expect("parses");

        // `preserve_order` keeps the arrival order; without it the keys would
        // come back alphabetically (`alpha`, `id`, `name`, `zeta`).
        assert_eq!(serde_json::to_string(&raw).expect("serialise"), text);
    }

    #[test]
    fn from_value_reports_a_type_mismatch() {
        let bad = serde_json::json!({"id": "not-a-number", "name": "x"});

        let err = Raw::<Item>::from_value(bad).expect_err("must not parse");

        assert!(err.to_string().contains("invalid type"), "{err}");
    }

    #[test]
    fn from_typed_serialises_what_the_typed_value_serialises_to() {
        let item = Item { id: 1, name: "one".into(), note: Some("n".into()) };

        let raw = Raw::from_typed(item.clone());

        assert_eq!(raw.json(), &serde_json::to_value(&item).expect("to_value"));
        assert_eq!(raw.typed(), &item);
    }

    #[test]
    fn deserialises_through_serde_from_text() {
        let text = serde_json::to_string(&sample()).expect("text");

        let raw: Raw<Item> = serde_json::from_str(&text).expect("parses");

        assert_eq!(raw.id, 7);
        assert_eq!(raw.json(), &sample());
    }

    #[test]
    fn deserialise_error_mentions_the_mismatch() {
        let err = serde_json::from_str::<Raw<Item>>(r#"{"id":"x","name":"n"}"#)
            .expect_err("must not parse");

        assert!(err.to_string().contains("invalid type"), "{err}");
    }

    #[test]
    fn equality_compares_json_not_typed_projection() {
        let a: Raw<Item> = Raw::from_value(sample()).expect("parses");
        let mut other = sample();
        other["reactions"] = serde_json::json!({"+1": 4});
        let b: Raw<Item> = Raw::from_value(other).expect("parses");

        // Same typed projection, different JSON: not equal.
        assert_eq!(a.typed(), b.typed());
        assert_ne!(a, b);
        assert_eq!(a, a.clone());
    }
}
