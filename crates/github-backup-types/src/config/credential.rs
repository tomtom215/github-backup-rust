// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub authentication credential types.

/// Authentication credential used to interact with the GitHub API.
#[derive(Debug, Clone)]
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
}
