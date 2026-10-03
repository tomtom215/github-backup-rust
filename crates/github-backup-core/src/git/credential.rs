// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Host-bound credentials for a git child process.
//!
//! The token is handed to git through an environment variable of the child and
//! an inline, URL-scoped credential helper.  Nothing is written to disk and the
//! token never appears in any argument list (`/proc/<pid>/cmdline`).
//!
//! * `-c credential.helper=` first resets the helper list, so a helper the user
//!   configured (`store`, `osxkeychain`, ...) can neither persist the token nor
//!   override it with a stored one.
//! * `-c credential.<scheme://host[:port]>.helper=...` applies only to requests
//!   for that origin.  A redirect, a hostile `.lfsconfig` or a submodule URL
//!   pointing elsewhere is never offered the token, and neither is git-lfs
//!   talking to such a host (it asks git for credentials, and git consults the
//!   same configuration).
//! * The helper answers only the `get` action; `store`/`erase` are ignored.

/// Name of the environment variable that carries the token to the helper.
pub(super) const TOKEN_ENV: &str = "GH_BACKUP_GIT_TOKEN";

/// The inline helper.  The leading `!` makes git run it through the shell.
const HELPER: &str = "!f() { test \"$1\" = get && printf 'username=x-access-token\\npassword=%s\\n' \"$GH_BACKUP_GIT_TOKEN\"; }; f";

/// Returns the `scheme://host[:port]` origin of an HTTP(S) clone URL, or `None`
/// for SSH, scp-like and local-path URLs (which never receive the token).
pub(super) fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if host.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{}", host.to_ascii_lowercase()))
}

/// The global `git -c ...` arguments that make the token available to `url`'s
/// origin only.  Empty when `url` is not an HTTP(S) URL.
pub(super) fn config_args(url: &str) -> Vec<String> {
    let Some(origin) = origin(url) else {
        return Vec::new();
    };
    vec![
        "-c".into(),
        "credential.helper=".into(),
        "-c".into(),
        format!("credential.{origin}.helper={HELPER}"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_of_https_and_http_urls() {
        assert_eq!(
            origin("https://github.com/o/r.git").as_deref(),
            Some("https://github.com")
        );
        assert_eq!(
            origin("HTTPS://User:pw@GHES.example.com:8443/o/r.git").as_deref(),
            Some("https://ghes.example.com:8443")
        );
        assert_eq!(
            origin("http://127.0.0.1:9/x").as_deref(),
            Some("http://127.0.0.1:9")
        );
    }

    #[test]
    fn no_origin_for_ssh_scp_and_paths() {
        assert_eq!(origin("git@github.com:o/r.git"), None);
        assert_eq!(origin("ssh://git@github.com/o/r.git"), None);
        assert_eq!(origin("/srv/repos/r.git"), None);
        assert_eq!(origin("file:///srv/r.git"), None);
        assert!(config_args("git@github.com:o/r.git").is_empty());
    }

    #[test]
    fn helper_is_scoped_and_the_list_is_reset_first() {
        let args = config_args("https://ghes.example.com/o/r.git");
        assert_eq!(args[1], "credential.helper=");
        assert!(args[3].starts_with("credential.https://ghes.example.com.helper=!"));
    }
}
