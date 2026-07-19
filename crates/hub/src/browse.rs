//! Browse/query endpoints: `GET /v1/projects`, `GET /v1/sessions`,
//! `GET /v1/sessions/:id/messages`. All require a valid token and span every
//! machine in the archive, with bounded, stable (id-tiebroken) pagination.

use axum::extract::{Path, Query, State};
use axum::http::HeaderName;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::Authenticated;
use crate::error::HubError;
use crate::pagination::Page;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ListParams {
    /// Machine hostname filter.
    pub machine: Option<String>,
    pub provider: Option<String>,
    /// Project name or path filter (sessions only), or `identity:<key>` for
    /// server-side expansion to the identity's member + aliased paths.
    pub project: Option<String>,
    /// In identity scope: `false` excludes worktree-only member paths.
    pub include_worktrees: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ProjectRow {
    pub id: i64,
    pub provider: String,
    pub project_path: String,
    pub name: Option<String>,
    pub storage_type: Option<String>,
    pub session_count: i32,
    pub message_count: i32,
    pub last_modified: Option<DateTime<Utc>>,
    pub machine_id: Uuid,
    pub machine_hostname: String,
    /// Git-fingerprint identity (NULL = not a git repo → path identity).
    pub identity_key: Option<String>,
    /// Linked `git worktree` member of its identity.
    pub git_worktree: bool,
    /// For worktrees: the main checkout's path.
    pub git_main_path: Option<String>,
}

pub async fn list_projects(
    _auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<ProjectRow>>, HubError> {
    let page = Page::from(params.limit, params.offset);
    let rows = sqlx::query!(
        r#"
        SELECT p.id              AS "id!",
               p.provider        AS "provider!",
               p.project_path    AS "project_path!",
               p.name,
               p.storage_type,
               p.session_count   AS "session_count!",
               p.message_count   AS "message_count!",
               p.last_modified,
               p.machine_id      AS "machine_id!",
               mac.hostname      AS "machine_hostname!",
               p.identity_key,
               p.git_worktree    AS "git_worktree!",
               p.git_main_path
        FROM projects p
        JOIN machines mac ON p.machine_id = mac.machine_id
        WHERE ($1::text IS NULL OR mac.hostname = $1)
          AND ($2::text IS NULL OR p.provider = $2)
        ORDER BY p.last_modified DESC NULLS LAST, p.id DESC
        LIMIT $3 OFFSET $4
        "#,
        params.machine,
        params.provider,
        page.limit,
        page.offset,
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| ProjectRow {
                id: r.id,
                provider: r.provider,
                project_path: r.project_path,
                name: r.name,
                storage_type: r.storage_type,
                session_count: r.session_count,
                message_count: r.message_count,
                last_modified: r.last_modified,
                machine_id: r.machine_id,
                machine_hostname: r.machine_hostname,
                identity_key: r.identity_key,
                git_worktree: r.git_worktree,
                git_main_path: r.git_main_path,
            })
            .collect(),
    ))
}

#[derive(Debug, Serialize)]
pub struct SessionRow {
    pub id: i64,
    pub provider: String,
    pub session_id: String,
    pub summary: Option<String>,
    pub file_path: Option<String>,
    pub entrypoint: Option<String>,
    pub message_count: i32,
    pub first_message_time: Option<DateTime<Utc>>,
    pub last_message_time: Option<DateTime<Utc>>,
    pub has_tool_use: bool,
    pub has_errors: bool,
    pub project_name: Option<String>,
    pub project_path: Option<String>,
    pub machine_hostname: String,
}

pub async fn list_sessions(
    _auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<SessionRow>>, HubError> {
    let page = Page::from(params.limit, params.offset);
    let scope = crate::identity_filter::resolve_project_scope(
        &state.pool,
        params.project.as_deref(),
        params.include_worktrees.unwrap_or(true),
    )
    .await?;
    let rows = sqlx::query!(
        r#"
        SELECT s.id                 AS "id!",
               s.provider           AS "provider!",
               s.session_id         AS "session_id!",
               s.summary,
               s.file_path,
               s.entrypoint,
               s.message_count      AS "message_count!",
               s.first_message_time,
               s.last_message_time,
               s.has_tool_use       AS "has_tool_use!",
               s.has_errors         AS "has_errors!",
               p.name               AS project_name,
               p.project_path     AS "project_path?",
               mac.hostname         AS "machine_hostname!"
        FROM sessions s
        LEFT JOIN projects p ON s.project_id = p.id
        JOIN machines mac    ON s.machine_id = mac.machine_id
        WHERE ($1::text IS NULL OR mac.hostname = $1)
          AND ($2::text IS NULL OR s.provider = $2)
          AND ($3::text IS NULL OR p.name = $3 OR p.project_path = $3)
          AND ($6::text[] IS NULL OR p.project_path = ANY($6))
        ORDER BY s.last_message_time DESC NULLS LAST, s.id DESC
        LIMIT $4 OFFSET $5
        "#,
        params.machine,
        params.provider,
        scope.plain,
        page.limit,
        page.offset,
        scope.paths.as_deref(),
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| SessionRow {
                id: r.id,
                provider: r.provider,
                session_id: r.session_id,
                summary: r.summary,
                file_path: r.file_path,
                entrypoint: r.entrypoint,
                message_count: r.message_count,
                first_message_time: r.first_message_time,
                last_message_time: r.last_message_time,
                has_tool_use: r.has_tool_use,
                has_errors: r.has_errors,
                project_name: r.project_name,
                project_path: r.project_path,
                machine_hostname: r.machine_hostname,
            })
            .collect(),
    ))
}

