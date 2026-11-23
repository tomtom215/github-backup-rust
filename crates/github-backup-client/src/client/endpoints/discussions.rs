// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! GitHub Discussions, through the GraphQL API: the REST API has no route
//! for them.
//!
//! Every list is a GraphQL connection of at most 100 nodes per page, followed
//! with cursors to its end; a connection nested in a node (a discussion's
//! labels, a comment's replies) is completed with `node(id:)` queries and
//! spliced back into that node, so nothing is ever cut off at a page size.
//! The nodes are returned exactly as GitHub sent them.
//!
//! # Cost
//!
//! GraphQL charges points, not requests: the number of connection pages a
//! query could need (each nested connection counts once per parent node it
//! may appear in), divided by 100.  The page sizes below keep each query at
//! about one point: 50 discussions with their labels and poll options
//! (1 + 50 + 50 pages), 50 comments with up to 50 replies each (1 + 50).
//! Every query also asks for `rateLimit { cost remaining resetAt }`, which is
//! logged at debug level and not part of the returned data.

use serde_json::{json, Value};
use tracing::debug;

use crate::api_client::NodePage;
use crate::error::ClientError;

use super::super::GitHubClient;

/// The fields of a user, an app or another actor.
macro_rules! actor {
    () => {
        "{ __typename login url avatarUrl }"
    };
}

/// `pageInfo` of a connection.
macro_rules! page_info {
    () => {
        "pageInfo { hasNextPage endCursor }"
    };
}

/// The fields of a label.
macro_rules! label {
    () => {
        "id name color description"
    };
}

/// The fields of a poll option.
macro_rules! poll_option {
    () => {
        "id option totalVoteCount"
    };
}

/// The fields of a discussion comment or reply (without its replies).
macro_rules! comment {
    () => {
        concat!(
            "id databaseId url body createdAt updatedAt publishedAt lastEditedAt deletedAt ",
            "createdViaEmail includesCreatedEdit author ",
            actor!(),
            " authorAssociation editor ",
            actor!(),
            " isAnswer isMinimized minimizedReason upvoteCount ",
            "reactionGroups { content reactors { totalCount } } replyTo { id }"
        )
    };
}

/// Asked for by every query; removed from the answer and logged.
macro_rules! rate_limit {
    () => {
        "rateLimit { cost remaining resetAt }"
    };
}

const CATEGORIES_QUERY: &str = concat!(
    "query($owner: String!, $name: String!, $after: String) { ",
    rate_limit!(),
    " repository(owner: $owner, name: $name) { discussionCategories(first: 100, after: $after) { ",
    page_info!(),
    " nodes { id name slug emoji emojiHTML description isAnswerable createdAt updatedAt } } } }"
);

const DISCUSSIONS_QUERY: &str = concat!(
    "query($owner: String!, $name: String!, $after: String) { ",
    rate_limit!(),
    " repository(owner: $owner, name: $name) { ",
    "discussions(first: 50, after: $after, orderBy: {field: UPDATED_AT, direction: DESC}) { ",
    page_info!(),
    " nodes { id databaseId number title body url resourcePath ",
    "createdAt updatedAt publishedAt lastEditedAt createdViaEmail includesCreatedEdit ",
    "author ",
    actor!(),
    " authorAssociation editor ",
    actor!(),
    " category { id name slug emoji description isAnswerable } ",
    "closed closedAt stateReason locked activeLockReason ",
    "isAnswered answerChosenAt answerChosenBy ",
    actor!(),
    " answer { id databaseId url } upvoteCount ",
    "reactionGroups { content reactors { totalCount } } ",
    "comments { totalCount } ",
    "labels(first: 100) { totalCount ",
    page_info!(),
    " nodes { ",
    label!(),
    " } } ",
    "poll { id question totalVoteCount options(first: 100) { totalCount ",
    page_info!(),
    " nodes { ",
    poll_option!(),
    " } } } } } } }"
);

const LABELS_QUERY: &str = concat!(
    "query($id: ID!, $after: String) { ",
    rate_limit!(),
    " node(id: $id) { ... on Discussion { labels(first: 100, after: $after) { ",
    page_info!(),
    " nodes { ",
    label!(),
    " } } } } }"
);

const POLL_OPTIONS_QUERY: &str = concat!(
    "query($id: ID!, $after: String) { ",
    rate_limit!(),
    " node(id: $id) { ... on DiscussionPoll { options(first: 100, after: $after) { ",
    page_info!(),
    " nodes { ",
    poll_option!(),
    " } } } } }"
);

