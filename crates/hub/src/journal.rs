//! Journal entries: the distilled per-`(entry_date, project_path)` retrieval
//! units (issue #12).
//!
//! * `GET  /v1/journal/pending`  (read-auth) — data-derived work list of
//!   `(entry_date, project_path)` groups needing distillation.
//! * `POST /v1/journal/entries`  (machine-token) — validated upsert by group key.
//! * `GET  /v1/journal/entries`  (read-auth) — browse `entry`-status rows.
//!
//! The `/v1/search` journal block ([`search_journal`]) lives here too, next to
//! the schema knowledge, and is called from [`crate::search`].
//!
//! All queries here are **runtime-checked** (`sqlx::query*` functions, not the
//! `query!` macros): the gate builds with `SQLX_OFFLINE=true` and no sqlx-cli,
//! so new compile-time-checked queries could not get their `.sqlx` metadata
//! regenerated. Runtime queries need no metadata.

use axum::extract::{Query, State};
use axum::Json;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::auth::{AuthedMachine, Authenticated};
use crate::error::HubError;
use crate::pagination::Page;
use crate::state::AppState;

/// Hour at which a logical day starts, applied in UTC. Sessions whose first
/// message falls before this hour count toward the previous calendar day, so
/// late-night work lands in the day it belongs to. Fixed default (04:00 UTC);
/// nothing existing needs reconfiguring to get it.
const DAY_START_HOUR: i32 = 4;

/// Number of `topics` an `entry`-status row must carry (inclusive range).
const MIN_TOPICS: usize = 3;
const MAX_TOPICS: usize = 8;

// ---------------------------------------------------------------------------
// GET /v1/journal/pending
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PendingParams {
    /// Inclusive lower bound on `entry_date` (`YYYY-MM-DD`).
    pub from: Option<String>,
    pub limit: Option<i64>,
}

/// One pending `(entry_date, project_path)` group, carrying the surrogate
/// session ids the distiller needs (so it takes no second lookup).
#[derive(Debug, Serialize, FromRow)]
pub struct PendingGroup {
    pub entry_date: NaiveDate,
    pub project_path: String,
    pub session_ids: Vec<i64>,
}

fn parse_date(s: Option<&str>, which: &str) -> Result<Option<NaiveDate>, HubError> {
    match s {
        None => Ok(None),
        Some(v) => NaiveDate::parse_from_str(v, "%Y-%m-%d")
            .map(Some)
            .map_err(|_| HubError::BadRequest(format!("invalid `{which}` date (need YYYY-MM-DD)"))),
    }
}

/// Data-derived work list. A group is pending when it has archived sessions but
/// no journal row, **or** when a session for it was ingested after the row's
/// `generated_at` (dirty — compared against `sessions.updated_at`, which is
/// bumped at ingest time even for old, late-arriving messages). Groups on the
/// still-open logical day are excluded. Newest-first, honoring `from` + `limit`.
pub async fn pending(
    _auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<PendingParams>,
) -> Result<Json<Vec<PendingGroup>>, HubError> {
    let from = parse_date(params.from.as_deref(), "from")?;
    let page = Page::from(params.limit, None);

    let rows = sqlx::query_as::<_, PendingGroup>(
        r"
        WITH grp AS (
            SELECT
                ((s.first_message_time - make_interval(hours => $1::int))
                    AT TIME ZONE 'UTC')::date        AS entry_date,
                p.project_path                       AS project_path,
                array_agg(s.id ORDER BY s.id)        AS session_ids,
                max(s.updated_at)                    AS latest_ingest
            FROM sessions s
            JOIN projects p ON s.project_id = p.id
            WHERE s.first_message_time IS NOT NULL
            GROUP BY 1, 2
        )
        SELECT g.entry_date, g.project_path, g.session_ids
        FROM grp g
        LEFT JOIN journal_entries j
            ON j.entry_date = g.entry_date
           AND j.project_path = g.project_path
        WHERE g.entry_date
                < ((now() - make_interval(hours => $1::int)) AT TIME ZONE 'UTC')::date
          AND ($2::date IS NULL OR g.entry_date >= $2)
          AND (j.id IS NULL OR g.latest_ingest > j.generated_at)
        ORDER BY g.entry_date DESC, g.project_path DESC
        LIMIT $3
        ",
    )
    .bind(DAY_START_HOUR)
    .bind(from)
    .bind(page.limit)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(rows))
}

