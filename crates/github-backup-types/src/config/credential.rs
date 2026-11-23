// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub authentication credential types.

use zeroize::Zeroizing;

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
    /// A GitHub App, acting as one of its installations.
    ///
    /// The API client signs a short-lived JWT with the App's private key,
    /// exchanges it for an installation access token (valid for one hour) and
    /// renews that token before it expires.
    App(AppCredential),
    /// No authentication — unauthenticated requests only.
    ///
    /// GitHub allows unauthenticated access to **public** data with a rate
    /// limit of 60 requests per hour.  Use a token for higher limits and
    /// access to private resources.
    Anonymous,
}

impl Credential {
    /// Returns the `Authorization` header value for this credential, or
    /// `None` when there is no fixed one: for [`Credential::Anonymous`] (no
    /// header should be sent) and for [`Credential::App`], whose token the API
    /// client mints and renews.
    #[must_use]
    pub fn authorization_header(&self) -> Option<String> {
        self.token().map(|t| format!("Bearer {t}"))
    }

    /// Returns the underlying token value, or `None` for
    /// [`Credential::Anonymous`] and [`Credential::App`] (whose token changes
    /// while a run lasts).
    ///
    /// Provided so call-sites that need the literal token (e.g. for
    /// `GIT_ASKPASS` HTTPS auth) can avoid relying on the `Debug` impl,
    /// which deliberately redacts the value.
    #[must_use]
    pub fn token(&self) -> Option<&str> {
        match self {
            Credential::Token(t) => Some(t.as_str()),
            Credential::App(_) | Credential::Anonymous => None,
        }
    }
}

/// The settings of a GitHub App credential.
///
/// `Debug` shows the issuer and the installation, never the private key.
#[derive(Clone)]
pub struct AppCredential {
    /// The JWT issuer: the App's client ID (`Iv23...`, which GitHub
    /// recommends) or its numeric App ID.
    pub app_id: String,
    /// The App's private key in PEM form (PKCS#1 `RSA PRIVATE KEY`, as GitHub
    /// issues it, or unencrypted PKCS#8 `PRIVATE KEY`).  Wiped from memory
    /// when dropped.
    pub private_key_pem: Zeroizing<String>,
    /// The installation to act as; `None` until it has been looked up from
    /// the account being backed up.
    pub installation_id: Option<u64>,
}

impl std::fmt::Debug for AppCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppCredential")
            .field("app_id", &self.app_id)
            .field("private_key_pem", &"[redacted]")
            .field("installation_id", &self.installation_id)
            .finish()
    }
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Surface the variant + a placeholder so logs are still useful.
            // The literal token is *never* part of the formatted output.
            Credential::Token(_) => write!(f, "Credential::Token([redacted])"),
            Credential::App(app) => write!(f, "Credential::{app:?}"),
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
    fn debug_redacts_the_app_key_and_there_is_no_fixed_token() {
        let cred = Credential::App(AppCredential {
            app_id: "Iv23abc".into(),
            private_key_pem: Zeroizing::new("-----BEGIN RSA PRIVATE KEY-----\nSECRETKEY".into()),
            installation_id: Some(7),
        });
        let dbg = format!("{cred:?}");
        assert!(!dbg.contains("SECRETKEY"), "{dbg}");
        assert!(dbg.contains("Iv23abc") && dbg.contains("Some(7)"), "{dbg}");
        assert_eq!(cred.token(), None);
        assert_eq!(cred.authorization_header(), None);
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