const COMMENTS_QUERY: &str = concat!(
    "query($id: ID!, $after: String) { ",
    rate_limit!(),
    " node(id: $id) { ... on Discussion { comments(first: 50, after: $after) { ",
    page_info!(),
    " nodes { ",
    comment!(),
    " replies(first: 50) { totalCount ",
    page_info!(),
    " nodes { ",
    comment!(),
    " } } } } } } }"
);

const REPLIES_QUERY: &str = concat!(
    "query($id: ID!, $after: String) { ",
    rate_limit!(),
    " node(id: $id) { ... on DiscussionComment { replies(first: 100, after: $after) { ",
    page_info!(),
    " nodes { ",
    comment!(),
    " } } } } }"
);

impl GitHubClient {
    /// Lists the discussion categories of a repository, every page.
    ///
    /// # Errors
    ///
    /// [`ClientError::GraphQl`] with `NOT_FOUND` when the repository is not
    /// visible, `FORBIDDEN` when the token may not read it; otherwise the
    /// errors of [`graphql`](Self::graphql) and a malformed answer.
    pub async fn list_discussion_categories(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<Value>, ClientError> {
        let mut nodes = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let vars = json!({"owner": owner, "name": repo, "after": after});
            let mut data = self.query(CATEGORIES_QUERY, vars).await?;
            let page = take_connection(&mut data, &["repository", "discussionCategories"])?;
            nodes.extend(page.nodes);
            match advance(after.as_deref(), page.next)? {
                Some(next) => after = Some(next),
                None => return Ok(nodes),
            }
        }
    }

    /// Returns one page of a repository's discussions, most recently updated
    /// first, starting after the cursor `after`.
    ///
    /// Each discussion's labels and poll options are complete (fetched to
    /// their last page); its comments are only counted
    /// (`comments.totalCount`), see
    /// [`list_discussion_comments`](Self::list_discussion_comments).
    ///
    /// # Errors
    ///
    /// As [`list_discussion_categories`](Self::list_discussion_categories).
    pub async fn list_discussions_page(
        &self,
        owner: &str,
        repo: &str,
        after: Option<&str>,
    ) -> Result<NodePage, ClientError> {
        let vars = json!({"owner": owner, "name": repo, "after": after});
        let mut data = self.query(DISCUSSIONS_QUERY, vars).await?;
        let mut page = take_connection(&mut data, &["repository", "discussions"])?;
        for discussion in &mut page.nodes {
            self.complete(discussion, "labels", LABELS_QUERY).await?;
            if let Some(poll) = discussion.get_mut("poll").filter(|p| !p.is_null()) {
                self.complete(poll, "options", POLL_OPTIONS_QUERY).await?;
            }
        }
        Ok(page)
    }

    /// Lists every comment of the discussion with node id `discussion_id`,
    /// oldest first, each with every one of its replies (`replies.nodes`).
    ///
    /// # Errors
    ///
    /// [`ClientError::GraphQl`] with `NOT_FOUND` when the discussion no
    /// longer exists; otherwise as
    /// [`list_discussion_categories`](Self::list_discussion_categories).
    pub async fn list_discussion_comments(
        &self,
        discussion_id: &str,
    ) -> Result<Vec<Value>, ClientError> {
        let mut comments = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let vars = json!({"id": discussion_id, "after": after});
            let mut data = self.query(COMMENTS_QUERY, vars).await?;
            let page = take_connection(&mut data, &["node", "comments"])?;
            for mut comment in page.nodes {
                self.complete(&mut comment, "replies", REPLIES_QUERY)
                    .await?;
                comments.push(comment);
            }
            match advance(after.as_deref(), page.next)? {
                Some(next) => after = Some(next),
                None => return Ok(comments),
            }
        }
    }

    /// Runs `query` and returns its data without `rateLimit`, which is
    /// logged.
    async fn query(&self, query: &str, variables: Value) -> Result<Value, ClientError> {
        let mut data = self.graphql(query, variables).await?;
        if let Some(limit) = data.as_object_mut().and_then(|d| d.remove("rateLimit")) {
            debug!(
                cost = %limit["cost"],
                remaining = %limit["remaining"],
                reset_at = %limit["resetAt"],
                "GraphQL query cost"
            );
        }
        Ok(data)
    }

    /// Fetches the pages of the connection `field` of `node` that its first
    /// page did not hold (with `query`, which takes the node's `id` and a
    /// cursor) and appends their nodes, so `node[field].nodes` is complete and
    /// its `pageInfo` says there is no next page.
    async fn complete(
        &self,
        node: &mut Value,
        field: &str,
        query: &str,
    ) -> Result<(), ClientError> {
        let Some(mut after) = next_cursor(&node[field])? else {
            return Ok(());
        };
        let id = node["id"]
            .as_str()
            .ok_or_else(|| malformed(&format!("a node with more `{field}` has no `id`")))?
            .to_string();
        let mut extra = Vec::new();
        let last = loop {
            let vars = json!({"id": id, "after": after});
            let mut data = self.query(query, vars).await?;
            let page = take_connection(&mut data, &["node", field])?;
            extra.extend(page.nodes);
            match advance(Some(&after), page.next)? {
                Some(next) => after = next,
                None => break after,
            }
        };
        let connection = &mut node[field];
        if let Some(nodes) = connection["nodes"].as_array_mut() {
            nodes.extend(extra);
        }
        connection["pageInfo"] = json!({"hasNextPage": false, "endCursor": last});
        Ok(())
    }
}

