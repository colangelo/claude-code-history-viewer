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

/// The scoped + deduplicated message relation every usage rollup builds on.
///
/// **The dedup is the whole point.** One assistant response occupies several
/// stored rows carrying an identical `usage` block, so a plain `SUM` over
/// `messages` over-reports. `DISTINCT ON` collapses to one row per
/// `(session, provider message id)`, falling back to `uuid` and finally the row
/// id — reproducing `dedup_token_totals` in the desktop oracle exactly
/// (`stats.rs:1500`). The row id fallback is unique, so a message carrying
/// neither identifier is never folded into another.
///
/// Bind order is fixed for every caller: `$1` paths, `$2` session id, `$3` tz,
/// `$4` from, `$5` to. Keeping it uniform is what lets one SQL body serve all
/// three scopes.
const SCOPED_DEDUPED: &str = r#"
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
),
deduped AS (
    SELECT DISTINCT ON (session_id, COALESCE(message_id, uuid, id::text)) *
      FROM scoped
     ORDER BY session_id, COALESCE(message_id, uuid, id::text), id
)
"#;

/// Bind the five uniform scope/window parameters, in order.
macro_rules! bind_scope {
    ($q:expr, $scope:expr, $w:expr) => {
        $q.bind($scope.paths().map(<[String]>::to_vec))
            .bind($scope.session())
            .bind(&$w.tz)
            .bind($w.from)
            .bind($w.to)
    };
}

type DailyRow = (String, i64, i64, i64, i64, i64, i64, Option<f64>);
type ModelRow = (String, i64, i64, i64, i64, i64, i64, Option<f64>);

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
);

/// Token/cost totals, message and session counts, and cost coverage.
async fn totals(pool: &PgPool, scope: &Scope, w: &Window) -> sqlx::Result<TotalsRow> {
    let sql = format!(
        "{SCOPED_DEDUPED}
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
               count(DISTINCT project_id)
          FROM deduped"
    );
    bind_scope!(sqlx::query_as::<_, TotalsRow>(&sql), scope, w)
        .fetch_one(pool)
        .await
}

