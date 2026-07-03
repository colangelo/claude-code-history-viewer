//! Browse/query endpoints: `GET /v1/projects`, `GET /v1/sessions`,
//! `GET /v1/sessions/:id/messages`. All require a valid token and span every
//! machine in the archive, with bounded, stable (id-tiebroken) pagination.

use axum::extract::{Path, Query, State};
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
    /// Project name or path filter (sessions only).
    pub project: Option<String>,
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
               mac.hostname      AS "machine_hostname!"
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
        ORDER BY s.last_message_time DESC NULLS LAST, s.id DESC
        LIMIT $4 OFFSET $5
        "#,
        params.machine,
        params.provider,
        params.project,
        page.limit,
        page.offset,
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

/// `:id` is the hub's surrogate session id (from the sessions list), which is
/// globally unique — unlike a provider session id, which can repeat per machine.
pub async fn session_messages(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(session_pk): Path<i64>,
    Query(page): Query<PageParams>,
) -> Result<Json<Vec<MessageRow>>, HubError> {
    let page = Page::from(page.limit, page.offset);
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
        ORDER BY m.seq ASC, m."timestamp" ASC NULLS LAST, m.id ASC
        LIMIT $2 OFFSET $3
        "#,
        session_pk,
        page.limit,
        page.offset,
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(
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
    ))
}