// ---------------------------------------------------------------------------
// POST /v1/journal/entries
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct EntryPayload {
    pub entry_date: NaiveDate,
    pub project_path: String,
    /// `"entry"` or `"skip"`.
    pub status: String,
    #[serde(default)]
    pub headline: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default)]
    pub session_ids: Vec<i64>,
    #[serde(default)]
    pub model: Option<String>,
}

/// Build the flattened FTS text for an entry from its headline, summary,
/// topics, and open questions.
fn entry_search_text(p: &EntryPayload) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if let Some(h) = &p.headline {
        parts.push(h);
    }
    if let Some(s) = &p.summary {
        parts.push(s);
    }
    parts.extend(p.topics.iter().map(String::as_str));
    parts.extend(p.open_questions.iter().map(String::as_str));
    parts.join(" ")
}

/// Validated upsert by `(entry_date, project_path)`. Machine-token auth (same
/// model as ingest). Invalid payloads are rejected with a `4xx` and no write.
pub async fn create(
    _machine: AuthedMachine,
    State(state): State<AppState>,
    Json(payload): Json<EntryPayload>,
) -> Result<Json<serde_json::Value>, HubError> {
    // -- validate ----------------------------------------------------------
    let is_skip = match payload.status.as_str() {
        "entry" => false,
        "skip" => true,
        other => {
            return Err(HubError::BadRequest(format!(
                "unknown status `{other}` (expected `entry` or `skip`)"
            )))
        }
    };

    if !is_skip {
        let headline = payload.headline.as_deref().unwrap_or("").trim();
        if headline.is_empty() {
            return Err(HubError::BadRequest(
                "entry status requires a headline".into(),
            ));
        }
        let summary = payload.summary.as_deref().unwrap_or("").trim();
        if summary.is_empty() {
            return Err(HubError::BadRequest(
                "entry status requires a summary".into(),
            ));
        }
        let n = payload.topics.len();
        if !(MIN_TOPICS..=MAX_TOPICS).contains(&n) {
            return Err(HubError::BadRequest(format!(
                "entry status requires between {MIN_TOPICS} and {MAX_TOPICS} topics (got {n})"
            )));
        }
    }

    // Referenced session ids must all exist (for either status).
    if !payload.session_ids.is_empty() {
        let mut ids = payload.session_ids.clone();
        ids.sort_unstable();
        ids.dedup();
        let found: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE id = ANY($1)")
            .bind(&ids)
            .fetch_one(&state.pool)
            .await?;
        if found != i64::try_from(ids.len()).unwrap_or(i64::MAX) {
            return Err(HubError::BadRequest(
                "one or more referenced session ids do not exist".into(),
            ));
        }
    }

    // -- upsert ------------------------------------------------------------
    // Skip rows carry no content and no FTS text (they never surface in browse
    // or search); entries carry the flattened search_text.
    let (headline, summary, search_text) = if is_skip {
        (None, None, None)
    } else {
        let st = entry_search_text(&payload);
        (payload.headline.clone(), payload.summary.clone(), Some(st))
    };
    let topics: Vec<String> = if is_skip {
        Vec::new()
    } else {
        payload.topics.clone()
    };
    let open_questions: Vec<String> = if is_skip {
        Vec::new()
    } else {
        payload.open_questions.clone()
    };

    sqlx::query(
        r"
        INSERT INTO journal_entries
            (entry_date, project_path, status, headline, summary, topics,
             open_questions, session_ids, model, generated_at, search_text)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now(), $10)
        ON CONFLICT (entry_date, project_path)
        DO UPDATE SET status         = excluded.status,
                      headline        = excluded.headline,
                      summary         = excluded.summary,
                      topics          = excluded.topics,
                      open_questions  = excluded.open_questions,
                      session_ids     = excluded.session_ids,
                      model           = excluded.model,
                      generated_at    = now(),
                      search_text     = excluded.search_text
        ",
    )
    .bind(payload.entry_date)
    .bind(&payload.project_path)
    .bind(&payload.status)
    .bind(headline)
    .bind(summary)
    .bind(&topics)
    .bind(&open_questions)
    .bind(&payload.session_ids)
    .bind(&payload.model)
    .bind(search_text)
    .execute(&state.pool)
    .await?;

    Ok(Json(serde_json::json!({ "status": "ok" })))
}

