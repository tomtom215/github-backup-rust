// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Element-by-element decoding of list responses into [`Page`]s.
//!
//! A list response is decoded as `Vec<serde_json::Value>` first and each
//! element is then converted to its typed model on its own.  One element that
//! does not fit (a `null` where the model expects an object, a deleted
//! account, a property of an unexpected type) is therefore kept verbatim in
//! [`Page::unparsed`] and reported, instead of failing the whole list - and,
//! with it, every later backup category of the repository.

use serde::de::DeserializeOwned;
use serde_json::Value;
use tracing::warn;

use github_backup_types::Page;

/// Appends the `values` of one API response to `page`.
///
/// An element that does not fit `T` is not an error.  It is kept as received
/// in [`Page::unparsed`] (so it is still written to the backup) and reported
/// with a `warn!` that names the endpoint `url`, the element's `index` in that
/// response and its `id` / `number` when it has them; decoding then continues
/// with the next element.
pub(crate) fn extend_page<T: DeserializeOwned>(page: &mut Page<T>, url: &str, values: Vec<Value>) {
    for (index, value) in values.into_iter().enumerate() {
        if let Some(error) = page.push_value(value) {
            warn!(
                url = %url,
                index,
                id = %identify(page.unparsed().last()),
                error = %error,
                "API object does not fit its typed model; \
                 keeping its JSON as received and continuing"
            );
        }
    }
}

/// Describes an element for log messages: its `id` and/or `number` if present.
fn identify(value: Option<&Value>) -> String {
    let Some(object) = value.and_then(Value::as_object) else {
        return "none (not an object)".to_string();
    };
    let mut parts = Vec::new();
    if let Some(id) = object.get("id") {
        parts.push(format!("id={id}"));
    }
    if let Some(number) = object.get("number") {
        parts.push(format!("number={number}"));
    }
    if parts.is_empty() {
        "none".to_string()
    } else {
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;
    use std::io;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)] // the fields only decide whether an element fits
    struct Item {
        id: u64,
        title: String,
        user: Option<String>,
    }

    /// Collects what a `tracing` subscriber writes.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("capture lock").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Capture {
        type Writer = Capture;

        fn make_writer(&'a self) -> Capture {
            self.clone()
        }
    }

    impl Capture {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().expect("capture lock").clone()).expect("utf-8 log")
        }
    }

    /// Runs `body` with a subscriber that records every event.
    fn captured(body: impl FnOnce()) -> String {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, body);
        capture.text()
    }

    fn good(id: u64) -> Value {
        json!({"id": id, "title": format!("item {id}"), "user": null, "extra": [1, 2, 3]})
    }

    #[test]
    fn well_formed_elements_are_typed_and_nothing_is_logged() {
        let mut page: Page<Item> = Page::new();

        let log =
            captured(|| extend_page(&mut page, "https://api.test/items", vec![good(1), good(2)]));

        assert_eq!(page.len(), 2);
        assert_eq!(page.unparsed_count(), 0);
        assert_eq!(page[1].id, 2);
        assert!(log.is_empty(), "unexpected log output: {log}");
    }

    #[test]
    fn null_absent_and_garbage_elements_are_isolated_not_fatal() {
        let absent = json!({"id": 3});
        let wrong_type = json!({"id": 4, "number": 9, "title": 5});
        let values = vec![
            good(1),
            Value::Null,        // index 1: null
            absent.clone(),     // index 2: `title` absent
            json!(42),          // index 3: a number
            wrong_type.clone(), // index 4: wrong type
            json!("text"),      // index 5: a string
            good(2),
        ];
        let mut page: Page<Item> = Page::new();

        let log = captured(|| extend_page(&mut page, "https://api.test/items?page=1", values));

        assert_eq!(page.len(), 2, "the two real items stay typed");
        assert_eq!(page.unparsed_count(), 5);
        assert_eq!(page.unparsed()[1], absent);
        assert_eq!(page.unparsed()[3], wrong_type);
        // Every element is still written to disk, the unparsed ones verbatim.
        let written = serde_json::to_value(&page).expect("serialise");
        assert_eq!(written.as_array().map(Vec::len), Some(7));
        assert_eq!(written[2], Value::Null);
        // ...and every one of them was reported.
        assert_eq!(
            log.matches("WARN").count(),
            5,
            "one warning per bad element:\n{log}"
        );
        assert!(log.contains("https://api.test/items?page=1"), "{log}");
    }

    #[test]
    fn warning_names_the_endpoint_the_index_and_the_id_or_number() {
        let mut page: Page<Item> = Page::new();
        let values = vec![good(1), json!({"id": 77, "number": 5, "title": 5})];

        let log = captured(|| extend_page(&mut page, "https://api.test/repos/o/r/issues", values));

        assert!(log.contains("https://api.test/repos/o/r/issues"), "{log}");
        assert!(log.contains("index=1"), "{log}");
        assert!(log.contains("id=77 number=5"), "{log}");
        assert!(
            log.contains("invalid type"),
            "serde error is included: {log}"
        );
    }

    #[test]
    fn identify_handles_objects_without_ids_and_non_objects() {
        assert_eq!(identify(Some(&json!({"title": "x"}))), "none");
        assert_eq!(identify(Some(&json!({"number": 3}))), "number=3");
        assert_eq!(identify(Some(&json!({"id": "abc"}))), "id=\"abc\"");
        assert_eq!(identify(Some(&json!(7))), "none (not an object)");
        assert_eq!(identify(None), "none (not an object)");
    }
}
