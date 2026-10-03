// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Command-line argument parsing.
//!
//! The module is split into focused sub-modules:
//!
//! - [`args`] — the top-level [`Args`] struct
//! - [`args_impl`] — `merge_config` and `into_backup_options` implementations
//! - [`clone_type`] — the `--clone-type` flag parser (`CliCloneType`)

pub mod args;
mod args_impl;
pub mod clone_type;

pub use args::Args;

/// Argument parsing for unit tests that must not depend on the machine they
/// run on.
///
/// Several options read environment variables (`GITHUB_TOKEN`,
/// `AWS_ACCESS_KEY_ID`, …).  Parsing through plain `Args::parse_from` would
/// therefore make test outcomes depend on the developer's shell, and a clap
/// error inside `parse_from` calls `process::exit`, which aborts the whole
/// test binary.  These helpers disable every `env =` binding first.
#[cfg(test)]
pub(crate) mod test_support {
    use clap::{ArgMatches, CommandFactory, FromArgMatches};

    use super::Args;

    /// Parses `argv` (including the program name) ignoring the environment.
    pub(crate) fn try_parse(argv: &[&str]) -> Result<Args, clap::Error> {
        try_parse_with_matches(argv).map(|(args, _)| args)
    }

    /// Like [`try_parse`], also returning the matches (for value sources).
    pub(crate) fn try_parse_with_matches(argv: &[&str]) -> Result<(Args, ArgMatches), clap::Error> {
        let matches = Args::command()
            .mut_args(|arg| arg.env(None::<&str>))
            .try_get_matches_from(argv.iter().copied())?;
        let args = Args::from_arg_matches(&matches)?;
        Ok((args, matches))
    }

    /// Parses `argv`, panicking with clap's message on error.
    pub(crate) fn parse(argv: &[&str]) -> Args {
        try_parse(argv).unwrap_or_else(|e| panic!("{e}"))
    }
}
