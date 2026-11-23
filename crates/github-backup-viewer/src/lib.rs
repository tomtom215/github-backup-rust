// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Renders a backup made by `github-backup` as a static HTML site that works
//! offline (`github-backup --render-html`).
//!
//! The input is one owner's backup directory (`<output>/<owner>`, holding
//! `json/` and `git/`); the output is a directory of plain HTML pages with one
//! style sheet: an index of the repositories, and per repository its overview,
//! issues, pull requests (conversation, reviews, review comments, commits),
//! releases, labels and milestones, plus the owner's gists, projects, starred
//! and watched repositories and followers.
//!
//! # Safety of the generated pages
//!
//! Issue text is written by strangers.  Every page carries a
//! Content-Security-Policy that allows no script and no remote load, Markdown
//! is rendered without raw HTML and with a URL scheme allow-list (see
//! the `markdown` module), and every other value is escaped.
//!
//! # Robustness
//!
//! A missing file is a category that was not backed up: the page shows what
//! exists.  A file that cannot be read or parsed is skipped and reported in
//! [`RenderReport::warnings`].  Output is deterministic (stable ordering, no
//! timestamps), so two renders of the same backup are byte-identical.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod html;
mod json;
mod markdown;
mod pages;
mod site;

#[cfg(test)]
mod tests;

pub use site::{render_site, RenderError, RenderOptions, RenderReport};