#[derive(Debug, Deserialize)]
pub struct PageParams {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct MessageRow {
    pub id: i64,
    pub message_key: String,
    pub uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub seq: i32,
    pub timestamp: Option<DateTime<Utc>>,
    pub message_type: Option<String>,
    pub role: Option<String>,
    pub model: Option<String>,
    pub stop_reason: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<i64>,
    pub is_sidechain: bool,
    pub content: Option<serde_json::Value>,
}

/// Resolve a `:id` path segment to a session surrogate id: numeric values are
/// taken as the surrogate id itself; anything else is looked up as a provider
/// session id (the UUID carried by search hits and session rows). A provider
/// session id can repeat across machines, so an ambiguous match is a 400
/// listing the candidate surrogate ids.
async fn resolve_session_ref(state: &AppState, session_ref: &str) -> Result<i64, HubError> {
    if let Ok(pk) = session_ref.parse::<i64>() {
        return Ok(pk);
    }
    let ids = sqlx::query_scalar!(
        r#"SELECT id AS "id!" FROM sessions WHERE session_id = $1 ORDER BY id"#,
        session_ref,
    )
    .fetch_all(&state.pool)
    .await?;
    match ids.as_slice() {
        [] => Err(HubError::NotFound(format!(
            "no session with id {session_ref}"
        ))),
        [pk] => Ok(*pk),
        many => Err(HubError::BadRequest(format!(
            "session id {session_ref} is ambiguous across machines; use a numeric session id from /v1/sessions (candidates: {})",
            many.iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// `:id` is the hub's surrogate session id (from the sessions list) or, as a
/// convenience, an unambiguous provider session id (see [`resolve_session_ref`]).
/// The response body is the page of messages; `X-Total-Count` carries the
/// session's total message count so clients can detect a truncated page.
pub async fn session_messages(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(session_ref): Path<String>,
    Query(page): Query<PageParams>,
) -> Result<([(HeaderName, String); 1], Json<Vec<MessageRow>>), HubError> {
    let session_pk = resolve_session_ref(&state, &session_ref).await?;
    let page = Page::from(page.limit, page.offset);
    // Read the maintained aggregate rather than `COUNT(*)`-ing the messages: a
    // count over `session_id` range-scans `messages_session_id_message_key_key`
    // (that unique index doubles as the only index on `session_id`), so every
    // page view of a 3,000-message session read 3,000 index tuples for a number
    // ingest already recomputes on that row, inside the same transaction that
    // wrote the messages. Same value, one row read.
    let total = i64::from(
        sqlx::query_scalar!(
            r#"SELECT message_count AS "count!" FROM sessions WHERE id = $1"#,
            session_pk,
        )
        .fetch_one(&state.pool)
        .await?,
    );
    let rows = sqlx::query!(
        r#"
        SELECT m.id            AS "id!",
               m.message_key   AS "message_key!",
               m.uuid,
               m.parent_uuid,
               m.seq           AS "seq!",
               m."timestamp",
               m.type          AS message_type,
               m.role,
               m.model,
               m.stop_reason,
               m.input_tokens,
               m.output_tokens,
               m.cost_usd,
               m.duration_ms,
               m.is_sidechain  AS "is_sidechain!",
               m.content
        FROM messages m
        WHERE m.session_id = $1
        ORDER BY m."timestamp" ASC NULLS LAST, m.seq ASC, m.id ASC
        LIMIT $2 OFFSET $3
        "#,
        session_pk,
        page.limit,
        page.offset,
    )
    .fetch_all(&state.pool)
    .await?;

    Ok((
        [(HeaderName::from_static("x-total-count"), total.to_string())],
        Json(
            rows.into_iter()
                .map(|r| MessageRow {
                    id: r.id,
                    message_key: r.message_key,
                    uuid: r.uuid,
                    parent_uuid: r.parent_uuid,
                    seq: r.seq,
                    timestamp: r.timestamp,
                    message_type: r.message_type,
                    role: r.role,
                    model: r.model,
                    stop_reason: r.stop_reason,
                    input_tokens: r.input_tokens,
                    output_tokens: r.output_tokens,
                    cost_usd: r.cost_usd,
                    duration_ms: r.duration_ms,
                    is_sidechain: r.is_sidechain,
                    content: r.content,
                })
                .collect(),
        ),
    ))
}
