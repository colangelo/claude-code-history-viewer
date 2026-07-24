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
///
/// `pub(crate)` so `health::healthz_journal` folds days identically — the
/// journal-staleness check must see the exact same closed-day boundary the
/// distiller drains against.
pub(crate) const DAY_START_HOUR: i32 = 4;

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
/// session ids the distiller needs (so it takes no second lookup) and the
/// `as_of` snapshot to echo back in the entry POST: storing the snapshot taken
/// *before* the distiller reads any transcript makes dirty-detection cover the
/// whole read-generate-POST window (data committing anywhere inside it is not
/// visible in `as_of` → the group stays dirty).
#[derive(Debug, Serialize, FromRow)]
pub struct PendingGroup {
    pub entry_date: NaiveDate,
    pub project_path: String,
    pub session_ids: Vec<i64>,
    pub as_of: String,
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
/// no journal row, **or** when a session's `ingest_xid` is not visible in the
/// row's `generated_snapshot` (dirty): that ingest committed after the entry's
/// snapshot was taken, so the entry cannot have seen its data — commit-order
/// exact, immune to wall-clock interleaving. Groups on the still-open logical
/// day are excluded. Newest-first, honoring `from` + `limit`.
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
                array_agg(s.ingest_xid)              AS ingest_xids
            FROM sessions s
            JOIN projects p ON s.project_id = p.id
            WHERE s.first_message_time IS NOT NULL
            GROUP BY 1, 2
        )
        SELECT g.entry_date, g.project_path, g.session_ids,
               pg_current_snapshot()::text AS as_of
        FROM grp g
        LEFT JOIN journal_entries j
            ON j.entry_date = g.entry_date
           AND j.project_path = g.project_path
        WHERE g.entry_date
                < ((now() - make_interval(hours => $1::int)) AT TIME ZONE 'UTC')::date
          AND ($2::date IS NULL OR g.entry_date >= $2)
          AND (j.id IS NULL OR EXISTS (
                SELECT 1 FROM unnest(g.ingest_xids) AS x
                WHERE NOT pg_visible_in_snapshot(x, j.generated_snapshot)))
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
    /// The `as_of` snapshot echoed from `GET /v1/journal/pending`. When set,
    /// dirty-detection is anchored to the moment the distiller *read* the
    /// group; when omitted (manual callers, tests) it defaults to POST time.
    #[serde(default)]
    pub as_of: Option<String>,
}

