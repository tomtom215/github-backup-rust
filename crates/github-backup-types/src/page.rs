// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! [`Page`]: a list of API objects in which one unexpected object cannot hide,
//! abort or drop the others.
//!
//! GitHub list endpoints return JSON arrays.  Deserialising such an array
//! straight into `Vec<T>` fails as a whole when a *single* element does not fit
//! `T` (a `null` where the model expects an object, a new enum value, a
//! deleted account, ...), which used to abort the list, and with it every
//! later backup category of the repository.
//!
//! A [`Page<T>`] is decoded element by element instead:
//!
//! * elements that fit `T` are kept as [`Raw<T>`] (typed view plus original
//!   JSON) and are what the backup logic iterates over;
//! * elements that do not fit are kept verbatim in [`Page::unparsed`].
//!
//! Serialising a page writes **every** element's original JSON - the typed
//! ones first, in order, then the unparsed ones - so nothing the API returned
//! is lost on disk even when the typed model could not understand it.
//!
//! ```
//! use github_backup_types::{Label, Page};
//!
//! let page: Page<Label> = serde_json::from_value(serde_json::json!([
//!     {"id": 1, "name": "bug", "color": "d73a4a", "description": null, "default": true},
//!     {"id": "surprise"},
//! ]))?;
//!
//! assert_eq!(page.len(), 1);              // typed view: the label that parsed
//! assert_eq!(page.unparsed_count(), 1);   // the odd one out is kept, not dropped
//! assert_eq!(serde_json::to_value(&page)?.as_array().map(Vec::len), Some(2));
//! # Ok::<(), serde_json::Error>(())
//! ```

use std::ops::Deref;

use serde::ser::SerializeSeq;
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::raw::Raw;

/// A decoded API list: the elements that fit `T` plus the ones that did not.
///
/// It dereferences to `[Raw<T>]`, so `len()`, `is_empty()`, `iter()`,
/// indexing and `for item in &page` work on the typed elements, and each
/// element dereferences to `T`.
///
/// See the [module documentation](self) for the rationale.
#[derive(Debug, Clone)]
pub struct Page<T> {
    items: Vec<Raw<T>>,
    unparsed: Vec<Value>,
}

impl<T> Default for Page<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> PartialEq for Page<T> {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items && self.unparsed == other.unparsed
    }
}

impl<T> Eq for Page<T> {}

impl<T> Page<T> {
    /// Creates an empty page.
    #[must_use]
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            unparsed: Vec::new(),
        }
    }

    /// Appends an element that already parsed.
    pub fn push(&mut self, item: Raw<T>) {
        self.items.push(item);
    }

    /// Moves every element of `other` (typed and unparsed) to the end of
    /// this page.  Used to merge consecutive API pages.
    pub fn append(&mut self, mut other: Page<T>) {
        self.items.append(&mut other.items);
        self.unparsed.append(&mut other.unparsed);
    }

    /// The elements that did not fit `T`, exactly as the API returned them.
    #[must_use]
    pub fn unparsed(&self) -> &[Value] {
        &self.unparsed
    }

    /// Number of elements that did not fit `T`.
    #[must_use]
    pub fn unparsed_count(&self) -> usize {
        self.unparsed.len()
    }

    /// Consumes the page and returns the typed elements.
    ///
    /// The [unparsed](Self::unparsed) elements are dropped; call this only
    /// after they have been reported or written out.
    #[must_use]
    pub fn into_items(self) -> Vec<Raw<T>> {
        self.items
    }
}

impl<T: DeserializeOwned> Page<T> {
    /// Appends one element: typed when it fits `T`, kept verbatim in
    /// [`unparsed`](Self::unparsed) when it does not.
    ///
    /// Returns the deserialisation error for an element that did not fit, so
    /// the caller can report it (the element itself is then the last entry of
    /// [`unparsed`](Self::unparsed)).
    #[must_use = "an element that did not fit should be reported"]
    pub fn push_value(&mut self, value: Value) -> Option<serde_json::Error> {
        match T::deserialize(&value) {
            Ok(typed) => {
                self.items.push(Raw::from_parts(typed, value));
                None
            }
            Err(error) => {
                self.unparsed.push(value);
                Some(error)
            }
        }
    }

    /// Builds a page from raw JSON elements, silently keeping the ones that do
    /// not fit `T` in [`unparsed`](Self::unparsed).
    ///
    /// Prefer [`push_value`](Self::push_value) where the failures need to be
    /// reported (the HTTP client does).
    #[must_use]
    pub fn from_values<I: IntoIterator<Item = Value>>(values: I) -> Self {
        let mut page = Self::new();
        for value in values {
            // Dropping the error is the documented behaviour of this helper.
            let _ = page.push_value(value);
        }
        page
    }
}

impl<T: Serialize> Page<T> {
    /// Builds a page from values that were constructed in code (tests, mocks)
    /// rather than parsed from an API response; see [`Raw::from_typed`].
    ///
    /// # Panics
    ///
    /// Panics if `T`'s `Serialize` implementation fails; the model types of
    /// this crate cannot fail to serialise.
    #[must_use]
    pub fn from_typed<I: IntoIterator<Item = T>>(items: I) -> Self {
        Self {
            items: items.into_iter().map(Raw::from_typed).collect(),
            unparsed: Vec::new(),
        }
    }
}

impl<T> Deref for Page<T> {
    type Target = [Raw<T>];

    fn deref(&self) -> &[Raw<T>] {
        &self.items
    }
}

