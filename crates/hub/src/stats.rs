//! Aggregate statistics over the archive.
//!
//! Replaces the retired desktop implementation (`src-tauri/src/commands/stats.rs`,
//! 6k LOC of mmap + SIMD JSONL scanning) with SQL, because ingest has already
//! extracted every field those scans were looking for. The desktop version is
//! the verification oracle for everything here — see the change's design.md for
//! the two metrics that deliberately diverge from it (D10 success rate, D12 tool
//! counts).
//!
//! Runtime `sqlx::query*` throughout, not the `query!` macros: the scope
//! predicate is uniform but the CI gate builds with `SQLX_OFFLINE` and has no
//! `.sqlx` metadata for new statements (same rationale as `journal.rs`).

use chrono::NaiveDate;
use history_core::models::{
    ActivityHeatmap, DailyStats, DateRange, GlobalStatsSummary, ModelStats, ProjectRanking,
    ProjectStatsSummary, ProviderUsageStats, SessionTokenStats, TokenDistribution, ToolUsageStats,
};
use sqlx::PgPool;

/// Time window and the timezone its day/hour buckets are expressed in.
///
/// Bucketing is done server-side (`AT TIME ZONE`) rather than shipping rows for
/// the client to re-bucket: "what hours do I work" is meaningless in UTC for a
/// user in `Europe/Rome`, and the conversion belongs next to the index.
#[derive(Debug, Clone)]
pub struct Window {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub tz: String,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            from: None,
            to: None,
            tz: "UTC".to_string(),
        }
    }
}

/// What the statistics cover.
#[derive(Debug, Clone)]
pub enum Scope {
    /// The whole archive.
    Global,
    /// One project identity, expanded to its member paths (fingerprinted rows
    /// plus manual aliases) by `identity_filter::resolve_project_scope`, so a
    /// repository that moved or was cloned reports as one project (design D6).
    Paths(Vec<String>),
    /// One session, by surrogate id.
    Session(i64),
}

impl Scope {
    fn paths(&self) -> Option<&[String]> {
        match self {
            Self::Paths(p) => Some(p),
            _ => None,
        }
    }
    fn session(&self) -> Option<i64> {
        match self {
            Self::Session(id) => Some(*id),
            _ => None,
        }
    }
}

/// Materialize the scoped + deduplicated messages ONCE per request.
///
/// **The dedup is the whole point.** One assistant response occupies several
/// stored rows carrying an identical `usage` block, so a plain `SUM` over
/// `messages` over-reports — measured at +51.5% on the live archive.
/// `DISTINCT ON` collapses to one row per `(session, provider message id)`,
/// falling back to `uuid` and finally the row id, reproducing
/// `dedup_token_totals` in the desktop oracle (`stats.rs:1500`). The row id
/// fallback is unique, so a message carrying neither identifier is never folded
/// into another.
///
/// **Why a temp table rather than a CTE per query.** Each dedup pass is a
/// parallel seq scan plus an external merge sort that spills ~60 MB per worker
/// to disk — about 3 s on the 2.6M-row archive. A summary needs ~10 rollups, and
/// re-deriving the CTE inside each one cost ~31 s per request (measured against
/// pg1). Materializing once and aggregating over the result turns that into one
/// sort plus cheap scans.
///
/// `ON COMMIT DROP` ties the table's lifetime to the surrounding transaction, so
/// there is nothing to clean up and concurrent requests cannot collide — each
/// session gets its own.
async fn materialize_scope(
    tx: &mut sqlx::PgConnection,
    scope: &Scope,
    w: &Window,
) -> sqlx::Result<()> {
    let sql = r#"
        CREATE TEMP TABLE stats_scope ON COMMIT DROP AS
        WITH scoped AS (
            SELECT m.id, m.session_id, m.message_id, m.uuid, m.provider, m.model,
                   m."timestamp", m.input_tokens, m.output_tokens,
                   m.cache_creation_tokens, m.cache_read_tokens, m.cost_usd,
                   s.project_id
              FROM messages m
              JOIN sessions s  ON s.id = m.session_id
              LEFT JOIN projects p ON p.id = s.project_id
             WHERE ($1::text[]  IS NULL OR p.project_path = ANY($1))
               AND ($2::bigint  IS NULL OR m.session_id = $2)
               AND ($4::date    IS NULL OR (m."timestamp" AT TIME ZONE $3)::date >= $4)
               AND ($5::date    IS NULL OR (m."timestamp" AT TIME ZONE $3)::date <= $5)
        )
        SELECT DISTINCT ON (session_id, COALESCE(message_id, uuid, id::text)) *
          FROM scoped
         ORDER BY session_id, COALESCE(message_id, uuid, id::text), id
    "#;
    sqlx::query(sql)
        .bind(scope.paths().map(<[String]>::to_vec))
        .bind(scope.session())
        .bind(&w.tz)
        .bind(w.from)
        .bind(w.to)
        .execute(&mut *tx)
        .await?;
    // Without stats the planner treats the temp table as tiny and picks bad
    // plans for the joins below.
    sqlx::query("ANALYZE stats_scope").execute(&mut *tx).await?;
    Ok(())
}