/// Build the flattened FTS text for an entry. Covers the prose (headline,
/// summary, topics, and open questions) AND the entry's identifying fields —
/// `entry_date`, `project_path`, and the session ids — so `/v1/search` can find
/// a journal row by any of them, as the search contract requires.
fn entry_search_text(p: &EntryPayload) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(p.entry_date.format("%Y-%m-%d").to_string());
    parts.push(p.project_path.clone());
    if let Some(h) = &p.headline {
        parts.push(h.clone());
    }
    if let Some(s) = &p.summary {
        parts.push(s.clone());
    }
    parts.extend(p.topics.iter().cloned());
    parts.extend(p.open_questions.iter().cloned());
    parts.extend(p.session_ids.iter().map(i64::to_string));
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
        // The spec requires every entry to record the model that generated it.
        if payload.model.as_deref().unwrap_or("").trim().is_empty() {
            return Err(HubError::BadRequest(
                "entry status requires a non-empty model".into(),
            ));
        }
    }

    // Session provenance is mandatory for BOTH statuses: an entry distills real
    // archived sessions, and a skip watermark must record which sessions it
    // judged. An empty set would let a caller clear pending with no drill-down
    // provenance, so reject it.
    if payload.session_ids.is_empty() {
        return Err(HubError::BadRequest(
            "session_ids must reference at least one archived session".into(),
        ));
    }
    let mut ids = payload.session_ids.clone();
    ids.sort_unstable();
    ids.dedup();

    // Provenance must be exact: every referenced session must BELONG to the
    // posted (entry_date, project_path) group — same logical-day fold as the
    // pending query — and the set must COVER the whole group. Membership stops
    // a mismatched id (existing, but from another project/date) from clearing
    // a group it never distilled; coverage stops a partial set from
    // watermarking sessions it never saw. A group that grew between the
    // distiller's pending read and this POST is therefore rejected — correct,
    // since the group is dirty again anyway and re-distills next run.
    let group_ids: Vec<i64> = sqlx::query_scalar(
        r"
        SELECT s.id
        FROM sessions s
        JOIN projects p ON s.project_id = p.id
        WHERE p.project_path = $1
          AND s.first_message_time IS NOT NULL
          AND ((s.first_message_time - make_interval(hours => $2::int))
                AT TIME ZONE 'UTC')::date = $3
        ORDER BY s.id
        ",
    )
    .bind(&payload.project_path)
    .bind(DAY_START_HOUR)
    .bind(payload.entry_date)
    .fetch_all(&state.pool)
    .await?;
    if ids.iter().any(|id| !group_ids.contains(id)) {
        return Err(HubError::BadRequest(
            "one or more session ids do not belong to this (entry_date, project_path) group".into(),
        ));
    }
    if group_ids.iter().any(|id| !ids.contains(id)) {
        return Err(HubError::BadRequest(
            "incomplete provenance: session_ids must cover every archived session in the group"
                .into(),
        ));
    }

    // Validate the optional `as_of` snapshot before it reaches the INSERT (a
    // bad cast inside the write would surface as a 500, not a client error).
    if let Some(snap) = payload.as_of.as_deref() {
        sqlx::query_scalar::<_, String>("SELECT ($1::pg_snapshot)::text")
            .bind(snap)
            .fetch_one(&state.pool)
            .await
            .map_err(|_| {
                HubError::BadRequest("invalid `as_of` (expected a pg_snapshot string)".into())
            })?;
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

    // `generated_snapshot` anchors dirty-detection: sessions whose `ingest_xid`
    // is not visible in it are dirty. The distiller echoes the `as_of` snapshot
    // it received from the pending endpoint (taken BEFORE it read any
    // transcript), so data committing anywhere in the read-generate-POST window
    // keeps the group dirty; callers that omit it (tests, manual) anchor to
    // POST time. `generated_at` stays as the human-facing timestamp.
    sqlx::query(
        r"
        INSERT INTO journal_entries
            (entry_date, project_path, status, headline, summary, topics,
             open_questions, session_ids, model, generated_at, generated_snapshot,
             search_text)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, clock_timestamp(),
                coalesce($10::pg_snapshot, pg_current_snapshot()), $11)
        ON CONFLICT (entry_date, project_path)
        DO UPDATE SET status             = excluded.status,
                      headline           = excluded.headline,
                      summary            = excluded.summary,
                      topics             = excluded.topics,
                      open_questions     = excluded.open_questions,
                      session_ids        = excluded.session_ids,
                      model              = excluded.model,
                      generated_at       = clock_timestamp(),
                      generated_snapshot = excluded.generated_snapshot,
                      search_text        = excluded.search_text
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
    .bind(&payload.as_of)
    .bind(search_text)
    .execute(&state.pool)
    .await?;

    // Fresh content should become semantically searchable promptly: nudge the
    // embedding sweep (no-op when no sweeper is running).
    state.embed_nudge.notify_one();

    Ok(Json(serde_json::json!({ "status": "ok" })))
}

// ---------------------------------------------------------------------------
// GET /v1/journal/entries (browse)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BrowseParams {
    /// Match `project_path`, or `identity:<key>` for server-side expansion
    /// to the identity's member + aliased paths.
    pub project: Option<String>,
    /// In identity scope: `false` excludes worktree-only member paths.
    pub include_worktrees: Option<bool>,
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
    let scope = crate::identity_filter::resolve_project_scope(
        &state.pool,
        params.project.as_deref(),
        params.include_worktrees.unwrap_or(true),
    )
    .await?;

    let rows = sqlx::query_as::<_, JournalEntry>(
        r"
        SELECT entry_date, project_path, status, headline, summary, topics,
               open_questions, session_ids, model, generated_at
        FROM journal_entries
        WHERE status = 'entry'
          AND ($1::text IS NULL OR project_path = $1)
          AND ($6::text[] IS NULL OR project_path = ANY($6))
          AND ($2::date IS NULL OR entry_date >= $2)
          AND ($3::date IS NULL OR entry_date <= $3)
        ORDER BY entry_date DESC, project_path DESC, id DESC
        LIMIT $4 OFFSET $5
        ",
    )
    .bind(&scope.plain)
    .bind(from)
    .bind(to)
    .bind(page.limit)
    .bind(page.offset)
    .bind(scope.paths.as_deref())
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

/// Journal retrieval mode for the `/v1/search` journal block. `Keyword` is
/// the default and byte-compatible with pre-mode behavior; the other two are
/// defined in `openspec/specs/semantic-search/spec.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalMode {
    Keyword,
    Semantic,
    Hybrid,
}

