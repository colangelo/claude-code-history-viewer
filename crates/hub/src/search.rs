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
    /// Project name or path filter, or `identity:<key>` for server-side
    /// expansion to the identity's member + aliased paths.
    pub project: Option<String>,
    /// In identity scope: `false` excludes worktree-only member paths.
    pub include_worktrees: Option<bool>,
    /// RFC 3339 lower/upper timestamp bounds.
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    /// `all` (default) | `messages` | `journal`. Controls whether the additive
    /// `journal` block is included and whether the message search runs.
    pub scope: Option<String>,
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
    /// 0-based index of this message in its session's browse ordering
    /// (`timestamp ASC NULLS LAST, seq ASC, id ASC`) — lets a client open the
    /// page containing the hit instead of page 1 (issue #20).
    pub position: i64,
}

#[derive(Debug, Serialize)]
pub struct SearchResults {
    pub results: Vec<SearchHit>,
    pub limit: i64,
    pub offset: i64,
    /// Additive journal block. Absent entirely at `scope=messages` (so that
    /// scope is byte-compatible with the pre-journal response shape); present
    /// at `scope=all` (default) and `scope=journal`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journal: Option<Vec<crate::journal::JournalHit>>,
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

    let scope = params.scope.as_deref().unwrap_or("all");
    let (want_messages, want_journal) = match scope {
        "all" => (true, true),
        "messages" => (true, false),
        "journal" => (false, true),
        other => {
            return Err(HubError::BadRequest(format!(
                "unknown scope `{other}` (expected `all`, `messages`, or `journal`)"
            )))
        }
    };

    // Resolve the project filter once (plain vs `identity:<key>` expansion)
    // and share it between the message and journal legs.
    let project_scope = crate::identity_filter::resolve_project_scope(
        &state.pool,
        params.project.as_deref(),
        params.include_worktrees.unwrap_or(true),
    )
    .await?;

    // Message hits: unchanged shape and ordering at every scope. Skipped
    // entirely for `scope=journal` (which performs no message search).
    let results: Vec<SearchHit> = if want_messages {
        message_hits(&state, &params, &project_scope, from, to, page).await?
    } else {
        Vec::new()
    };

    // Additive journal block: present for `all` (default) and `journal`, absent
    // for `messages`.
    let journal = if want_journal {
        Some(
            crate::journal::search_journal(
                &state,
                &params.q,
                &project_scope,
                page.limit,
                page.offset,
            )
            .await?,
        )
    } else {
        None
    };

    Ok(Json(SearchResults {
        results,
        limit: page.limit,
        offset: page.offset,
        journal,
    }))
}

/// The message full-text search. The `q` CTE combines the websearch parse
/// with an optional prefix variant (issue #19, `fts::prefix_tsquery`) —
/// OR-ing only ever ADDS hits, and the variant is absent for advanced-syntax
/// queries so phrases/negation keep exact semantics. `position` locates the
/// hit in its session's browse ordering (issue #20).
async fn message_hits(
    state: &AppState,
    params: &SearchParams,
    project_scope: &crate::identity_filter::ProjectScope,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    page: Page,
) -> Result<Vec<SearchHit>, HubError> {
    let prefix = crate::fts::prefix_tsquery(&params.q);
    let rows = sqlx::query!(
        r#"
        WITH q AS (
            SELECT CASE
                WHEN $10::text IS NULL THEN websearch_to_tsquery('simple', $1)
                ELSE websearch_to_tsquery('simple', $1) || to_tsquery('simple', $10)
            END AS tsq
        )
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
                        q.tsq,
                        'MaxFragments=2, MinWords=3, MaxWords=14') AS "snippet!",
            ts_rank(m.text_search, q.tsq) AS "rank!",
            (
                SELECT count(*) FROM messages m2
                WHERE m2.session_id = m.session_id
                  AND (
                    CASE WHEN m."timestamp" IS NULL
                        THEN m2."timestamp" IS NOT NULL
                             OR (m2."timestamp" IS NULL
                                 AND (m2.seq, m2.id) < (m.seq, m.id))
                        ELSE m2."timestamp" IS NOT NULL
                             AND (m2."timestamp", m2.seq, m2.id)
                                 < (m."timestamp", m.seq, m.id)
                    END
                  )
            ) AS "position!"
        FROM q, messages m
        JOIN sessions s   ON m.session_id = s.id
        LEFT JOIN projects p ON s.project_id = p.id
        JOIN machines mac ON m.machine_id = mac.machine_id
        WHERE m.text_search @@ q.tsq
          AND ($2::text IS NULL OR m.provider = $2)
          AND ($3::text IS NULL OR mac.hostname = $3)
          AND ($4::text IS NULL OR p.name = $4 OR p.project_path = $4)
          AND ($9::text[] IS NULL OR p.project_path = ANY($9))
          AND ($5::timestamptz IS NULL OR m."timestamp" >= $5)
          AND ($6::timestamptz IS NULL OR m."timestamp" <= $6)
        ORDER BY "rank!" DESC, m."timestamp" DESC NULLS LAST, m.id DESC
        LIMIT $7 OFFSET $8
        "#,
        params.q,
        params.provider,
        params.machine,
        project_scope.plain,
        from,
        to,
        page.limit,
        page.offset,
        project_scope.paths.as_deref(),
        prefix,
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(rows
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
            position: r.position,
        })
        .collect())
}