type DailyRow = (String, i64, i64, i64, i64, i64, i64, Option<f64>);
type ModelRow = (String, i64, i64, i64, i64, i64, i64, Option<f64>);
/// Totals plus the date range and summed session duration, from one scan.
type TotalsRow = (
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<f64>,
    i64,
    i64,
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
    i32,
    i64,
);

/// Sum of the four token columns, as SQL. Repeated in several rollups, so it
/// lives in one place rather than being retyped (and mistyped) per query.
const TOKEN_SUM: &str = "coalesce(sum(coalesce(input_tokens,0) + coalesce(output_tokens,0)
                       + coalesce(cache_creation_tokens,0)
                       + coalesce(cache_read_tokens,0)), 0)::bigint";

/// Token/cost totals, message and session counts, and cost coverage.
async fn totals(tx: &mut sqlx::PgConnection) -> sqlx::Result<TotalsRow> {
    sqlx::query_as::<_, TotalsRow>(
        r#"
        SELECT sum(input_tokens)::bigint,
               sum(output_tokens)::bigint,
               sum(cache_creation_tokens)::bigint,
               sum(cache_read_tokens)::bigint,
               -- NOT coalesced to 0: NULL means 'nobody reported cost', which
               -- is a different statement from 'it was free'.
               sum(cost_usd),
               count(*) FILTER (WHERE cost_usd IS NOT NULL),
               count(*),
               count(DISTINCT session_id),
               count(DISTINCT project_id),
               -- Folded in rather than separate statements: each one would be
               -- another full scan of a 2.5M-row temp table.
               to_char(min("timestamp") AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SSZ'),
               to_char(max("timestamp") AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SSZ'),
               coalesce(date_part('day', max("timestamp") - min("timestamp")), 0)::int,
               (SELECT coalesce(sum(EXTRACT(EPOCH FROM (last - first)) / 60), 0)::bigint
                  FROM (SELECT session_id, min("timestamp") AS first, max("timestamp") AS last
                          FROM stats_scope GROUP BY session_id) s)
          FROM stats_scope"#,
    )
    .fetch_one(&mut *tx)
    .await
}

/// Per-day buckets in the caller's timezone.
async fn daily(tx: &mut sqlx::PgConnection, tz: &str) -> sqlx::Result<Vec<DailyStats>> {
    let sql = format!(
        r#"SELECT to_char(("timestamp" AT TIME ZONE $1)::date, 'YYYY-MM-DD'),
                  {TOKEN_SUM},
                  coalesce(sum(input_tokens), 0)::bigint,
                  coalesce(sum(output_tokens), 0)::bigint,
                  count(*),
                  count(DISTINCT session_id),
                  count(DISTINCT extract(hour FROM "timestamp" AT TIME ZONE $1)),
                  sum(cost_usd)
             FROM stats_scope
            WHERE "timestamp" IS NOT NULL
            GROUP BY 1 ORDER BY 1"#
    );
    let rows: Vec<DailyRow> = sqlx::query_as(&sql).bind(tz).fetch_all(&mut *tx).await?;
    Ok(rows
        .into_iter()
        .map(|r| DailyStats {
            date: r.0,
            total_tokens: r.1.max(0) as u64,
            input_tokens: r.2.max(0) as u64,
            output_tokens: r.3.max(0) as u64,
            message_count: r.4.max(0) as usize,
            session_count: r.5.max(0) as usize,
            active_hours: r.6.max(0) as usize,
            cost_usd: r.7,
        })
        .collect())
}

/// Hour-of-day × day-of-week activity.
async fn heatmap(tx: &mut sqlx::PgConnection, tz: &str) -> sqlx::Result<Vec<ActivityHeatmap>> {
    let sql = format!(
        r#"SELECT extract(hour FROM "timestamp" AT TIME ZONE $1)::int,
                  extract(dow  FROM "timestamp" AT TIME ZONE $1)::int,
                  count(*)::bigint,
                  {TOKEN_SUM}
             FROM stats_scope
            WHERE "timestamp" IS NOT NULL
            GROUP BY 1, 2 ORDER BY 2, 1"#
    );
    let rows: Vec<(i32, i32, i64, i64)> = sqlx::query_as(&sql).bind(tz).fetch_all(&mut *tx).await?;
    Ok(rows
        .into_iter()
        .map(|r| ActivityHeatmap {
            hour: r.0.clamp(0, 23) as u8,
            day: r.1.clamp(0, 6) as u8,
            activity_count: r.2.max(0) as u32,
            tokens_used: r.3.max(0) as u64,
        })
        .collect())
}

/// Which usage collection a row belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Tool,
    Skill,
    Subagent,
}

impl ToolKind {
    fn tag(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Skill => "skill",
            Self::Subagent => "subagent",
        }
    }
}

