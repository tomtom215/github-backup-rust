// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub REST API endpoint methods, split by resource category.
//!
//! Each submodule adds `impl GitHubClient` methods for one concern:
//!
//! | Module         | Methods                                              |
//! |---------------|------------------------------------------------------|
//! | [`repos`]     | user / org repository lists                          |
//! | [`social`]    | followers, following, starred, watched, gists         |
//! | [`issues`]    | issues, comments, timeline events                    |
//! | [`pulls`]     | pull requests, review comments, commits, reviews     |
//! | [`repo_meta`] | labels, milestones, releases, hooks, advisories, … |
//! | [`keys`]      | deploy keys, collaborators                           |
//! | [`org`]       | organisation members and teams                       |
//! | [`actions`]   | workflows, workflow runs, deployment environments    |

mod actions;
mod community;
mod issues;
mod keys;
mod org;
mod pulls;
mod repo_meta;
mod repos;
mod social;