impl<'a, T> IntoIterator for &'a Page<T> {
    type Item = &'a Raw<T>;
    type IntoIter = std::slice::Iter<'a, Raw<T>>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl<T> Serialize for Page<T> {
    /// Serialises as one JSON array holding the original JSON of every
    /// element: the typed ones in order, then the unparsed ones.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.items.len() + self.unparsed.len()))?;
        for item in &self.items {
            seq.serialize_element(item.json())?;
        }
        for value in &self.unparsed {
            seq.serialize_element(value)?;
        }
        seq.end()
    }
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for Page<T> {
    /// Reads a JSON array; elements that do not fit `T` land in
    /// [`Page::unparsed`] instead of failing the whole list.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let values = Vec::<Value>::deserialize(deserializer)?;
        Ok(Self::from_values(values))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Item {
        id: u64,
        name: String,
    }

    fn good(id: u64) -> Value {
        json!({"id": id, "name": format!("item-{id}"), "extra": {"kept": true}})
    }

    #[test]
    fn well_formed_elements_are_all_typed_and_round_trip_exactly() {
        let values = vec![good(1), good(2), good(3)];

        let page: Page<Item> = Page::from_values(values.clone());

        assert_eq!(page.len(), 3);
        assert_eq!(page.unparsed_count(), 0);
        assert_eq!(page[1].name, "item-2");
        assert_eq!(
            serde_json::to_value(&page).expect("serialise"),
            Value::Array(values)
        );
    }

    #[test]
    fn a_bad_element_is_isolated_and_still_serialised() {
        let bad = json!({"id": "nope", "name": 5});
        let page: Page<Item> = Page::from_values(vec![good(1), bad.clone(), good(3)]);

        assert_eq!(page.len(), 2, "the two good elements stay typed");
        assert_eq!(page.unparsed(), std::slice::from_ref(&bad));
        // Typed elements first (in order), then the unparsed ones: nothing lost.
        assert_eq!(
            serde_json::to_value(&page).expect("serialise"),
            json!([good(1), good(3), bad])
        );
    }

    #[test]
    fn non_object_garbage_is_kept_not_dropped() {
        let garbage = vec![
            Value::Null,
            json!(42),
            json!("text"),
            json!([1, 2]),
            json!({}),
        ];
        let mut input = vec![good(1)];
        input.extend(garbage.clone());

        let page: Page<Item> = Page::from_values(input);

        assert_eq!(page.len(), 1);
        assert_eq!(page.unparsed(), garbage.as_slice());
        assert_eq!(page.unparsed_count(), 5);
    }

    #[test]
    fn push_value_reports_the_error_for_an_unfit_element() {
        let mut page: Page<Item> = Page::new();

        assert!(page.push_value(good(1)).is_none());
        let error = page.push_value(json!({"id": 2})).expect("must report");

        assert!(error.to_string().contains("missing field"), "{error}");
        assert_eq!(page.unparsed().last(), Some(&json!({"id": 2})));
    }

    #[test]
    fn append_merges_typed_and_unparsed_elements() {
        let mut first: Page<Item> = Page::from_values(vec![good(1), json!(null)]);
        let second: Page<Item> = Page::from_values(vec![good(2), json!(false)]);

        first.append(second);

        assert_eq!(first.len(), 2);
        assert_eq!(first.unparsed(), &[json!(null), json!(false)]);
    }

    #[test]
    fn empty_page_serialises_to_an_empty_array() {
        let page: Page<Item> = Page::new();

        assert!(page.is_empty());
        assert_eq!(serde_json::to_string(&page).expect("serialise"), "[]");
    }

    #[test]
    fn from_typed_builds_a_page_without_unparsed_elements() {
        let page = Page::from_typed(vec![
            Item {
                id: 1,
                name: "a".into(),
            },
            Item {
                id: 2,
                name: "b".into(),
            },
        ]);

        assert_eq!(page.len(), 2);
        assert_eq!(page.unparsed_count(), 0);
        assert_eq!(
            serde_json::to_value(&page).expect("serialise"),
            json!([{"id": 1, "name": "a"}, {"id": 2, "name": "b"}])
        );
    }

    #[test]
    fn deserialises_tolerantly_from_an_array() {
        let text = r#"[{"id":1,"name":"a","x":1}, 7, {"id":2,"name":"b"}]"#;

        let page: Page<Item> = serde_json::from_str(text).expect("parses");

        assert_eq!(page.len(), 2);
        assert_eq!(page.unparsed(), &[json!(7)]);
    }

    #[test]
    fn deserialising_a_non_array_is_still_an_error() {
        let err = serde_json::from_str::<Page<Item>>(r#"{"message":"Not Found"}"#)
            .expect_err("an object is not a list");

        assert!(err.to_string().contains("expected a sequence"), "{err}");
    }

    #[test]
    fn iterating_a_reference_yields_raw_elements_with_typed_access() {
        let page: Page<Item> = Page::from_values(vec![good(1), good(2)]);

        let ids: Vec<u64> = (&page).into_iter().map(|item| item.id).collect();
        let mut names = Vec::new();
        for item in &page {
            names.push(item.name.clone());
        }

        assert_eq!(ids, [1, 2]);
        assert_eq!(names, ["item-1", "item-2"]);
    }

    #[test]
    fn into_items_returns_the_typed_elements() {
        let page: Page<Item> = Page::from_values(vec![good(1), json!(null)]);

        let items = page.into_items();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, 1);
    }

    #[test]
    fn output_is_byte_identical_across_repeated_serialisation() {
        let page: Page<Item> = Page::from_values(vec![good(1), json!({"b": 1, "a": 2}), good(3)]);

        let first = serde_json::to_string_pretty(&page).expect("first");
        let second = serde_json::to_string_pretty(&page).expect("second");

        assert_eq!(first, second);
    }
}