/// All three usage collections — tools, skills, subagents — in ONE pass.
///
/// Previously three near-identical queries, each re-joining
/// `message_tool_uses` to the scope and to the outcomes. That join is the most
/// expensive part of a summary, so the `VALUES` lateral fans each invocation
/// out into whichever collections it belongs to and groups them together,
/// paying for the join once instead of three times.
///
/// Success is resolved by `LEFT JOIN`ing the outcome that reports on the
/// invocation, preferring it over the invocation's own flag (design D10):
/// `COALESCE(r.is_error, u.is_error, false)`. The join is scoped by
/// `(session_id, tool_use_id)` — **`tool_use_id` alone is not unique across the
/// archive and fans out** (design D11).
///
/// Joining against `stats_scope` rather than `messages` means invocations are
/// deduplicated with their messages: a response stored three times contributes
/// its tool call once, exactly as it contributes its tokens once.
///
/// `avg_execution_time` is `None`: `duration_ms` is a per-message figure and
/// cannot be attributed to one invocation of several in that message.
async fn all_tool_usage(
    tx: &mut sqlx::PgConnection,
    limit: usize,
) -> sqlx::Result<(
    Vec<ToolUsageStats>,
    Vec<ToolUsageStats>,
    Vec<ToolUsageStats>,
)> {
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        r"
        SELECT k.kind, k.name,
               count(*)::bigint,
               count(*) FILTER (WHERE NOT COALESCE(r.is_error, u.is_error, false))::bigint
          FROM message_tool_uses u
          JOIN stats_scope d ON d.id = u.message_ref
          LEFT JOIN message_tool_results r
                 ON r.session_id = u.session_id AND r.tool_use_id = u.tool_use_id
          CROSS JOIN LATERAL (VALUES ('tool', u.tool_name),
                                     ('skill', u.skill_name),
                                     ('subagent', u.subagent_type)) AS k(kind, name)
         WHERE k.name IS NOT NULL
         GROUP BY 1, 2
         ORDER BY 3 DESC
        ",
    )
    .fetch_all(&mut *tx)
    .await?;

    let take = |kind: ToolKind| -> Vec<ToolUsageStats> {
        rows.iter()
            .filter(|r| r.0 == kind.tag())
            .take(limit)
            .map(|r| ToolUsageStats {
                tool_name: r.1.clone(),
                usage_count: r.2.max(0) as u32,
                success_rate: if r.2 > 0 {
                    r.3 as f32 / r.2 as f32
                } else {
                    0.0
                },
                avg_execution_time: None,
            })
            .collect()
    };
    Ok((
        take(ToolKind::Tool),
        take(ToolKind::Skill),
        take(ToolKind::Subagent),
    ))
}

