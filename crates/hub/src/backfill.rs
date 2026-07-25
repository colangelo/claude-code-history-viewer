//! One-time backfill of the analytics fields over already-stored messages.
//!
//! Live ingest derives these on insert (see [`crate::extract`]), but the archive
//! holds ~2.6M messages predating that, so the columns and side tables have to
//! be populated retroactively. Everything here is:
//!
//! - **resumable** — progress is a monotonic `messages.id` cursor, so an
//!   interrupted run continues from where it stopped;
//! - **idempotent** — re-running never double-writes (`message_id` is only set
//!   where NULL; derived rows use `ON CONFLICT DO NOTHING`);
//! - **batched** — never one statement over a 6.4 GB table.
//!
//! Measured scope at time of writing (pg1, 2026-07-25): 2,643,609 messages,
//! ~280k carrying a `messageId`, ~128k invocations and ~127k outcomes to derive.

use sqlx::PgPool;

use crate::extract;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BackfillStats {
    pub scanned: u64,
    pub message_ids: u64,
    pub tool_uses: u64,
    pub tool_results: u64,
}

/// Rows per batch. Large enough that the cursor overhead is negligible, small
/// enough that each statement stays short and interruptible.
pub const DEFAULT_BATCH: i64 = 20_000;

/// Populate `messages.message_id` from `raw->>'messageId'`.
///
/// Deliberately expressed in SQL rather than streamed through
/// [`extract::message_id`]: the rule is a single JSONB path, and keeping the
/// 6.4 GB of `raw` inside Postgres is worth more than routing 2.6M rows through
/// the process. `message_id_sql_matches_rust_extractor` in the integration tests
/// pins the two against each other so they cannot drift.
///
/// Returns `(rows_scanned, rows_updated, next_cursor)`; `next_cursor` is `None`
/// when the table is exhausted.
async fn message_id_batch(
    pool: &PgPool,
    after: i64,
    batch: i64,
) -> anyhow::Result<(u64, u64, Option<i64>)> {
    let row: (Option<i64>, i64, i64) = sqlx::query_as(
        r"
        WITH b AS (
            SELECT id FROM messages WHERE id > $1 ORDER BY id LIMIT $2
        ),
        upd AS (
            UPDATE messages m
               SET message_id = m.raw->>'messageId'
              FROM b
             WHERE m.id = b.id
               AND m.message_id IS NULL
               AND nullif(m.raw->>'messageId', '') IS NOT NULL
            RETURNING 1
        )
        SELECT (SELECT max(id) FROM b),
               (SELECT count(*) FROM b),
               (SELECT count(*) FROM upd)
        ",
    )
    .bind(after)
    .bind(batch)
    .fetch_one(pool)
    .await?;

    let (max_id, scanned, updated) = row;
    Ok((scanned as u64, updated as u64, max_id))
}

/// A candidate message for tool extraction, carrying only the fields the
/// extractor reads.
#[derive(sqlx::FromRow)]
struct ToolCandidate {
    id: i64,
    session_id: i64,
    r#type: Option<String>,
    slim_content: Option<serde_json::Value>,
    raw_tool: serde_json::Value,
}

