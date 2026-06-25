//! `GET /v1/search` — full-text search across the archive.

use axum::extract::{Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::auth::Authenticated;
use crate::error::HubError;
use crate::pagination::Page;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    /// Free-text query (websearch syntax).
    pub q: String,
    pub provider: Option<String>,
    /// Machine hostname filter.
    pub machine: Option<String>,
    /// Project name or path filter.
    pub project: Option<String>,
    /// RFC 3339 lower/upper timestamp bounds.
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// One search hit with its session + project + machine context.
#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub message_id: i64,
    pub message_key: String,
    pub uuid: Option<String>,
    pub provider: String,
    pub session_pk: i64,
    pub session_id: String,
    pub session_summary: Option<String>,
    pub project_name: Option<String>,
    pub project_path: Option<String>,
    pub machine_hostname: String,
    pub timestamp: Option<DateTime<Utc>>,
    pub message_type: Option<String>,
    pub role: Option<String>,
    pub model: Option<String>,
    pub snippet: String,
    pub rank: f32,
}

#[derive(Debug, Serialize)]
pub struct SearchResults {
    pub results: Vec<SearchHit>,
    pub limit: i64,
    pub offset: i64,
}

fn parse_bound(s: Option<&str>, which: &str) -> Result<Option<DateTime<Utc>>, HubError> {
    match s {
        None => Ok(None),
        Some(v) => DateTime::parse_from_rfc3339(v)
            .map(|dt| Some(dt.with_timezone(&Utc)))
            .map_err(|_| {
                HubError::BadRequest(format!("invalid `{which}` timestamp (need RFC 3339)"))
            }),
    }
}

pub async fn search(
    _auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<Json<SearchResults>, HubError> {
    if params.q.trim().is_empty() {
        return Err(HubError::BadRequest("`q` must not be empty".into()));
    }
    let page = Page::from(params.limit, params.offset);
    let from = parse_bound(params.from.as_deref(), "from")?;
    let to = parse_bound(params.to.as_deref(), "to")?;

    let rows = sqlx::query!(
        r#"
        SELECT
            m.id              AS "message_id!",
            m.message_key     AS "message_key!",
            m.uuid,
            m.provider        AS "provider!",
            m.session_id      AS "session_pk!",
            s.session_id      AS "session_id!",
            s.summary         AS session_summary,
            p.name            AS project_name,
            p.project_path     AS "project_path?",
            mac.hostname      AS "machine_hostname!",
            m."timestamp",
            m.type            AS message_type,
            m.role,
            m.model,
            ts_headline('simple', coalesce(m.search_text, ''),
                        websearch_to_tsquery('simple', $1),
                        'MaxFragments=2, MinWords=3, MaxWords=14') AS "snippet!",
            ts_rank(m.text_search, websearch_to_tsquery('simple', $1)) AS "rank!"
        FROM messages m
        JOIN sessions s   ON m.session_id = s.id
        LEFT JOIN projects p ON s.project_id = p.id
        JOIN machines mac ON m.machine_id = mac.machine_id
        WHERE m.text_search @@ websearch_to_tsquery('simple', $1)
          AND ($2::text IS NULL OR m.provider = $2)
          AND ($3::text IS NULL OR mac.hostname = $3)
          AND ($4::text IS NULL OR p.name = $4 OR p.project_path = $4)
          AND ($5::timestamptz IS NULL OR m."timestamp" >= $5)
          AND ($6::timestamptz IS NULL OR m."timestamp" <= $6)
        ORDER BY "rank!" DESC, m."timestamp" DESC NULLS LAST, m.id DESC
        LIMIT $7 OFFSET $8
        "#,
        params.q,
        params.provider,
        params.machine,
        params.project,
        from,
        to,
        page.limit,
        page.offset,
    )
    .fetch_all(&state.pool)
    .await?;

    let results = rows
        .into_iter()
        .map(|r| SearchHit {
            message_id: r.message_id,
            message_key: r.message_key,
            uuid: r.uuid,
            provider: r.provider,
            session_pk: r.session_pk,
            session_id: r.session_id,
            session_summary: r.session_summary,
            project_name: r.project_name,
            project_path: r.project_path,
            machine_hostname: r.machine_hostname,
            timestamp: r.timestamp,
            message_type: r.message_type,
            role: r.role,
            model: r.model,
            snippet: r.snippet,
            rank: r.rank,
        })
        .collect();

    Ok(Json(SearchResults {
        results,
        limit: page.limit,
        offset: page.offset,
    }))
}