/// Takes the connection at `path` in `data`: its nodes and, if it has a
/// next page, that page's cursor.
fn take_connection(data: &mut Value, path: &[&str]) -> Result<NodePage, ClientError> {
    let mut connection = data;
    for key in path {
        connection = connection
            .get_mut(*key)
            .filter(|v| !v.is_null())
            .ok_or_else(|| malformed(&format!("the answer has no `{}`", path.join("."))))?;
    }
    let next = next_cursor(connection)?;
    let nodes = match connection.get_mut("nodes").map(Value::take) {
        Some(Value::Array(nodes)) => nodes,
        _ => return Err(malformed(&format!("`{}` has no nodes", path.join(".")))),
    };
    Ok(NodePage { nodes, next })
}

/// The cursor of the page after `connection`, `None` when it is the last.
fn next_cursor(connection: &Value) -> Result<Option<String>, ClientError> {
    let info = &connection["pageInfo"];
    if info["hasNextPage"].as_bool() != Some(true) {
        return Ok(None);
    }
    match info["endCursor"].as_str() {
        Some(cursor) => Ok(Some(cursor.to_string())),
        None => Err(malformed("a connection has a next page but no cursor")),
    }
}

/// The cursor to ask for next, refusing one that does not move on (a server
/// that answers the same page again would otherwise be asked forever).
fn advance(previous: Option<&str>, next: Option<String>) -> Result<Option<String>, ClientError> {
    match next {
        Some(next) if previous == Some(next.as_str()) => {
            Err(malformed("a connection returned the same cursor twice"))
        }
        other => Ok(other),
    }
}