impl JournalMode {
    pub fn parse(s: Option<&str>) -> Result<Self, HubError> {
        match s.unwrap_or("keyword") {
            "keyword" => Ok(Self::Keyword),
            "semantic" => Ok(Self::Semantic),
            "hybrid" => Ok(Self::Hybrid),
            other => Err(HubError::BadRequest(format!(
                "unknown mode `{other}` (expected `keyword`, `semantic`, or `hybrid`)"
            ))),
        }
    }
}

/// Reciprocal-rank-fusion constant (the standard k=60): rank-based fusion
/// needs no score normalization between `ts_rank` and cosine.
const RRF_K: f64 = 60.0;

/// A keyword/semantic hit paired with its journal-row id (ids drive fusion
/// and are not serialized).
#[derive(FromRow)]
struct RankedRow {
    id: i64,
    #[sqlx(flatten)]
    hit: JournalHit,
}

/// Ranked journal block for `/v1/search`. Returns the hits plus a degraded
/// flag: `true` when a semantic/hybrid request fell back to keyword because
/// the embedder or embeddings were unavailable (never an error). The project
/// scope carries either a plain `project_path` match or a pre-resolved
/// identity path set (see `identity_filter`).
pub async fn search_journal(
    state: &AppState,
    q: &str,
    scope: &crate::identity_filter::ProjectScope,
    limit: i64,
    offset: i64,
    mode: JournalMode,
) -> Result<(Vec<JournalHit>, bool), HubError> {
    if mode == JournalMode::Keyword {
        let hits = keyword_hits(state, q, scope, limit, offset).await?;
        return Ok((hits.into_iter().map(|r| r.hit).collect(), false));
    }

    // Semantic ranking over the scoped active-model vectors; `None` = the
    // capability is unavailable right now → keyword results + degraded flag.
    let Some(ranked) = semantic_ranked(state, q, scope).await? else {
        let hits = keyword_hits(state, q, scope, limit, offset).await?;
        return Ok((hits.into_iter().map(|r| r.hit).collect(), true));
    };

    // `top_k` = offset+limit rows from each ranking fill the requested page
    // from either side.
    let top_k: i64 = offset.max(0) + limit.max(0);
    let (start, end) = (offset.max(0) as usize, top_k as usize);
    match mode {
        JournalMode::Keyword => unreachable!("handled above"),
        JournalMode::Semantic => {
            let page: Vec<(i64, f64)> = ranked
                .iter()
                .skip(start)
                .take(end - start)
                .map(|(id, sim)| (*id, f64::from(*sim)))
                .collect();
            Ok((hits_for_ranked(state, &page).await?, false))
        }
        JournalMode::Hybrid => {
            let keyword = keyword_hits(state, q, scope, top_k, 0).await?;
            let mut scores: std::collections::HashMap<i64, f64> = std::collections::HashMap::new();
            let mut hit_by_id: std::collections::HashMap<i64, JournalHit> =
                std::collections::HashMap::new();
            for (pos, row) in keyword.into_iter().enumerate() {
                *scores.entry(row.id).or_default() += 1.0 / (RRF_K + (pos + 1) as f64);
                hit_by_id.insert(row.id, row.hit);
            }
            for (pos, (id, _sim)) in ranked.iter().take(end).enumerate() {
                *scores.entry(*id).or_default() += 1.0 / (RRF_K + (pos + 1) as f64);
            }
            let mut fused: Vec<(i64, f64)> = scores.into_iter().collect();
            fused.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(b.0.cmp(&a.0))
            });
            let page: Vec<(i64, f64)> = fused.into_iter().skip(start).take(end - start).collect();
            // Reuse hits already fetched by the keyword leg; fetch the rest.
            let missing: Vec<i64> = page
                .iter()
                .map(|(id, _)| *id)
                .filter(|id| !hit_by_id.contains_key(id))
                .collect();
            hit_by_id.extend(hits_by_ids(state, &missing).await?);
            let mut out = Vec::with_capacity(page.len());
            for (id, score) in page {
                if let Some(mut hit) = hit_by_id.remove(&id) {
                    hit.rank = score as f32;
                    out.push(hit);
                }
            }
            Ok((out, false))
        }
    }
}

