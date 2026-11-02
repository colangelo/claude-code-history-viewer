//! Browse/query endpoints: `GET /v1/projects`, `GET /v1/sessions`,
//! `GET /v1/sessions/:id/messages`. All require a valid token and span every
//! machine in the archive, with bounded, stable (id-tiebroken) pagination.
//! Every response carries the filtered total in `X-Total-Count` (the limit is
//! silently clamped to [`crate::pagination::MAX_LIMIT`], so without the total
//! a capped page is indistinguishable from the whole result set), and unknown
//! query params are a 400, never ignored — an unsupported filter that returns
//! the unfiltered list reads exactly like a real answer.

use axum::extract::{Path, Query, State};
use axum::http::HeaderName;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::auth::{Authenticated, Visibility};
use crate::error::HubError;
use crate::pagination::Page;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectListParams {
    /// Machine hostname filter.
    pub machine: Option<String>,
    pub provider: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionListParams {
    /// Machine hostname filter.
    pub machine: Option<String>,
    pub provider: Option<String>,
    /// Project name or path filter, or `identity:<key>` for server-side
    /// expansion to the identity's member + aliased paths.
    pub project: Option<String>,
    /// Hub project surrogate id (a `/v1/projects` row id). Unlike `project`,
    /// which can span machines, this scopes to that one (machine, path) row.
    pub project_id: Option<i64>,
    /// In identity scope: `false` excludes worktree-only member paths.
    pub include_worktrees: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
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

/// A lab reader sees a project only while it holds at least one visible
/// session, and its counts are over the visible sessions (design R2-K): an
/// empty project could be a client folder, so it is hidden too.
pub async fn list_projects(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<ProjectListParams>,
) -> Result<([(HeaderName, String); 1], Json<Vec<ProjectRow>>), HubError> {
    let vis = auth.visibility(&state);
    let page = Page::from(params.limit, params.offset);
    let (seen, sessions, messages) = project_visibility(&vis);
    let total = sqlx::query_scalar::<_, i64>(&format!(
        r"
        SELECT COUNT(*)
        FROM projects p
        JOIN machines mac ON p.machine_id = mac.machine_id
        WHERE ($1::text IS NULL OR mac.hostname = $1)
          AND ($2::text IS NULL OR p.provider = $2)
          AND {seen}
        "
    ))
    .bind(&params.machine)
    .bind(&params.provider)
    .fetch_one(&state.pool)
    .await?;
    let rows = sqlx::query_as::<_, ProjectRow>(&format!(
        r"
        SELECT p.id, p.provider, p.project_path, p.name, p.storage_type,
               {sessions} AS session_count,
               {messages} AS message_count,
               p.last_modified, p.machine_id,
               mac.hostname AS machine_hostname,
               p.identity_key, p.git_worktree, p.git_main_path
        FROM projects p
        JOIN machines mac ON p.machine_id = mac.machine_id
        WHERE ($1::text IS NULL OR mac.hostname = $1)
          AND ($2::text IS NULL OR p.provider = $2)
          AND {seen}
        ORDER BY p.last_modified DESC NULLS LAST, p.id DESC
        LIMIT $3 OFFSET $4
        "
    ))
    .bind(&params.machine)
    .bind(&params.provider)
    .bind(page.limit)
    .bind(page.offset)
    .fetch_all(&state.pool)
    .await?;
    Ok((
        [(HeaderName::from_static("x-total-count"), total.to_string())],
        Json(rows),
    ))
}

/// For a project row `p`: (is it visible, its session count, its message count),
/// as SQL. With `All` the stored aggregates, else counts over visible sessions.
fn project_visibility(vis: &Visibility) -> (String, String, String) {
    if vis.sees_all() {
        return (
            "true".into(),
            "p.session_count".into(),
            "p.message_count".into(),
        );
    }
    let ok = vis.session_ok("vs");
    (
        format!("EXISTS (SELECT 1 FROM sessions vs WHERE vs.project_id = p.id AND {ok})"),
        format!("(SELECT count(*)::int FROM sessions vs WHERE vs.project_id = p.id AND {ok})"),
        format!(
            "(SELECT COALESCE(sum(vs.message_count), 0)::int FROM sessions vs WHERE vs.project_id = p.id AND {ok})"
        ),
    )
}

#[derive(Debug, Serialize, FromRow)]
pub struct SessionRow {
    pub id: i64,
    pub provider: String,
    pub session_id: String,
    pub summary: Option<String>,
    pub file_path: Option<String>,
    pub entrypoint: Option<String>,
    /// The session's stored counter — not a count of stored rows, and it can exceed
    /// them (#47). Not a count of conversation turns either; see `conversation_count`.
    pub message_count: i32,
    /// Rows of this session with `content IS NOT NULL`: the conversation items.
    /// Additive beside `message_count` (#41); computed per returned row.
    pub conversation_count: i64,
    pub first_message_time: Option<DateTime<Utc>>,
    pub last_message_time: Option<DateTime<Utc>>,
    pub has_tool_use: bool,
    pub has_errors: bool,
    pub project_name: Option<String>,
    pub project_path: Option<String>,
    pub machine_hostname: String,
}

pub async fn list_sessions(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<SessionListParams>,
) -> Result<([(HeaderName, String); 1], Json<Vec<SessionRow>>), HubError> {
    let vis = auth.visibility(&state).session_ok("s");
    let page = Page::from(params.limit, params.offset);
    let scope = crate::identity_filter::resolve_project_scope(
        &state.pool,
        params.project.as_deref(),
        params.include_worktrees.unwrap_or(true),
    )
    .await?;
    let filter = format!(
        r"
        FROM sessions s
        LEFT JOIN projects p ON s.project_id = p.id
        JOIN machines mac    ON s.machine_id = mac.machine_id
        WHERE ($1::text IS NULL OR mac.hostname = $1)
          AND ($2::text IS NULL OR s.provider = $2)
          AND ($3::text IS NULL OR p.name = $3 OR p.project_path = $3)
          AND ($4::text[] IS NULL OR p.project_path = ANY($4))
          AND ($5::bigint IS NULL OR s.project_id = $5)
          AND {vis}
        "
    );
    let total = sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) {filter}"))
        .bind(&params.machine)
        .bind(&params.provider)
        .bind(&scope.plain)
        .bind(scope.paths.as_deref())
        .bind(params.project_id)
        .fetch_one(&state.pool)
        .await?;
    let rows = sqlx::query_as::<_, SessionRow>(&format!(
        r"
        SELECT s.id, s.provider, s.session_id, s.summary, s.file_path, s.entrypoint,
               s.message_count,
               (SELECT count(*) FROM messages m
                 WHERE m.session_id = s.id AND m.content IS NOT NULL) AS conversation_count,
               s.first_message_time, s.last_message_time, s.has_tool_use, s.has_errors,
               p.name AS project_name, p.project_path,
               mac.hostname AS machine_hostname
        {filter}
        ORDER BY s.last_message_time DESC NULLS LAST, s.id DESC
        LIMIT $6 OFFSET $7
        "
    ))
    .bind(&params.machine)
    .bind(&params.provider)
    .bind(&scope.plain)
    .bind(scope.paths.as_deref())
    .bind(params.project_id)
    .bind(page.limit)
    .bind(page.offset)
    .fetch_all(&state.pool)
    .await?;
    Ok((
        [(HeaderName::from_static("x-total-count"), total.to_string())],
        Json(rows),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageParams {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    /// RFC 3339 bounds on the half-open window `[from, to)`. Either may stand
    /// alone. Added so the journal distiller can read one logical day of a
    /// session instead of the whole session (#35).
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Serialize, FromRow)]
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
    /// The working folder from the stored record (`raw->>'cwd'`, #84 M1).
    /// Serialized as `null` when absent, never left out: the journal distiller
    /// reads a missing key as "this hub cannot say" and protects the group,
    /// while `null` is "no signal" for that message.
    pub cwd: Option<String>,
}

/// Resolve a `:id` path segment to a session surrogate id: numeric values are
/// taken as the surrogate id itself; anything else is looked up as a provider
/// session id (the UUID carried by search hits and session rows). A provider
/// session id can repeat across machines, so an ambiguous match is a 400
/// listing the candidate surrogate ids.
async fn resolve_session_ref(
    state: &AppState,
    vis: &Visibility,
    session_ref: &str,
) -> Result<i64, HubError> {
    // Visibility before resolution (design R2-G): a hidden session is a
    // missing one, for a numeric id and a provider id alike, and an ambiguity
    // 400 lists only visible candidates.
    let not_found = || HubError::NotFound(format!("no session with id {session_ref}"));
    let ok = vis.session_ok("s");
    let ids: Vec<i64> = if let Ok(pk) = session_ref.parse::<i64>() {
        sqlx::query_scalar(&format!(
            "SELECT s.id FROM sessions s WHERE s.id = $1 AND {ok}"
        ))
        .bind(pk)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_scalar(&format!(
            "SELECT s.id FROM sessions s WHERE s.session_id = $1 AND {ok} ORDER BY s.id"
        ))
        .bind(session_ref)
        .fetch_all(&state.pool)
        .await?
    };
    match ids.as_slice() {
        [] => Err(not_found()),
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
/// The response body is the page of messages; `X-Total-Count` carries the total
/// so clients can detect a truncated page.
///
/// `from` / `to` bound the half-open window `[from, to)`; either may stand alone.
/// When one is given, `X-Total-Count` is the count **within the window** — a
/// header reporting the session total next to a filtered body would make paging
/// wrong, which is the one thing that header exists to prevent.
///
/// The two statements below are runtime queries, not `query!`: the gate builds
/// with `SQLX_OFFLINE=true` and no sqlx-cli, so a new compile-time-checked
/// statement has no way to get its `.sqlx` metadata generated. Same reason every
/// query in `journal.rs` is runtime.
pub async fn session_messages(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(session_ref): Path<String>,
    Query(params): Query<PageParams>,
) -> Result<([(HeaderName, String); 1], Json<Vec<MessageRow>>), HubError> {
    let from = crate::search::parse_bound(params.from.as_deref(), "from")?;
    let to = crate::search::parse_bound(params.to.as_deref(), "to")?;
    let session_pk = resolve_session_ref(&state, &auth.visibility(&state), &session_ref).await?;
    let page = Page::from(params.limit, params.offset);
    // Unwindowed, read the maintained aggregate rather than `COUNT(*)`-ing the
    // messages: a count over `session_id` range-scans
    // `messages_session_id_message_key_key` (that unique index doubles as the
    // only index on `session_id`), so every page view of a 3,000-message session
    // read 3,000 index tuples for a number ingest already recomputes on that row,
    // inside the same transaction that wrote the messages. Same value, one row
    // read.
    //
    // Windowed, that aggregate is the wrong number and there is nothing to
    // reuse — but `messages_session_timestamp_idx` (migration 0006) makes the
    // count a range scan over just the window.
    let total = if from.is_some() || to.is_some() {
        sqlx::query_scalar::<_, i64>(
            r#"
            SELECT count(*)
            FROM messages
            WHERE session_id = $1
              AND ($2::timestamptz IS NULL OR "timestamp" >= $2)
              AND ($3::timestamptz IS NULL OR "timestamp" <  $3)
            "#,
        )
        .bind(session_pk)
        .bind(from)
        .bind(to)
        .fetch_one(&state.pool)
        .await?
    } else {
        i64::from(
            sqlx::query_scalar::<_, i32>("SELECT message_count FROM sessions WHERE id = $1")
                .bind(session_pk)
                .fetch_one(&state.pool)
                .await?,
        )
    };
    let rows = sqlx::query_as::<_, MessageRow>(
        r#"
        SELECT m.id,
               m.message_key,
               m.uuid,
               m.parent_uuid,
               m.seq,
               m."timestamp",
               m.type          AS message_type,
               m.role,
               m.model,
               m.stop_reason,
               m.input_tokens,
               m.output_tokens,
               m.cost_usd,
               m.duration_ms,
               m.is_sidechain,
               m.content,
               m.raw->>'cwd'   AS cwd
        FROM messages m
        WHERE m.session_id = $1
          AND ($4::timestamptz IS NULL OR m."timestamp" >= $4)
          AND ($5::timestamptz IS NULL OR m."timestamp" <  $5)
        ORDER BY m."timestamp" ASC NULLS LAST, m.seq ASC, m.id ASC
        LIMIT $2 OFFSET $3
        "#,
    )
    .bind(session_pk)
    .bind(page.limit)
    .bind(page.offset)
    .bind(from)
    .bind(to)
    .fetch_all(&state.pool)
    .await?;

    Ok((
        [(HeaderName::from_static("x-total-count"), total.to_string())],
        Json(rows),
    ))
}