/// Per-day buckets in the caller's timezone.
async fn daily(pool: &PgPool, scope: &Scope, w: &Window) -> sqlx::Result<Vec<DailyStats>> {
    let sql = format!(
        "{SCOPED_DEDUPED}
        SELECT to_char((\"timestamp\" AT TIME ZONE $3)::date, 'YYYY-MM-DD') AS date,
               coalesce(sum(coalesce(input_tokens,0) + coalesce(output_tokens,0)
                          + coalesce(cache_creation_tokens,0)
                          + coalesce(cache_read_tokens,0)), 0)::bigint AS total_tokens,
               coalesce(sum(input_tokens), 0)::bigint  AS input_tokens,
               coalesce(sum(output_tokens), 0)::bigint AS output_tokens,
               count(*)                                AS message_count,
               count(DISTINCT session_id)              AS session_count,
               count(DISTINCT extract(hour FROM \"timestamp\" AT TIME ZONE $3)) AS active_hours,
               sum(cost_usd)                           AS cost_usd
          FROM deduped
         WHERE \"timestamp\" IS NOT NULL
         GROUP BY 1 ORDER BY 1"
    );
    let rows: Vec<DailyRow> = bind_scope!(sqlx::query_as(&sql), scope, w)
        .fetch_all(pool)
        .await?;
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
async fn heatmap(pool: &PgPool, scope: &Scope, w: &Window) -> sqlx::Result<Vec<ActivityHeatmap>> {
    let sql = format!(
        "{SCOPED_DEDUPED}
        SELECT extract(hour FROM \"timestamp\" AT TIME ZONE $3)::int AS hour,
               extract(dow  FROM \"timestamp\" AT TIME ZONE $3)::int AS day,
               count(*)::bigint AS activity_count,
               coalesce(sum(coalesce(input_tokens,0) + coalesce(output_tokens,0)
                          + coalesce(cache_creation_tokens,0)
                          + coalesce(cache_read_tokens,0)), 0)::bigint AS tokens_used
          FROM deduped
         WHERE \"timestamp\" IS NOT NULL
         GROUP BY 1, 2 ORDER BY 2, 1"
    );
    let rows: Vec<(i32, i32, i64, i64)> = bind_scope!(sqlx::query_as(&sql), scope, w)
        .fetch_all(pool)
        .await?;
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

/// Which tool-name column a usage rollup groups by.
#[derive(Clone, Copy)]
pub enum ToolKind {
    Tool,
    Skill,
    Subagent,
}

impl ToolKind {
    fn column(self) -> &'static str {
        match self {
            Self::Tool => "u.tool_name",
            Self::Skill => "u.skill_name",
            Self::Subagent => "u.subagent_type",
        }
    }
}

/// Tool / skill / subagent usage with a *resolved* success rate.
///
/// Success is resolved by `LEFT JOIN`ing the outcome that reports on the
/// invocation, preferring it over the invocation's own flag (design D10):
/// `COALESCE(r.is_error, u.is_error, false)`. The join is scoped by
/// `(session_id, tool_use_id)` — **`tool_use_id` alone is not unique across the
/// archive and fans out** (design D11).
///
/// `avg_execution_time` is `None`: `duration_ms` is a per-message figure and
/// cannot be attributed to one invocation of several in that message.
async fn tools(
    pool: &PgPool,
    scope: &Scope,
    w: &Window,
    kind: ToolKind,
    limit: i64,
) -> sqlx::Result<Vec<ToolUsageStats>> {
    let col = kind.column();
    let sql = format!(
        "{SCOPED_DEDUPED}
        SELECT {col} AS name,
               count(*)::bigint AS uses,
               count(*) FILTER (WHERE NOT COALESCE(r.is_error, u.is_error, false))::bigint AS ok
          FROM message_tool_uses u
          JOIN deduped d ON d.id = u.message_ref
          LEFT JOIN message_tool_results r
                 ON r.session_id = u.session_id AND r.tool_use_id = u.tool_use_id
         WHERE {col} IS NOT NULL
         GROUP BY 1 ORDER BY 2 DESC LIMIT $6"
    );
    let rows: Vec<(String, i64, i64)> = bind_scope!(sqlx::query_as(&sql), scope, w)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(name, uses, ok)| ToolUsageStats {
            tool_name: name,
            usage_count: uses.max(0) as u32,
            success_rate: if uses > 0 {
                ok as f32 / uses as f32
            } else {
                0.0
            },
            avg_execution_time: None,
        })
        .collect())
}

async fn models(pool: &PgPool, scope: &Scope, w: &Window) -> sqlx::Result<Vec<ModelStats>> {
    let sql = format!(
        "{SCOPED_DEDUPED}
        SELECT model,
               count(*)::bigint,
               coalesce(sum(coalesce(input_tokens,0) + coalesce(output_tokens,0)
                          + coalesce(cache_creation_tokens,0)
                          + coalesce(cache_read_tokens,0)), 0)::bigint,
               coalesce(sum(input_tokens),0)::bigint,
               coalesce(sum(output_tokens),0)::bigint,
               coalesce(sum(cache_creation_tokens),0)::bigint,
               coalesce(sum(cache_read_tokens),0)::bigint,
               sum(cost_usd)
          FROM deduped WHERE model IS NOT NULL
         GROUP BY 1 ORDER BY 3 DESC"
    );
    let rows: Vec<ModelRow> = bind_scope!(sqlx::query_as(&sql), scope, w)
        .fetch_all(pool)
        .await?;
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

async fn providers(
    pool: &PgPool,
    scope: &Scope,
    w: &Window,
) -> sqlx::Result<Vec<ProviderUsageStats>> {
    let sql = format!(
        "{SCOPED_DEDUPED}
        SELECT provider,
               count(DISTINCT project_id)::bigint,
               count(DISTINCT session_id)::bigint,
               count(*)::bigint,
               coalesce(sum(coalesce(input_tokens,0) + coalesce(output_tokens,0)
                          + coalesce(cache_creation_tokens,0)
                          + coalesce(cache_read_tokens,0)), 0)::bigint
          FROM deduped GROUP BY 1 ORDER BY 4 DESC"
    );
    let rows: Vec<(String, i64, i64, i64, i64)> = bind_scope!(sqlx::query_as(&sql), scope, w)
        .fetch_all(pool)
        .await?;
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

/// First/last message and the span between them.
async fn date_range(pool: &PgPool, scope: &Scope, w: &Window) -> sqlx::Result<DateRange> {
    let sql = format!(
        "{SCOPED_DEDUPED}
        SELECT to_char(min(\"timestamp\") AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SSZ'),
               to_char(max(\"timestamp\") AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SSZ'),
               coalesce(date_part('day', max(\"timestamp\") - min(\"timestamp\")), 0)::int
          FROM deduped"
    );
    let r: (Option<String>, Option<String>, i32) = bind_scope!(sqlx::query_as(&sql), scope, w)
        .fetch_one(pool)
        .await?;
    Ok(DateRange {
        first_message: r.0,
        last_message: r.1,
        days_span: r.2.max(0) as u32,
    })
}

/// Summed session wall-clock, in minutes.
async fn session_duration_minutes(pool: &PgPool, scope: &Scope, w: &Window) -> sqlx::Result<i64> {
    let sql = format!(
        "{SCOPED_DEDUPED}
        SELECT coalesce(sum(EXTRACT(EPOCH FROM (last - first)) / 60), 0)::bigint
          FROM (SELECT session_id, min(\"timestamp\") AS first, max(\"timestamp\") AS last
                  FROM deduped GROUP BY session_id) s"
    );
    bind_scope!(sqlx::query_scalar(&sql), scope, w)
        .fetch_one(pool)
        .await
}

async fn top_projects(
    pool: &PgPool,
    scope: &Scope,
    w: &Window,
) -> sqlx::Result<Vec<ProjectRanking>> {
    let sql = format!(
        "{SCOPED_DEDUPED}
        SELECT coalesce(p.name, p.project_path, '(unknown)'),
               count(DISTINCT d.session_id)::bigint,
               count(*)::bigint,
               coalesce(sum(coalesce(d.input_tokens,0) + coalesce(d.output_tokens,0)
                          + coalesce(d.cache_creation_tokens,0)
                          + coalesce(d.cache_read_tokens,0)), 0)::bigint
          FROM deduped d LEFT JOIN projects p ON p.id = d.project_id
         GROUP BY 1 ORDER BY 4 DESC LIMIT 10"
    );
    let rows: Vec<(String, i64, i64, i64)> = bind_scope!(sqlx::query_as(&sql), scope, w)
        .fetch_all(pool)
        .await?;
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
    let scope = Scope::Global;
    let t = totals(pool, &scope, w).await?;
    let dist = distribution(&t);
    Ok(GlobalStatsSummary {
        total_projects: t.8.unwrap_or(0).max(0) as u32,
        total_sessions: t.7.max(0) as u32,
        total_messages: t.6.max(0) as u32,
        total_tokens: total_tokens(&dist),
        total_cost_usd: t.4,
        cost_reported_messages: t.5.max(0) as u32,
        total_session_duration_minutes: session_duration_minutes(pool, &scope, w).await?.max(0)
            as u64,
        date_range: date_range(pool, &scope, w).await?,
        token_distribution: dist,
        daily_stats: daily(pool, &scope, w).await?,
        activity_heatmap: heatmap(pool, &scope, w).await?,
        most_used_tools: tools(pool, &scope, w, ToolKind::Tool, 20).await?,
        most_used_skills: tools(pool, &scope, w, ToolKind::Skill, 20).await?,
        most_used_subagents: tools(pool, &scope, w, ToolKind::Subagent, 20).await?,
        provider_distribution: providers(pool, &scope, w).await?,
        model_distribution: models(pool, &scope, w).await?,
        top_projects: top_projects(pool, &scope, w).await?,
    })
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
    let scope = Scope::Paths(paths);
    let t = totals(pool, &scope, w).await?;
    let dist = distribution(&t);
    let sessions = t.7.max(0) as usize;
    let duration = session_duration_minutes(pool, &scope, w).await?.max(0) as u32;
    let heat = heatmap(pool, &scope, w).await?;
    let most_active_hour = heat
        .iter()
        .max_by_key(|h| h.activity_count)
        .map_or(0, |h| h.hour);
    let tokens = total_tokens(&dist);
    Ok(ProjectStatsSummary {
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
        most_used_tools: tools(pool, &scope, w, ToolKind::Tool, 20).await?,
        most_used_skills: tools(pool, &scope, w, ToolKind::Skill, 20).await?,
        most_used_subagents: tools(pool, &scope, w, ToolKind::Subagent, 20).await?,
        daily_stats: daily(pool, &scope, w).await?,
        activity_heatmap: heat,
        token_distribution: dist,
    })
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

    let scope = Scope::Session(session_pk);
    let t = totals(pool, &scope, w).await?;
    let dist = distribution(&t);
    let range = date_range(pool, &scope, w).await?;
    Ok(Some(SessionTokenStats {
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
        first_message_time: range.first_message.unwrap_or_default(),
        last_message_time: range.last_message.unwrap_or_default(),
        summary,
        most_used_tools: tools(pool, &scope, w, ToolKind::Tool, 20).await?,
    }))
}