/// The pre-mode FTS ranking, unchanged except for carrying the row id.
/// `skip` rows have a NULL `search_text` and are filtered out anyway, so
/// they never match. Prefix variant for plain queries (issue #19) — see
/// `fts::prefix_tsquery`.
async fn keyword_hits(
    state: &AppState,
    q: &str,
    scope: &crate::identity_filter::ProjectScope,
    limit: i64,
    offset: i64,
) -> Result<Vec<RankedRow>, HubError> {
    let prefix = crate::fts::prefix_tsquery(q);
    let hits = sqlx::query_as::<_, RankedRow>(
        r"
        WITH q AS (
            SELECT CASE
                WHEN $6::text IS NULL THEN websearch_to_tsquery('simple', $1)
                ELSE websearch_to_tsquery('simple', $1) || to_tsquery('simple', $6)
            END AS tsq
        )
        SELECT id, entry_date, project_path, headline, summary, topics,
               open_questions, session_ids, model, generated_at,
               ts_rank(text_search, q.tsq) AS rank
        FROM q, journal_entries
        WHERE status = 'entry'
          AND text_search @@ q.tsq
          AND ($2::text IS NULL OR project_path = $2)
          AND ($5::text[] IS NULL OR project_path = ANY($5))
        ORDER BY rank DESC, entry_date DESC, id DESC
        LIMIT $3 OFFSET $4
        ",
    )
    .bind(q)
    .bind(&scope.plain)
    .bind(limit)
    .bind(offset)
    .bind(scope.paths.as_deref())
    .bind(prefix)
    .fetch_all(&state.pool)
    .await?;

    Ok(hits)
}

/// Full best-first semantic ranking of the scoped entries, or `None` when
/// the capability is unavailable (no embedder configured, embed failure, or
/// no active-model vectors in scope) — the caller degrades to keyword.
/// Exact in-process cosine over the whole (journal-scale) set; vectors whose
/// dimension mismatches the query are skipped defensively.
async fn semantic_ranked(
    state: &AppState,
    q: &str,
    scope: &crate::identity_filter::ProjectScope,
) -> Result<Option<Vec<(i64, f32)>>, HubError> {
    let Some(embedder) = state.embedder.as_ref() else {
        return Ok(None);
    };
    let query_vec = match embedder.embed(&crate::embed::query_text(q)) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(error = %format!("{e:#}"), "query embed unavailable; degrading");
            return Ok(None);
        }
    };

    let rows = sqlx::query(
        r"
        SELECT e.journal_entry_id, e.embedding
        FROM journal_embeddings e
        JOIN journal_entries je ON je.id = e.journal_entry_id
        WHERE e.model = $1
          AND je.status = 'entry'
          AND ($2::text IS NULL OR je.project_path = $2)
          AND ($3::text[] IS NULL OR je.project_path = ANY($3))
        ",
    )
    .bind(embedder.model_id())
    .bind(&scope.plain)
    .bind(scope.paths.as_deref())
    .fetch_all(&state.pool)
    .await?;

    let mut ranked: Vec<(i64, f32)> = rows
        .iter()
        .filter_map(|row| {
            use sqlx::Row;
            let id: i64 = row.get(0);
            let embedding: Vec<f32> = row.get(1);
            crate::embed::cosine(&query_vec, &embedding).map(|sim| (id, sim))
        })
        .collect();
    if ranked.is_empty() {
        return Ok(None);
    }
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.0.cmp(&a.0))
    });
    Ok(Some(ranked))
}

/// Fetch full journal hits for `(id, score)` pairs, preserving the given
/// order and stamping `rank` with the mode's score.
async fn hits_for_ranked(
    state: &AppState,
    ranked: &[(i64, f64)],
) -> Result<Vec<JournalHit>, HubError> {
    let ids: Vec<i64> = ranked.iter().map(|(id, _)| *id).collect();
    let mut by_id = hits_by_ids(state, &ids).await?;
    Ok(ranked
        .iter()
        .filter_map(|(id, score)| {
            by_id.remove(id).map(|mut hit| {
                hit.rank = *score as f32;
                hit
            })
        })
        .collect())
}

/// Load journal hits keyed by row id (`rank` left at 0 for the caller to
/// stamp with the active mode's score).
async fn hits_by_ids(
    state: &AppState,
    ids: &[i64],
) -> Result<std::collections::HashMap<i64, JournalHit>, HubError> {
    if ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let rows = sqlx::query_as::<_, RankedRow>(
        r"
        SELECT id, entry_date, project_path, headline, summary, topics,
               open_questions, session_ids, model, generated_at,
               0.0::float4 AS rank
        FROM journal_entries
        WHERE id = ANY($1)
        ",
    )
    .bind(ids)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows.into_iter().map(|r| (r.id, r.hit)).collect())
}