async fn models(tx: &mut sqlx::PgConnection) -> sqlx::Result<Vec<ModelStats>> {
    let sql = format!(
        "SELECT model, count(*)::bigint, {TOKEN_SUM},
                coalesce(sum(input_tokens),0)::bigint,
                coalesce(sum(output_tokens),0)::bigint,
                coalesce(sum(cache_creation_tokens),0)::bigint,
                coalesce(sum(cache_read_tokens),0)::bigint,
                sum(cost_usd)
           FROM stats_scope WHERE model IS NOT NULL
          GROUP BY 1 ORDER BY 3 DESC"
    );
    let rows: Vec<ModelRow> = sqlx::query_as(&sql).fetch_all(&mut *tx).await?;
    Ok(rows
        .into_iter()
        .map(|r| ModelStats {
            model_name: r.0,
            message_count: r.1.max(0) as u32,
            token_count: r.2.max(0) as u64,
            input_tokens: r.3.max(0) as u64,
            output_tokens: r.4.max(0) as u64,
            cache_creation_tokens: r.5.max(0) as u64,
            cache_read_tokens: r.6.max(0) as u64,
            // No provider reports reasoning tokens through `TokenUsage`, so the
            // oracle's value is 0 too — this is parity, not a gap.
            reasoning_tokens: 0,
            cost_usd: r.7,
        })
        .collect())
}

async fn providers(tx: &mut sqlx::PgConnection) -> sqlx::Result<Vec<ProviderUsageStats>> {
    let sql = format!(
        "SELECT provider,
                count(DISTINCT project_id)::bigint,
                count(DISTINCT session_id)::bigint,
                count(*)::bigint,
                {TOKEN_SUM}
           FROM stats_scope GROUP BY 1 ORDER BY 4 DESC"
    );
    let rows: Vec<(String, i64, i64, i64, i64)> = sqlx::query_as(&sql).fetch_all(&mut *tx).await?;
    Ok(rows
        .into_iter()
        .map(|r| ProviderUsageStats {
            provider_id: r.0,
            projects: r.1.max(0) as u32,
            sessions: r.2.max(0) as u32,
            messages: r.3.max(0) as u32,
            tokens: r.4.max(0) as u64,
        })
        .collect())
}

async fn top_projects(tx: &mut sqlx::PgConnection) -> sqlx::Result<Vec<ProjectRanking>> {
    let sql = format!(
        "SELECT coalesce(p.name, p.project_path, '(unknown)'),
                count(DISTINCT d.session_id)::bigint,
                count(*)::bigint,
                {TOKEN_SUM}
           FROM stats_scope d LEFT JOIN projects p ON p.id = d.project_id
          GROUP BY 1 ORDER BY 4 DESC LIMIT 10"
    );
    let rows: Vec<(String, i64, i64, i64)> = sqlx::query_as(&sql).fetch_all(&mut *tx).await?;
    Ok(rows
        .into_iter()
        .map(|r| ProjectRanking {
            project_name: r.0,
            sessions: r.1.max(0) as u32,
            messages: r.2.max(0) as u32,
            tokens: r.3.max(0) as u64,
        })
        .collect())
}

fn distribution(t: &TotalsRow) -> TokenDistribution {
    TokenDistribution {
        input: t.0.unwrap_or(0).max(0) as u64,
        output: t.1.unwrap_or(0).max(0) as u64,
        cache_creation: t.2.unwrap_or(0).max(0) as u64,
        cache_read: t.3.unwrap_or(0).max(0) as u64,
        reasoning: 0,
    }
}

fn total_tokens(d: &TokenDistribution) -> u64 {
    d.input + d.output + d.cache_creation + d.cache_read
}

/// Archive-wide statistics.
pub async fn global(pool: &PgPool, w: &Window) -> sqlx::Result<GlobalStatsSummary> {
    let mut tx = pool.begin().await?;
    materialize_scope(&mut tx, &Scope::Global, w).await?;

    let t = totals(&mut tx).await?;
    let dist = distribution(&t);
    let tool_usage = all_tool_usage(&mut tx, 20).await?;
    let out = GlobalStatsSummary {
        total_projects: t.8.unwrap_or(0).max(0) as u32,
        total_sessions: t.7.max(0) as u32,
        total_messages: t.6.max(0) as u32,
        total_tokens: total_tokens(&dist),
        total_cost_usd: t.4,
        cost_reported_messages: t.5.max(0) as u32,
        total_session_duration_minutes: t.12.max(0) as u64,
        date_range: DateRange {
            first_message: t.9.clone(),
            last_message: t.10.clone(),
            days_span: t.11.max(0) as u32,
        },
        token_distribution: dist,
        daily_stats: daily(&mut tx, &w.tz).await?,
        activity_heatmap: heatmap(&mut tx, &w.tz).await?,
        most_used_tools: tool_usage.0,
        most_used_skills: tool_usage.1,
        most_used_subagents: tool_usage.2,
        provider_distribution: providers(&mut tx).await?,
        model_distribution: models(&mut tx).await?,
        top_projects: top_projects(&mut tx).await?,
    };
    // Commit (not rollback) so `ON COMMIT DROP` fires normally; nothing was
    // written outside the temp table.
    tx.commit().await?;
    Ok(out)
}

