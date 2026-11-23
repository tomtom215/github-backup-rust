// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GraphQL queries, for data the REST API does not offer.

use bytes::Bytes;
use hyper::Method;
use serde_json::{json, Map, Value};

use crate::error::ClientError;

use super::{Call, GitHubClient};

impl GitHubClient {
    /// Runs a GraphQL **query** with `variables` and returns its `data`.
    ///
    /// The query is posted to the GraphQL endpoint of the API base
    /// (`https://api.github.com/graphql`; on GitHub Enterprise Server
    /// `https://HOST/api/graphql`) with the same credential, redirect,
    /// rate-limit and retry handling as a REST `GET`: a query changes
    /// nothing, so it is repeated after a 5xx response or a transport
    /// failure.  GraphQL's own rate-limit reports - a `200` whose `errors`
    /// say `RATE_LIMITED` or "rate limit" - are waited out like a `403`/`429`.
    ///
    /// Do not send mutations through this method: a repeated mutation could
    /// act twice.
    ///
    /// # Errors
    ///
    /// [`ClientError::GraphQl`] when the response carries a non-empty
    /// `errors` array, even next to partial `data` (a backup must not keep a
    /// partial answer as if it were complete); its `kinds` lists the error
    /// types, such as `NOT_FOUND` or `FORBIDDEN`.  [`ClientError::Json`] when
    /// the response has neither `data` nor `errors`.  Otherwise the errors of
    /// any API request.
    pub async fn graphql(&self, query: &str, variables: Value) -> Result<Value, ClientError> {
        let url = graphql_url(&self.api_base);
        let body = serde_json::to_vec(&json!({"query": query, "variables": variables}))?;
        let call = Call {
            method: Method::POST,
            body: Bytes::from(body),
            graphql: true,
            ..Call::get(&url, &[("content-type", "application/json")])
        };
        let (bytes, _) = self.execute(&call).await?;
        let mut response: Map<String, Value> = serde_json::from_slice(&bytes)?;
        if let Some(errors) = response
            .get("errors")
            .and_then(Value::as_array)
            .filter(|errors| !errors.is_empty())
        {
            return Err(graphql_error(errors));
        }
        match response.remove("data") {
            Some(data) if !data.is_null() => Ok(data),
            _ => Err(ClientError::Json(serde::de::Error::custom(format!(
                "GraphQL response from {url} has neither `data` nor `errors`"
            )))),
        }
    }
}

/// The GraphQL endpoint of a REST API base URL: `{base}/graphql`, except on
/// GitHub Enterprise Server, where the REST API is `https://HOST/api/v3` and
/// GraphQL is `https://HOST/api/graphql`.
fn graphql_url(api_base: &str) -> String {
    match api_base.strip_suffix("/api/v3") {
        Some(host) => format!("{host}/api/graphql"),
        None => format!("{api_base}/graphql"),
    }
}

fn graphql_error(errors: &[Value]) -> ClientError {
    let field =
        |error: &Value, name: &str| error.get(name).and_then(Value::as_str).map(str::to_string);
    let message = errors
        .iter()
        .map(|error| field(error, "message").unwrap_or_else(|| error.to_string()))
        .collect::<Vec<_>>()
        .join("; ");
    let kinds = errors
        .iter()
        .filter_map(|error| field(error, "type"))
        .collect();
    ClientError::GraphQl { message, kinds }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_graphql_endpoint_follows_the_api_base() {
        assert_eq!(
            graphql_url("https://api.github.com"),
            "https://api.github.com/graphql"
        );
        assert_eq!(
            graphql_url("https://ghe.example.com/api/v3"),
            "https://ghe.example.com/api/graphql"
        );
    }

    #[test]
    fn every_error_message_and_type_is_kept() {
        let errors = [
            json!({"type": "NOT_FOUND", "message": "Could not resolve to a Repository"}),
            json!({"message": "Something else"}),
        ];
        let ClientError::GraphQl { message, kinds } = graphql_error(&errors) else {
            panic!("not a GraphQL error");
        };
        assert_eq!(message, "Could not resolve to a Repository; Something else");
        assert_eq!(kinds, ["NOT_FOUND"]);
    }
}