fn malformed(what: &str) -> ClientError {
    ClientError::Json(serde::de::Error::custom(format!(
        "unexpected GraphQL answer: {what}"
    )))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use github_backup_types::config::Credential;

    use super::*;

    /// A local GraphQL server: answers the `n`-th request with `answers[n]`
    /// and records each request body.
    async fn serve(answers: Vec<Value>) -> (GitHubClient, Arc<Mutex<Vec<Value>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&bodies);
        tokio::spawn(async move {
            let mut answers = answers.into_iter();
            while let Ok((mut stream, _)) = listener.accept().await {
                let body = read_body(&mut stream).await;
                log.lock().expect("log").push(body);
                let answer = answers.next().unwrap_or(json!({"data": null})).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        let client = GitHubClient::for_tests_at(Credential::Token("ghp_dummy".into()), &base);
        (client, bodies)
    }

    /// Reads one request and returns its JSON body.
    async fn read_body(stream: &mut tokio::net::TcpStream) -> Value {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream.read(&mut chunk).await.expect("read");
            buf.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&buf).to_string();
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let length: usize = head
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse().ok())?
                    })
                    .expect("content-length");
                if body.len() >= length {
                    return serde_json::from_str(&body[..length]).expect("json body");
                }
            }
            assert!(n > 0, "connection closed mid-request");
        }
    }

    fn page(nodes: Value, next: Option<&str>) -> Value {
        json!({"pageInfo": {"hasNextPage": next.is_some(), "endCursor": next}, "nodes": nodes})
    }

    fn data(value: Value) -> Value {
        json!({"data": value})
    }

    /// Two pages of comments; the first comment has replies beyond its first
    /// page.  Every page is asked for with the right cursor and the replies
    /// end up complete inside their comment.
    #[tokio::test]
    async fn comments_and_replies_are_followed_to_their_last_page() {
        let first = json!({"id": "C1", "body": "one",
            "replies": {"totalCount": 2, "pageInfo": {"hasNextPage": true, "endCursor": "r1"},
                        "nodes": [{"id": "R1"}]}});
        let second = json!({"id": "C2", "body": "two",
            "replies": {"totalCount": 0, "pageInfo": {"hasNextPage": false, "endCursor": null},
                        "nodes": []}});
        let (client, bodies) = serve(vec![
            data(
                json!({"rateLimit": {"cost": 1, "remaining": 4999, "resetAt": "x"},
                        "node": {"comments": page(json!([first]), Some("c1"))}}),
            ),
            data(json!({"node": {"replies": page(json!([{"id": "R2"}]), None)}})),
            data(json!({"node": {"comments": page(json!([second]), None)}})),
        ])
        .await;

        let comments = client
            .list_discussion_comments("D1")
            .await
            .expect("comments");

        let ids: Vec<&str> = comments.iter().map(|c| c["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["C1", "C2"]);
        assert_eq!(
            comments[0]["replies"]["nodes"],
            json!([{"id": "R1"}, {"id": "R2"}])
        );
        assert_eq!(comments[0]["replies"]["pageInfo"]["hasNextPage"], false);
        assert_eq!(comments[0]["replies"]["totalCount"], 2);

        let bodies = bodies.lock().unwrap().clone();
        assert_eq!(bodies.len(), 3);
        assert_eq!(bodies[0]["query"], COMMENTS_QUERY);
        assert_eq!(bodies[0]["variables"], json!({"id": "D1", "after": null}));
        assert_eq!(bodies[1]["query"], REPLIES_QUERY);
        assert_eq!(bodies[1]["variables"], json!({"id": "C1", "after": "r1"}));
        assert_eq!(bodies[2]["variables"], json!({"id": "D1", "after": "c1"}));
    }

    #[tokio::test]
    async fn a_discussion_page_has_its_labels_completed_and_its_cursor() {
        let discussion = json!({"id": "D1", "number": 7, "poll": null,
            "labels": {"totalCount": 2, "pageInfo": {"hasNextPage": true, "endCursor": "l1"},
                       "nodes": [{"id": "L1"}]}});
        let (client, bodies) = serve(vec![
            data(json!({"repository": {"discussions": page(json!([discussion]), Some("d1"))}})),
            data(json!({"node": {"labels": page(json!([{"id": "L2"}]), None)}})),
        ])
        .await;

        let page = client
            .list_discussions_page("o", "r", Some("d0"))
            .await
            .expect("page");

        assert_eq!(page.next.as_deref(), Some("d1"));
        assert_eq!(
            page.nodes[0]["labels"]["nodes"],
            json!([{"id": "L1"}, {"id": "L2"}])
        );
        let bodies = bodies.lock().unwrap().clone();
        assert_eq!(bodies[0]["query"], DISCUSSIONS_QUERY);
        assert_eq!(
            bodies[0]["variables"],
            json!({"owner": "o", "name": "r", "after": "d0"})
        );
        assert_eq!(bodies[1]["query"], LABELS_QUERY);
        assert_eq!(bodies[1]["variables"], json!({"id": "D1", "after": "l1"}));
    }

    #[tokio::test]
    async fn categories_are_listed_across_pages() {
        let (client, _) = serve(vec![
            data(json!({"repository": {"discussionCategories": page(json!([{"id": "A"}]), Some("a"))}})),
            data(json!({"repository": {"discussionCategories": page(json!([{"id": "B"}]), None)}})),
        ])
        .await;
        let categories = client
            .list_discussion_categories("o", "r")
            .await
            .expect("ok");
        assert_eq!(categories, vec![json!({"id": "A"}), json!({"id": "B"})]);
    }

    #[tokio::test]
    async fn a_repository_that_cannot_be_resolved_is_a_not_found_error() {
        let (client, _) = serve(vec![json!({
            "data": {"repository": null},
            "errors": [{"type": "NOT_FOUND", "message": "Could not resolve to a Repository"}]
        })])
        .await;
        let err = client
            .list_discussions_page("o", "r", None)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, ClientError::GraphQl { kinds, .. } if kinds == &["NOT_FOUND"]),
            "{err:?}"
        );
    }

    #[test]
    fn a_cursor_that_does_not_move_on_is_refused() {
        assert!(advance(Some("x"), Some("x".into())).is_err());
        assert_eq!(
            advance(Some("x"), Some("y".into())).unwrap().as_deref(),
            Some("y")
        );
        assert_eq!(advance(None, None).unwrap(), None);
    }

    #[test]
    fn a_next_page_without_a_cursor_is_malformed() {
        let mut value =
            json!({"c": {"pageInfo": {"hasNextPage": true, "endCursor": null}, "nodes": []}});
        assert!(take_connection(&mut value, &["c"]).is_err());
        let mut value = json!({"c": null});
        assert!(take_connection(&mut value, &["c"]).is_err());
    }
}