// ---------------------------------------------------------------------------
// GET /v1/journal/entries (browse)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BrowseParams {
    /// Match `project_path`.
    pub project: Option<String>,
    /// Inclusive `entry_date` bounds (`YYYY-MM-DD`).
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// A full `entry`-status journal row.
#[derive(Debug, Serialize, FromRow)]
pub struct JournalEntry {
    pub entry_date: NaiveDate,
    pub project_path: String,
    pub status: String,
    pub headline: Option<String>,
    pub summary: Option<String>,
    pub topics: Vec<String>,
    pub open_questions: Vec<String>,
    pub session_ids: Vec<i64>,
    pub model: Option<String>,
    pub generated_at: DateTime<Utc>,
}

/// Browse `entry`-status rows, filterable by project and date range,
/// newest-first, paginated. Skip rows never surface.
pub async fn browse(
    _auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<BrowseParams>,
) -> Result<Json<Vec<JournalEntry>>, HubError> {
    let from = parse_date(params.from.as_deref(), "from")?;
    let to = parse_date(params.to.as_deref(), "to")?;
    let page = Page::from(params.limit, params.offset);

    let rows = sqlx::query_as::<_, JournalEntry>(
        r"
        SELECT entry_date, project_path, status, headline, summary, topics,
               open_questions, session_ids, model, generated_at
        FROM journal_entries
        WHERE status = 'entry'
          AND ($1::text IS NULL OR project_path = $1)
          AND ($2::date IS NULL OR entry_date >= $2)
          AND ($3::date IS NULL OR entry_date <= $3)
        ORDER BY entry_date DESC, project_path DESC, id DESC
        LIMIT $4 OFFSET $5
        ",
    )
    .bind(&params.project)
    .bind(from)
    .bind(to)
    .bind(page.limit)
    .bind(page.offset)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(rows))
}

// ---------------------------------------------------------------------------
// journal block for GET /v1/search
// ---------------------------------------------------------------------------

/// One journal search hit. Carries the entry's identifying fields plus its
/// content and independent FTS rank.
#[derive(Debug, Serialize, FromRow)]
pub struct JournalHit {
    pub entry_date: NaiveDate,
    pub project_path: String,
    pub headline: Option<String>,
    pub summary: Option<String>,
    pub topics: Vec<String>,
    pub open_questions: Vec<String>,
    pub session_ids: Vec<i64>,
    pub model: Option<String>,
    pub generated_at: DateTime<Utc>,
    pub rank: f32,
}

/// Ranked FTS over `entry`-status journal rows for the `/v1/search` journal
/// block. `skip` rows have a NULL `search_text` and are filtered out anyway, so
/// they never match. `project`, when set, matches `project_path`.
pub async fn search_journal(
    state: &AppState,
    q: &str,
    project: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<JournalHit>, HubError> {
    let hits = sqlx::query_as::<_, JournalHit>(
        r"
        SELECT entry_date, project_path, headline, summary, topics,
               open_questions, session_ids, model, generated_at,
               ts_rank(text_search, websearch_to_tsquery('simple', $1)) AS rank
        FROM journal_entries
        WHERE status = 'entry'
          AND text_search @@ websearch_to_tsquery('simple', $1)
          AND ($2::text IS NULL OR project_path = $2)
        ORDER BY rank DESC, entry_date DESC, id DESC
        LIMIT $3 OFFSET $4
        ",
    )
    .bind(q)
    .bind(project)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    Ok(hits)
}
