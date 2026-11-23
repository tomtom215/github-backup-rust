// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Account look-ups made before a backup: whose token this is, and whether a
//! login names a user or an organisation.

use serde_json::Value;

use crate::error::ClientError;

use super::super::GitHubClient;
use super::encode_segment;

/// What kind of account a login names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountKind {
    /// A personal account.
    User,
    /// An organisation.
    Organization,
}

impl GitHubClient {
    /// The login of the account the credential belongs to (`GET /user`).
    ///
    /// Unlike the look-up the own-account listings make, a refusal is an
    /// error here: the caller asked for the login and has no fallback.
    ///
    /// # Errors
    ///
    /// [`ClientError::ApiError`] when GitHub refuses the request (`401` for a
    /// bad token; a GitHub App installation token cannot call it), the other
    /// errors of a request, and [`ClientError::Json`] when the answer has no
    /// `login`.
    pub async fn account_login(&self) -> Result<String, ClientError> {
        let url = format!("{}/user", self.api());
        let (user, _) = self.get_json_with_link::<Value>(&url).await?;
        user.get("login")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                ClientError::Json(serde::de::Error::custom("GET /user returned no `login`"))
            })
    }

    /// Whether `login` is a user or an organisation (`GET /users/{login}`);
    /// `None` when there is no such account (`404`).
    ///
    /// # Errors
    ///
    /// Any other failure of the request.
    pub async fn account_kind(&self, login: &str) -> Result<Option<AccountKind>, ClientError> {
        let url = format!("{}/users/{}", self.api(), encode_segment(login));
        match self.get_json_with_link::<Value>(&url).await {
            Ok((account, _)) => Ok(Some(
                if account.get("type").and_then(Value::as_str) == Some("Organization") {
                    AccountKind::Organization
                } else {
                    AccountKind::User
                },
            )),
            Err(ClientError::ApiError { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
