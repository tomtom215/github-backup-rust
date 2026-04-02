// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Custom [`tracing_subscriber::Layer`] that forwards every tracing event to
//! the TUI progress channel as a [`BackupEvent::LogLine`].
//!
//! While a backup is running the normal stderr logger is suppressed; this layer
//! captures structured log output and displays it in the TUI log panel instead.

use tracing::{Event, Subscriber};
use tracing_subscriber::{layer::Context, Layer};

use crate::event::BackupEvent;
use crate::progress::ProgressTx;

/// A `tracing` [`Layer`] that sends formatted events to the TUI.
pub struct TuiTracingLayer {
    tx: ProgressTx,
}

impl TuiTracingLayer {
    pub fn new(tx: ProgressTx) -> Self {
        Self { tx }
    }
}

impl<S: Subscriber> Layer<S> for TuiTracingLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let level = *event.metadata().level();

        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);

        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();

        let level_str = match level {
            tracing::Level::ERROR => "ERROR",
            tracing::Level::WARN => "WARN",
            tracing::Level::INFO => "INFO",
            tracing::Level::DEBUG => "DEBUG",
            tracing::Level::TRACE => "TRACE",
        }
        .to_string();

        let message = visitor.format();

        // Parse well-known structured events for richer TUI updates.
        // "repository processed" events carry repo=full_name and progress=N/M.
        if level == tracing::Level::INFO {
            if message.contains("repository processed") || message.contains("repository backup") {
                // Extract repo name for repo-level progress tracking.
                if let Some(repo_name) = visitor.field_value("repo") {
                    let success = !message.contains("failed");
                    let _ = self.tx.send(BackupEvent::RepoCompleted {
                        name: repo_name,
                        success,
                        error: if success { None } else { Some(message.clone()) },
                    });
                }
            } else if message.contains("fetched repository list") {
                if let Some(count_str) = visitor.field_value("count") {
                    if let Ok(total) = count_str.parse::<u64>() {
                        let _ = self.tx.send(BackupEvent::ReposDiscovered { total });
                    }
                }
            } else if message.contains("backing up") || message.contains("dry-run: would back up") {
                if let Some(repo_name) = visitor.field_value("repo") {
                    let _ = self.tx.send(BackupEvent::RepoStarted { name: repo_name });
                }
            }
        }

        // Always emit the log line itself.
        let _ = self.tx.send(BackupEvent::LogLine {
            timestamp,
            level: level_str,
            message,
        });
    }
}

// ── Field visitor ─────────────────────────────────────────────────────────────

#[derive(Default)]
struct FieldVisitor {
    message: String,
    fields: Vec<(String, String)>,
}

impl tracing::field::Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let s = format!("{value:?}");
        if field.name() == "message" {
            // Strip surrounding quotes from debug-formatted string literals.
            self.message = if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
                s[1..s.len() - 1].replace("\\\"", "\"")
            } else {
                s
            };
        } else {
            // Strip quotes from field values too.
            let v = if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
                s[1..s.len() - 1].replace("\\\"", "\"")
            } else {
                s
            };
            self.fields.push((field.name().to_string(), v));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.fields
            .push((field.name().to_string(), format!("{value:.2}")));
    }
}

impl FieldVisitor {
    /// Formats the captured fields into a single human-readable string.
    fn format(&self) -> String {
        let mut s = self.message.clone();
        for (k, v) in &self.fields {
            if !s.is_empty() {
                s.push(' ');
            }
            s.push_str(k);
            s.push('=');
            s.push_str(v);
        }
        s
    }

    /// Looks up a specific field value by name.
    fn field_value(&self, name: &str) -> Option<String> {
        self.fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }
}
