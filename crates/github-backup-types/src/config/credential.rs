// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub authentication credential types.

/// Authentication credential used to interact with the GitHub API.
///
/// `Debug` is implemented manually so a token is **never** printed in full:
/// only the variant name (and a short tag indicating that a token is set)
/// appears.  Use [`Credential::token`] when you actually need the value.
#[derive(Clone)]
pub enum Credential {
    /// Classic or fine-grained personal access token.
    ///
    /// Used as `Authorization: Bearer <token>` on every API request.
    Token(String),
    /// No authentication — unauthenticated requests only.
    ///
    /// GitHub allows unauthenticated access to **public** data with a rate
    /// limit of 60 requests per hour.  Use a token for higher limits and
    /// access to private resources.
    Anonymous,
}

impl Credential {
    /// Returns the `Authorization` header value for this credential, or
    /// `None` for [`Credential::Anonymous`] (no header should be sent).
    #[must_use]
    pub fn authorization_header(&self) -> Option<String> {
        match self {
            Credential::Token(t) => Some(format!("Bearer {t}")),
            Credential::Anonymous => None,
        }
    }

    /// Returns the underlying token value, or `None` for
    /// [`Credential::Anonymous`].
    ///
    /// Provided so call-sites that need the literal token (e.g. for
    /// `GIT_ASKPASS` HTTPS auth) can avoid relying on the `Debug` impl,
    /// which deliberately redacts the value.
    #[must_use]
    pub fn token(&self) -> Option<&str> {
        match self {
            Credential::Token(t) => Some(t.as_str()),
            Credential::Anonymous => None,
        }
    }
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Surface the variant + a placeholder so logs are still useful.
            // The literal token is *never* part of the formatted output.
            Credential::Token(_) => write!(f, "Credential::Token([redacted])"),
            Credential::Anonymous => write!(f, "Credential::Anonymous"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_token() {
        let cred = Credential::Token("ghp_dont_leak_me".to_string());
        let dbg = format!("{cred:?}");
        assert!(!dbg.contains("ghp_dont_leak_me"), "Debug must redact token");
        assert!(dbg.contains("[redacted]"));
    }

    #[test]
    fn debug_shows_anonymous_variant() {
        assert_eq!(
            format!("{:?}", Credential::Anonymous),
            "Credential::Anonymous"
        );
    }

    #[test]
    fn authorization_header_is_bearer_token() {
        let cred = Credential::Token("abc".to_string());
        assert_eq!(cred.authorization_header(), Some("Bearer abc".to_string()));
    }

    #[test]
    fn authorization_header_is_none_for_anonymous() {
        assert!(Credential::Anonymous.authorization_header().is_none());
    }

    #[test]
    fn token_returns_inner_value() {
        let cred = Credential::Token("inner".to_string());
        assert_eq!(cred.token(), Some("inner"));
        assert!(Credential::Anonymous.token().is_none());
    }
}