/// Statistics for one project identity, folded across its member paths.
///
/// `paths` comes from `identity_filter::resolve_project_scope`. An empty slice
/// (unknown identity) correctly matches nothing, which the caller turns into a
/// `404` rather than an empty-but-successful body.
pub async fn project(
    pool: &PgPool,
    name: String,
    paths: Vec<String>,
    w: &Window,
) -> sqlx::Result<ProjectStatsSummary> {
    let mut tx = pool.begin().await?;
    materialize_scope(&mut tx, &Scope::Paths(paths), w).await?;

    let t = totals(&mut tx).await?;
    let dist = distribution(&t);
    let sessions = t.7.max(0) as usize;
    let duration = t.12.max(0) as u32;
    let heat = heatmap(&mut tx, &w.tz).await?;
    let most_active_hour = heat
        .iter()
        .max_by_key(|h| h.activity_count)
        .map_or(0, |h| h.hour);
    let tokens = total_tokens(&dist);
    let tool_usage = all_tool_usage(&mut tx, 20).await?;
    let out = ProjectStatsSummary {
        project_name: name,
        total_sessions: sessions,
        total_messages: t.6.max(0) as usize,
        total_tokens: tokens,
        total_cost_usd: t.4,
        cost_reported_messages: t.5.max(0) as u32,
        avg_tokens_per_session: if sessions > 0 {
            tokens / sessions as u64
        } else {
            0
        },
        avg_session_duration: if sessions > 0 {
            duration / sessions as u32
        } else {
            0
        },
        total_session_duration: duration,
        most_active_hour,
        most_used_tools: tool_usage.0,
        most_used_skills: tool_usage.1,
        most_used_subagents: tool_usage.2,
        daily_stats: daily(&mut tx, &w.tz).await?,
        activity_heatmap: heat,
        token_distribution: dist,
    };
    tx.commit().await?;
    Ok(out)
}

/// Statistics for one session. `None` when the session does not exist.
pub async fn session(
    pool: &PgPool,
    session_pk: i64,
    w: &Window,
) -> sqlx::Result<Option<SessionTokenStats>> {
    let meta: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
        r"
        SELECT s.session_id, s.summary, coalesce(p.name, p.project_path)
          FROM sessions s LEFT JOIN projects p ON p.id = s.project_id
         WHERE s.id = $1
        ",
    )
    .bind(session_pk)
    .fetch_optional(pool)
    .await?;
    let Some((provider_session_id, summary, project_name)) = meta else {
        return Ok(None);
    };

    let mut tx = pool.begin().await?;
    materialize_scope(&mut tx, &Scope::Session(session_pk), w).await?;

    let t = totals(&mut tx).await?;
    let dist = distribution(&t);

    let out = SessionTokenStats {
        session_id: provider_session_id,
        project_name: project_name.unwrap_or_else(|| "(unknown)".to_string()),
        total_input_tokens: dist.input,
        total_output_tokens: dist.output,
        total_cache_creation_tokens: dist.cache_creation,
        total_cache_read_tokens: dist.cache_read,
        total_reasoning_tokens: 0,
        total_tokens: total_tokens(&dist),
        total_cost_usd: t.4,
        message_count: t.6.max(0) as usize,
        first_message_time: t.9.clone().unwrap_or_default(),
        last_message_time: t.10.clone().unwrap_or_default(),
        summary,
        most_used_tools: all_tool_usage(&mut tx, 20).await?.0,
    };
    tx.commit().await?;
    Ok(Some(out))
}