/// Derive `message_tool_uses` / `message_tool_results` for one batch.
///
/// The SELECT **projects** `content` down to just the keys
/// [`extract::tool_uses`] and [`extract::tool_results`] consult, instead of
/// shipping it whole. That is not an optimization detail — a single
/// `tool_result` can carry tens of MiB of command output, and the archive has
/// ~127k of them, so selecting `content` verbatim would move many GB through
/// the process to read a handful of short strings per row.
///
/// **The projection is coupled to the extractor**: every key the extractor
/// reads must appear here, or the backfill will silently derive less than live
/// ingest does. `projection_feeds_the_extractor_identically` in the integration
/// tests pins that.
async fn tool_batch(
    pool: &PgPool,
    after: i64,
    batch: i64,
) -> anyhow::Result<(u64, u64, u64, Option<i64>)> {
    let rows: Vec<ToolCandidate> = sqlx::query_as(
        r#"
        SELECT m.id,
               m.session_id,
               m.type,
               (SELECT jsonb_agg(jsonb_build_object(
                           'type',        e->>'type',
                           'id',          e->>'id',
                           'name',        e->>'name',
                           'tool_use_id', e->>'tool_use_id',
                           'is_error',    e->'is_error',
                           'input',       jsonb_build_object(
                                              'skill',         e->'input'->>'skill',
                                              'subagent_type', e->'input'->>'subagent_type')))
                  FROM jsonb_array_elements(m.content) e
                 WHERE e->>'type' IN ('tool_use', 'tool_result')) AS slim_content,
               jsonb_build_object(
                   'toolUse',       m.raw->'toolUse',
                   'toolUseResult', m.raw->'toolUseResult') AS raw_tool
          FROM messages m
         WHERE m.id > $1
           AND (jsonb_typeof(m.content) = 'array'
                    AND (m.content @> '[{"type": "tool_use"}]'
                      OR m.content @> '[{"type": "tool_result"}]')
                OR m.raw->'toolUse'->>'name' IS NOT NULL)
         ORDER BY m.id
         LIMIT $2
        "#,
    )
    .bind(after)
    .bind(batch)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok((0, 0, 0, None));
    }
    let last_id = rows.last().map(|r| r.id);
    let scanned = rows.len() as u64;

    let mut tu_ref: Vec<i64> = Vec::new();
    let mut tu_session: Vec<i64> = Vec::new();
    let mut tu_seq: Vec<i32> = Vec::new();
    let mut tu_name: Vec<String> = Vec::new();
    let mut tu_use_id: Vec<Option<String>> = Vec::new();
    let mut tu_skill: Vec<Option<String>> = Vec::new();
    let mut tu_subagent: Vec<Option<String>> = Vec::new();
    let mut tu_is_error: Vec<bool> = Vec::new();

    let mut tr_ref: Vec<i64> = Vec::new();
    let mut tr_session: Vec<i64> = Vec::new();
    let mut tr_seq: Vec<i32> = Vec::new();
    let mut tr_use_id: Vec<String> = Vec::new();
    let mut tr_is_error: Vec<bool> = Vec::new();

    for r in &rows {
        let content = r.slim_content.as_ref();
        for u in extract::tool_uses(r.r#type.as_deref(), content, &r.raw_tool) {
            tu_ref.push(r.id);
            tu_session.push(r.session_id);
            tu_seq.push(u.seq);
            tu_name.push(u.tool_name);
            tu_use_id.push(u.tool_use_id);
            tu_skill.push(u.skill_name);
            tu_subagent.push(u.subagent_type);
            tu_is_error.push(u.is_error);
        }
        for res in extract::tool_results(content) {
            tr_ref.push(r.id);
            tr_session.push(r.session_id);
            tr_seq.push(res.seq);
            tr_use_id.push(res.tool_use_id);
            tr_is_error.push(res.is_error);
        }
    }

    let mut uses = 0u64;
    if !tu_ref.is_empty() {
        uses = sqlx::query(
            r"
            INSERT INTO message_tool_uses
                (message_ref, session_id, seq, tool_name, tool_use_id, skill_name,
                 subagent_type, is_error)
            SELECT * FROM UNNEST(
                $1::bigint[], $2::bigint[], $3::int[], $4::text[], $5::text[], $6::text[],
                $7::text[], $8::boolean[]
            )
            ON CONFLICT (message_ref, seq) DO NOTHING
            ",
        )
        .bind(&tu_ref)
        .bind(&tu_session)
        .bind(&tu_seq)
        .bind(&tu_name)
        .bind(&tu_use_id)
        .bind(&tu_skill)
        .bind(&tu_subagent)
        .bind(&tu_is_error)
        .execute(pool)
        .await?
        .rows_affected();
    }

    let mut results = 0u64;
    if !tr_ref.is_empty() {
        results = sqlx::query(
            r"
            INSERT INTO message_tool_results (message_ref, session_id, seq, tool_use_id, is_error)
            SELECT * FROM UNNEST($1::bigint[], $2::bigint[], $3::int[], $4::text[], $5::boolean[])
            ON CONFLICT (message_ref, seq) DO NOTHING
            ",
        )
        .bind(&tr_ref)
        .bind(&tr_session)
        .bind(&tr_seq)
        .bind(&tr_use_id)
        .bind(&tr_is_error)
        .execute(pool)
        .await?
        .rows_affected();
    }

    Ok((scanned, uses, results, last_id))
}

/// Run both backfill phases to completion.
///
/// Phases are sequential rather than interleaved so each keeps its own simple
/// cursor, and so an operator watching the log can tell which one is running.
pub async fn run(pool: &PgPool, batch: i64) -> anyhow::Result<BackfillStats> {
    let mut stats = BackfillStats::default();

    tracing::info!("backfill phase 1/2: messages.message_id");
    let mut cursor = 0i64;
    loop {
        let (scanned, updated, next) = message_id_batch(pool, cursor, batch).await?;
        stats.scanned += scanned;
        stats.message_ids += updated;
        match next {
            Some(id) => cursor = id,
            None => break,
        }
        tracing::info!(
            cursor,
            scanned = stats.scanned,
            filled = stats.message_ids,
            "message_id progress"
        );
    }

    tracing::info!("backfill phase 2/2: tool invocations and outcomes");
    let mut cursor = 0i64;
    loop {
        let (scanned, uses, results, next) = tool_batch(pool, cursor, batch).await?;
        stats.tool_uses += uses;
        stats.tool_results += results;
        match next {
            Some(id) => cursor = id,
            None => break,
        }
        tracing::info!(
            cursor,
            candidates = scanned,
            uses = stats.tool_uses,
            results = stats.tool_results,
            "tool progress"
        );
    }

    tracing::info!(?stats, "backfill complete");
    Ok(stats)
}
