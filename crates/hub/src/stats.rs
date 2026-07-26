//! Aggregate statistics over the archive, computed from the `DuckDB` mirror.
//!
//! A port of the Postgres rollups this replaces, not a redesign: the same eight
//! aggregates, the same `history-core` return types, the same semantics. Every
//! rule the Postgres version documented is preserved here, and the comments
//! explaining *why* each rule exists travel with it — they were each paid for by
//! a bug the oracle gate caught.
//!
//! The Postgres implementation is **gone rather than kept as a fallback**
//! (design D4): two ports of eight rollups drift, and a cold mirror answering
//! the old 18 s query instead of a `503` is not a kindness. Its last job was to
//! be diffed against — 700 output rows byte-identical over 2,818,119 live
//! messages, 16.65 s vs 0.535 s, then a per-fixture differential in CI over
//! three scopes and four window shapes. That gate passed twice before this
//! deletion; from here the independent check is the desktop oracle (group 6).
//!
//! What changed in the port is where the deduplication happens. Postgres
//! computed `usage_row` per request inside the scope materialization (a window
//! function over the scoped rows, ~3.4 s of a 13.7 s summary). Here it is a
//! stored column the refresher fills once, so materialization is a filter and a
//! join. The one consequence is that `usage_row` is now global rather than
//! window-relative: measured over the live archive, 10 of 2,648,869 dedup groups
//! straddle a date boundary, so a windowed comparison can differ for those and
//! nothing else (design D3).
//!
//! Everything here is **synchronous** — `duckdb-rs` has no async API. Callers
//! must run these on `spawn_blocking` with their own cloned connection, which is
//! also what keeps the per-request `stats_scope` temp table private to the
//! request that built it (`stats_api::with_mirror` is the only caller, and it
//! does exactly that).

use chrono::{DateTime, NaiveDate, NaiveDateTime};
use duckdb::types::Value;
use duckdb::{params_from_iter, Connection};
use history_core::models::{
    ActivityHeatmap, DailyStats, DateRange, GlobalStatsSummary, ModelStats, ProjectRanking,
    ProjectStatsSummary, ProviderUsageStats, SessionTokenStats, TokenDistribution, ToolUsageStats,
};

use crate::tz_spans;

/// Time window and the timezone its day/hour buckets are expressed in.
///
/// Bucketing is done server-side rather than shipping rows for the client to
/// re-bucket: "what hours do I work" is meaningless in UTC for a user in
/// `Europe/Rome`, and hour buckets re-bucket exactly only to whole-hour offsets.
///
/// `tz` is a parsed IANA zone rather than a string, so an unknown zone is
/// unrepresentable past the API boundary and no rollup has to decide what to do
/// with one. The alternative — carrying the name and validating its *shape* —
/// let `Not/A_Zone` through to the query layer.
#[derive(Debug, Clone)]
pub struct Window {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub tz: chrono_tz::Tz,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            from: None,
            to: None,
            tz: chrono_tz::UTC,
        }
    }
}

/// What the statistics cover.
#[derive(Debug, Clone)]
pub enum Scope {
    /// The whole archive.
    Global,
    /// One project identity, expanded to its member paths (fingerprinted rows
    /// plus manual aliases) by [`resolve_identity_paths`], so a repository that
    /// moved or was cloned reports as one project (design D6).
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

/// Sum of the four token columns, as SQL.
///
/// The `::BIGINT` cast is load-bearing, not cosmetic: `DuckDB` widens `sum()`
/// over `BIGINT` to `HUGEINT`, which does not decode into an `i64` and fails at
/// the row-read rather than at the query. Every `sum` of a token column below
/// carries the same cast for the same reason.
const TOKEN_SUM: &str = "coalesce(sum(coalesce(input_tokens,0) + coalesce(output_tokens,0)
                       + coalesce(cache_creation_tokens,0)
                       + coalesce(cache_read_tokens,0)), 0)::BIGINT";

/// The archive's UTC date range, used only to bound the timezone-span walk.
/// Two indexed lookups; the walk itself is over hours, so bounding it matters.
fn data_range(conn: &Connection) -> duckdb::Result<(NaiveDateTime, NaiveDateTime)> {
    // Read as text and parse here rather than enabling duckdb's `chrono`
    // feature for two values: the cast's output shape is already pinned by
    // `duckdb_capability_test`, and the whole point of D7 is to keep the
    // timestamp path free of type-mapping surprises.
    let (lo, hi) = conn.query_row(
        "SELECT substr(CAST(min(ts_utc) AS VARCHAR), 1, 19),
                substr(CAST(max(ts_utc) AS VARCHAR), 1, 19) FROM messages",
        [],
        |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
            ))
        },
    )?;
    // An empty mirror has no range; any single instant works, because the spans
    // built from it still cover all time.
    let fallback = DateTime::from_timestamp(0, 0)
        .expect("epoch is representable")
        .naive_utc();
    let parse = |s: Option<String>| {
        s.and_then(|s| NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").ok())
            .unwrap_or(fallback)
    };
    Ok((parse(lo), parse(hi)))
}

/// Materialize the requested zone's UTC-offset spans so `DuckDB` can shift each
/// row into local time (design D7).
///
/// This is what stands in for `AT TIME ZONE`, which needs the `icu` extension
/// the bundled build does not have. The spans are half-open, contiguous, and
/// cover all representable time, so the join below is an inner join that cannot
/// drop a row.
fn materialize_tz_spans(conn: &Connection, w: &Window) -> duckdb::Result<()> {
    let (lo, hi) = data_range(conn)?;
    let spans = tz_spans::spans(w.tz, lo, hi);
    conn.execute_batch(
        "CREATE OR REPLACE TEMP TABLE tz_spans
           (from_utc TIMESTAMP, to_utc TIMESTAMP, offset_secs BIGINT);",
    )?;
    let mut ins = conn
        .prepare("INSERT INTO tz_spans VALUES (CAST(? AS TIMESTAMP), CAST(? AS TIMESTAMP), ?)")?;
    for s in &spans {
        ins.execute(duckdb::params![
            s.from.format("%Y-%m-%d %H:%M:%S").to_string(),
            s.to.format("%Y-%m-%d %H:%M:%S").to_string(),
            i64::from(s.offset_secs),
        ])?;
    }
    Ok(())
}

/// Materialize the scoped messages ONCE per request.
///
/// **Why a table and not a CTE.** A summary runs ~8 aggregates; re-deriving the
/// scope inside each one pays the join eight times. Materializing costs 0.066 s
/// on the live archive and the eight aggregates then read a narrow local table.
///
/// **Why `TEMP`.** The table is scoped to the connection that created it, and
/// each request holds its own cloned connection, so two concurrent summaries
/// over different projects cannot see each other's scope. `CREATE OR REPLACE`
/// rather than a unique name because the connection is the isolation boundary —
/// there is never more than one `stats_scope` per connection.
///
/// **Dedup applies to usage, NOT to tool invocations.** A single assistant
/// response is streamed across several records sharing one `message.id` and one
/// `usage` block, but each record carries DIFFERENT content blocks — so its tool
/// calls are distinct events that all really happened. The table therefore keeps
/// every scoped row and *flags* it, rather than deleting duplicates. Caught by
/// the oracle gate: tool counts were ~4x low before this.
fn materialize_scope(conn: &Connection, scope: &Scope, w: &Window) -> duckdb::Result<()> {
    materialize_tz_spans(conn, w)?;

    // Two predicate groups, because they apply at different depths: scope
    // narrows the base tables, while the window compares against `ts_local`,
    // which only exists once the offset span has been joined on. Binds are
    // pushed in the order their `?` appears in the assembled SQL — scope first
    // (inner query), window second (outer).
    let mut scope_preds: Vec<&str> = Vec::new();
    let mut window_preds: Vec<&str> = Vec::new();
    let mut binds: Vec<Value> = Vec::new();

    if let Some(paths) = scope.paths() {
        // Bound through a table rather than interpolated into an IN list:
        // project paths are user data, and an identity can expand to many of
        // them. `DuckDB` has no array bind that survives `params_from_iter`.
        conn.execute_batch("CREATE OR REPLACE TEMP TABLE scope_paths (path VARCHAR);")?;
        let mut ins = conn.prepare("INSERT INTO scope_paths VALUES (?)")?;
        for p in paths {
            ins.execute([p])?;
        }
        scope_preds.push("p.project_path IN (SELECT path FROM scope_paths)");
    }
    if let Some(session_id) = scope.session() {
        scope_preds.push("m.session_id = ?");
        binds.push(Value::BigInt(session_id));
    }
    // Bucketed in the caller's timezone before the date comparison, so "since
    // the 1st" means the 1st where the user lives (see `Window`).
    if let Some(from) = w.from {
        window_preds.push("CAST(ts_local AS DATE) >= CAST(? AS DATE)");
        binds.push(Value::Text(from.to_string()));
    }
    if let Some(to) = w.to {
        window_preds.push("CAST(ts_local AS DATE) <= CAST(? AS DATE)");
        binds.push(Value::Text(to.to_string()));
    }

    // `ts_local` is derived in an inner query so the window predicate can name
    // it. Only *bucketing* uses it: the date-range strings are explicitly UTC
    // and the active-time gaps are absolute durations, so both keep `ts_utc`.
    //
    // The span join is a LEFT JOIN even though the spans cover all
    // representable time, because a message with a NULL timestamp has nothing
    // to match. Such rows must survive — they still count toward
    // `count(DISTINCT session_id)` and the tool rollups — and every rollup that
    // buckets already filters on `ts_utc IS NOT NULL`.
    let sql = format!(
        r"CREATE OR REPLACE TEMP TABLE stats_scope AS
          SELECT * FROM (
            SELECT m.id, m.session_id, m.provider, m.model, m.ts_utc,
                   m.ts_utc + to_seconds(z.offset_secs) AS ts_local,
                   m.input_tokens, m.output_tokens, m.cache_creation_tokens,
                   m.cache_read_tokens, m.cost_usd, s.project_id,
                   m.usage_row, m.conversational
              FROM messages m
              JOIN sessions s ON s.id = m.session_id
              LEFT JOIN projects p ON p.id = s.project_id
              LEFT JOIN tz_spans z
                     ON m.ts_utc >= z.from_utc AND m.ts_utc < z.to_utc
              {scope_where}
          ) {window_where}",
        scope_where = clause(&scope_preds),
        window_where = clause(&window_preds),
    );
    conn.execute(&sql, params_from_iter(binds))?;
    Ok(())
}

fn clause(preds: &[&str]) -> String {
    if preds.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", preds.join(" AND "))
    }
}

/// Token/cost totals, message and session counts, cost coverage, date range and
/// active session time — all from one scan, because each extra statement would
/// be another pass over the scope.
struct Totals {
    input: Option<i64>,
    output: Option<i64>,
    cache_creation: Option<i64>,
    cache_read: Option<i64>,
    cost_usd: Option<f64>,
    cost_reported: i64,
    messages: i64,
    sessions: i64,
    projects: i64,
    first: Option<String>,
    last: Option<String>,
    days_span: i32,
    active_minutes: i64,
}

fn totals(conn: &Connection) -> duckdb::Result<Totals> {
    conn.query_row(
        r#"
        SELECT sum(input_tokens) FILTER (WHERE usage_row)::BIGINT,
               sum(output_tokens) FILTER (WHERE usage_row)::BIGINT,
               sum(cache_creation_tokens) FILTER (WHERE usage_row)::BIGINT,
               sum(cache_read_tokens) FILTER (WHERE usage_row)::BIGINT,
               -- NOT coalesced to 0: NULL means 'nobody reported cost', which
               -- is a different statement from 'it was free'.
               sum(cost_usd) FILTER (WHERE usage_row),
               count(*) FILTER (WHERE cost_usd IS NOT NULL AND usage_row),
               count(*) FILTER (WHERE usage_row AND conversational),
               count(DISTINCT session_id),
               count(DISTINCT project_id),
               -- `strftime` is unavailable once ICU is in the picture and the
               -- column is not TIMESTAMPTZ anyway, so the RFC3339-ish shape the
               -- frontend expects is assembled from the canonical cast:
               -- 'YYYY-MM-DD HH:MM:SS[.ffffff]' -> 'YYYY-MM-DDTHH:MM:SSZ'.
               replace(substr(CAST(min(ts_utc) AS VARCHAR), 1, 19), ' ', 'T') || 'Z',
               replace(substr(CAST(max(ts_utc) AS VARCHAR), 1, 19), ' ', 'T') || 'Z',
               coalesce(date_part('day', max(ts_utc) - min(ts_utc)), 0)::INT,
               -- ACTIVE time, not wall-clock span.
               --
               -- Summing (last - first) per session counts every idle hour: a
               -- session resumed across days contributes those days in full,
               -- which rendered as "950 days of session time" inside a 30-day
               -- window — arithmetically defensible and useless. Instead sum
               -- the gaps between consecutive messages and ignore any gap
               -- longer than the idle threshold, so a resumed session
               -- contributes only the stretches actually worked.
               --
               -- This DIVERGES from the desktop oracle, which uses the raw
               -- span. Deliberate, and recorded as divergence #4.
               (SELECT coalesce(sum(gap) / 60.0, 0)::BIGINT
                  FROM (SELECT EXTRACT(EPOCH FROM (ts_utc - lag(ts_utc)
                                 OVER (PARTITION BY session_id ORDER BY ts_utc))) AS gap
                          FROM stats_scope WHERE ts_utc IS NOT NULL) g
                 WHERE gap IS NOT NULL AND gap > 0 AND gap <= 1800)
          FROM stats_scope"#,
        [],
        |r| {
            Ok(Totals {
                input: r.get(0)?,
                output: r.get(1)?,
                cache_creation: r.get(2)?,
                cache_read: r.get(3)?,
                cost_usd: r.get(4)?,
                cost_reported: r.get(5)?,
                messages: r.get(6)?,
                sessions: r.get(7)?,
                projects: r.get(8)?,
                first: r.get(9)?,
                last: r.get(10)?,
                days_span: r.get(11)?,
                active_minutes: r.get(12)?,
            })
        },
    )
}

/// Per-day buckets in the caller's timezone.
fn daily(conn: &Connection) -> duckdb::Result<Vec<DailyStats>> {
    let sql = format!(
        r"SELECT CAST(CAST(ts_local AS DATE) AS VARCHAR),
                  {TOKEN_SUM},
                  coalesce(sum(input_tokens), 0)::BIGINT,
                  coalesce(sum(output_tokens), 0)::BIGINT,
                  count(*) FILTER (WHERE conversational),
                  count(DISTINCT session_id),
                  count(DISTINCT extract(hour FROM ts_local)),
                  sum(cost_usd)
             FROM stats_scope
            WHERE ts_utc IS NOT NULL AND usage_row
            GROUP BY 1 ORDER BY 1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| {
        Ok(DailyStats {
            date: r.get(0)?,
            total_tokens: r.get::<_, i64>(1)?.max(0) as u64,
            input_tokens: r.get::<_, i64>(2)?.max(0) as u64,
            output_tokens: r.get::<_, i64>(3)?.max(0) as u64,
            message_count: r.get::<_, i64>(4)?.max(0) as usize,
            session_count: r.get::<_, i64>(5)?.max(0) as usize,
            active_hours: r.get::<_, i64>(6)?.max(0) as usize,
            cost_usd: r.get(7)?,
        })
    })?;
    rows.collect()
}

/// Hour-of-day × day-of-week activity.
fn heatmap(conn: &Connection) -> duckdb::Result<Vec<ActivityHeatmap>> {
    let sql = format!(
        r"SELECT extract(hour FROM ts_local)::INT,
                  extract(dow  FROM ts_local)::INT,
                  count(*)::BIGINT,
                  {TOKEN_SUM}
             FROM stats_scope
            WHERE ts_utc IS NOT NULL AND usage_row
            GROUP BY 1, 2 ORDER BY 2, 1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| {
        Ok(ActivityHeatmap {
            hour: r.get::<_, i32>(0)?.clamp(0, 23) as u8,
            day: r.get::<_, i32>(1)?.clamp(0, 6) as u8,
            activity_count: r.get::<_, i64>(2)?.max(0) as u32,
            tokens_used: r.get::<_, i64>(3)?.max(0) as u64,
        })
    })?;
    rows.collect()
}

/// Which usage collection a row belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolKind {
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

type ToolTriple = (
    Vec<ToolUsageStats>,
    Vec<ToolUsageStats>,
    Vec<ToolUsageStats>,
);

/// All three usage collections — tools, skills, subagents — in ONE pass.
///
/// The `VALUES` lateral fans each invocation out into whichever collections it
/// belongs to, so the expensive join to the scope and to the outcomes is paid
/// once instead of three times.
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
fn all_tool_usage(conn: &Connection, limit: usize) -> duckdb::Result<ToolTriple> {
    let mut stmt = conn.prepare(
        r"
        SELECT k.kind, k.name,
               count(*)::BIGINT,
               count(*) FILTER (WHERE NOT COALESCE(r.is_error, u.is_error, false))::BIGINT
          FROM message_tool_uses u
          JOIN stats_scope d ON d.id = u.message_ref
          -- Outcomes are collapsed to ONE row per (session, tool_use_id)
          -- before joining. Without this the LEFT JOIN fans out whenever an
          -- invocation has more than one recorded outcome — which happens when
          -- a tool_result record is itself stored more than once — and the
          -- invocation is counted twice. Caught by the oracle gate: it inflated
          -- Bash by exactly 1 in a session with 83 real invocations.
          -- `bool_or`: errored if ANY recorded outcome says so.
          LEFT JOIN (SELECT session_id, tool_use_id, bool_or(is_error) AS is_error
                       FROM message_tool_results GROUP BY 1, 2) r
                 ON r.session_id = u.session_id AND r.tool_use_id = u.tool_use_id
          CROSS JOIN LATERAL (VALUES ('tool', u.tool_name),
                                     ('skill', u.skill_name),
                                     ('subagent', u.subagent_type)) AS k(kind, name)
         WHERE k.name IS NOT NULL
         GROUP BY 1, 2
         -- The name tiebreak is an addition to the Postgres original, which
         -- ordered on count alone: with ties, which entries survive the top-N
         -- cut was then a coin flip between two identical requests.
         ORDER BY 3 DESC, 2
        ",
    )?;
    let rows: Vec<(String, String, i64, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<duckdb::Result<Vec<_>>>()?;

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

fn models(conn: &Connection) -> duckdb::Result<Vec<ModelStats>> {
    let sql = format!(
        "SELECT model, count(*) FILTER (WHERE conversational)::BIGINT, {TOKEN_SUM},
                coalesce(sum(input_tokens),0)::BIGINT,
                coalesce(sum(output_tokens),0)::BIGINT,
                coalesce(sum(cache_creation_tokens),0)::BIGINT,
                coalesce(sum(cache_read_tokens),0)::BIGINT,
                sum(cost_usd)
           FROM stats_scope WHERE model IS NOT NULL AND usage_row
          GROUP BY 1 ORDER BY 3 DESC, 1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| {
        Ok(ModelStats {
            model_name: r.get(0)?,
            message_count: r.get::<_, i64>(1)?.max(0) as u32,
            token_count: r.get::<_, i64>(2)?.max(0) as u64,
            input_tokens: r.get::<_, i64>(3)?.max(0) as u64,
            output_tokens: r.get::<_, i64>(4)?.max(0) as u64,
            cache_creation_tokens: r.get::<_, i64>(5)?.max(0) as u64,
            cache_read_tokens: r.get::<_, i64>(6)?.max(0) as u64,
            // No provider reports reasoning tokens through `TokenUsage`, so the
            // oracle's value is 0 too — this is parity, not a gap.
            reasoning_tokens: 0,
            cost_usd: r.get(7)?,
        })
    })?;
    rows.collect()
}

fn providers(conn: &Connection) -> duckdb::Result<Vec<ProviderUsageStats>> {
    let sql = format!(
        "SELECT provider,
                count(DISTINCT project_id)::BIGINT,
                count(DISTINCT session_id)::BIGINT,
                -- Conversational, matching `total_messages`. Counting every
                -- stored row here made one project report more messages than
                -- the whole archive total, on the same screen.
                count(*) FILTER (WHERE conversational)::BIGINT,
                {TOKEN_SUM}
           FROM stats_scope WHERE usage_row GROUP BY 1 ORDER BY 4 DESC, 1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| {
        Ok(ProviderUsageStats {
            provider_id: r.get(0)?,
            projects: r.get::<_, i64>(1)?.max(0) as u32,
            sessions: r.get::<_, i64>(2)?.max(0) as u32,
            messages: r.get::<_, i64>(3)?.max(0) as u32,
            tokens: r.get::<_, i64>(4)?.max(0) as u64,
        })
    })?;
    rows.collect()
}

fn top_projects(conn: &Connection) -> duckdb::Result<Vec<ProjectRanking>> {
    let sql = format!(
        "SELECT coalesce(p.project_name, p.project_path, '(unknown)'),
                count(DISTINCT d.session_id)::BIGINT,
                count(*) FILTER (WHERE d.conversational)::BIGINT,
                {TOKEN_SUM}
           FROM stats_scope d LEFT JOIN projects p ON p.id = d.project_id
          WHERE d.usage_row
          GROUP BY 1 ORDER BY 4 DESC, 1 LIMIT 10"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| {
        Ok(ProjectRanking {
            project_name: r.get(0)?,
            sessions: r.get::<_, i64>(1)?.max(0) as u32,
            messages: r.get::<_, i64>(2)?.max(0) as u32,
            tokens: r.get::<_, i64>(3)?.max(0) as u64,
        })
    })?;
    rows.collect()
}

fn distribution(t: &Totals) -> TokenDistribution {
    TokenDistribution {
        input: t.input.unwrap_or(0).max(0) as u64,
        output: t.output.unwrap_or(0).max(0) as u64,
        cache_creation: t.cache_creation.unwrap_or(0).max(0) as u64,
        cache_read: t.cache_read.unwrap_or(0).max(0) as u64,
        reasoning: 0,
    }
}

fn total_tokens(d: &TokenDistribution) -> u64 {
    d.input + d.output + d.cache_creation + d.cache_read
}

/// Archive-wide statistics.
pub fn global(conn: &Connection, w: &Window) -> duckdb::Result<GlobalStatsSummary> {
    materialize_scope(conn, &Scope::Global, w)?;

    let t = totals(conn)?;
    let dist = distribution(&t);
    let tool_usage = all_tool_usage(conn, 20)?;
    Ok(GlobalStatsSummary {
        total_projects: t.projects.max(0) as u32,
        total_sessions: t.sessions.max(0) as u32,
        total_messages: t.messages.max(0) as u32,
        total_tokens: total_tokens(&dist),
        total_cost_usd: t.cost_usd,
        cost_reported_messages: t.cost_reported.max(0) as u32,
        total_session_duration_minutes: t.active_minutes.max(0) as u64,
        date_range: DateRange {
            first_message: t.first.clone(),
            last_message: t.last.clone(),
            days_span: t.days_span.max(0) as u32,
        },
        token_distribution: dist,
        daily_stats: daily(conn)?,
        activity_heatmap: heatmap(conn)?,
        most_used_tools: tool_usage.0,
        most_used_skills: tool_usage.1,
        most_used_subagents: tool_usage.2,
        provider_distribution: providers(conn)?,
        model_distribution: models(conn)?,
        top_projects: top_projects(conn)?,
    })
}

/// Statistics for one project identity, folded across its member paths.
///
/// `paths` comes from the identity expansion. An empty slice (unknown identity)
/// correctly matches nothing, which the caller turns into a `404` rather than an
/// empty-but-successful body.
pub fn project(
    conn: &Connection,
    name: String,
    paths: Vec<String>,
    w: &Window,
) -> duckdb::Result<ProjectStatsSummary> {
    materialize_scope(conn, &Scope::Paths(paths), w)?;

    let t = totals(conn)?;
    let dist = distribution(&t);
    let sessions = t.sessions.max(0) as usize;
    let duration = t.active_minutes.max(0) as u32;
    let heat = heatmap(conn)?;
    let most_active_hour = heat
        .iter()
        .max_by_key(|h| h.activity_count)
        .map_or(0, |h| h.hour);
    let tokens = total_tokens(&dist);
    let tool_usage = all_tool_usage(conn, 20)?;
    Ok(ProjectStatsSummary {
        project_name: name,
        total_sessions: sessions,
        total_messages: t.messages.max(0) as usize,
        total_tokens: tokens,
        total_cost_usd: t.cost_usd,
        cost_reported_messages: t.cost_reported.max(0) as u32,
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
        daily_stats: daily(conn)?,
        activity_heatmap: heat,
        token_distribution: dist,
        // Same rollup as the global scope: `models` reads the materialized
        // `stats_scope`, which here already carries only this identity's rows.
        model_distribution: models(conn)?,
    })
}

/// Statistics for one session. `None` when the session does not exist.
///
/// The session's own metadata is read from the mirror too, not from Postgres:
/// otherwise this endpoint alone would still need the database at serve time,
/// and "statistics survive a Postgres outage" would quietly hold for two of the
/// three scopes.
pub fn session(
    conn: &Connection,
    session_pk: i64,
    w: &Window,
) -> duckdb::Result<Option<SessionTokenStats>> {
    let meta = conn
        .query_row(
            r"
            SELECT s.session_id, s.summary, coalesce(p.project_name, p.project_path)
              FROM sessions s LEFT JOIN projects p ON p.id = s.project_id
             WHERE s.id = ?
            ",
            [session_pk],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .ok();
    let Some((provider_session_id, summary, project_name)) = meta else {
        return Ok(None);
    };

    materialize_scope(conn, &Scope::Session(session_pk), w)?;
    let t = totals(conn)?;
    let dist = distribution(&t);

    Ok(Some(SessionTokenStats {
        session_id: provider_session_id.unwrap_or_default(),
        project_name: project_name.unwrap_or_else(|| "(unknown)".to_string()),
        total_input_tokens: dist.input,
        total_output_tokens: dist.output,
        total_cache_creation_tokens: dist.cache_creation,
        total_cache_read_tokens: dist.cache_read,
        total_reasoning_tokens: 0,
        total_tokens: total_tokens(&dist),
        total_cost_usd: t.cost_usd,
        message_count: t.messages.max(0) as usize,
        first_message_time: t.first.clone().unwrap_or_default(),
        last_message_time: t.last.clone().unwrap_or_default(),
        summary,
        most_used_tools: all_tool_usage(conn, 20)?.0,
    }))
}

/// What a session reference resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionRef {
    /// Exactly one archived session.
    Found(i64),
    /// Nothing in the mirror answers to it.
    Absent,
    /// A provider session id shared by several archived sessions — the same
    /// session synced from more than one machine. Reported rather than resolved
    /// arbitrarily, matching the browse endpoint: silently picking the lowest id
    /// would report one machine's copy of the session as if it were the whole
    /// thing.
    Ambiguous(Vec<i64>),
}

/// Resolve a session reference — numeric row id or provider session id —
/// against the mirror (Gitea #26).
///
/// Same acceptance rule as the browse-side `resolve_session_ref`, but without
/// touching Postgres, so an unknown UUID is a `404` rather than a parse `400`,
/// and a stats request stays answerable while Postgres is away.
///
/// Unlike the browse version, a numeric reference is *checked* rather than
/// trusted: there is a mirror row to check it against right here, and the
/// alternative is discovering the absence further down as an empty rollup.
pub fn resolve_session_ref(conn: &Connection, reference: &str) -> duckdb::Result<SessionRef> {
    if let Ok(id) = reference.parse::<i64>() {
        let found: Option<i64> = conn
            .query_row("SELECT id FROM sessions WHERE id = ?", [id], |r| r.get(0))
            .map(Some)
            .or_else(none_if_absent)?;
        return Ok(found.map_or(SessionRef::Absent, SessionRef::Found));
    }
    let mut stmt = conn.prepare("SELECT id FROM sessions WHERE session_id = ? ORDER BY id")?;
    let ids: Vec<i64> = stmt
        .query_map([reference], |r| r.get(0))?
        .collect::<duckdb::Result<_>>()?;
    Ok(match ids.as_slice() {
        [] => SessionRef::Absent,
        [pk] => SessionRef::Found(*pk),
        _ => SessionRef::Ambiguous(ids),
    })
}

/// Expand a project identity key into its member paths, from the mirror.
///
/// Mirrors `identity_filter::resolve_project_scope`: fingerprinted project rows
/// carrying the key, plus manually aliased (typically moved-away) paths.
/// `include_worktrees = false` drops a path only when EVERY project row binding
/// it to the identity is a linked worktree — a path that is a main checkout
/// anywhere stays in, because inclusion is the safe default. Aliased paths are
/// dead paths, never worktrees, and are always included.
pub fn resolve_identity_paths(
    conn: &Connection,
    identity_key: &str,
    include_worktrees: bool,
) -> duckdb::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        r"
        SELECT project_path FROM projects
         WHERE identity_key = ?
         GROUP BY project_path
        HAVING ? OR NOT bool_and(coalesce(git_worktree, false))
        UNION
        SELECT project_path FROM project_identity_aliases
         WHERE identity_key = ?
        ",
    )?;
    let rows = stmt.query_map(
        duckdb::params![identity_key, include_worktrees, identity_key],
        |r| r.get::<_, Option<String>>(0),
    )?;
    Ok(rows
        .collect::<duckdb::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect())
}

/// A human label for the identity: its most common project name, else the
/// shortest member path (the main checkout, rather than a worktree).
pub fn identity_display_name(
    conn: &Connection,
    identity_key: &str,
    paths: &[String],
) -> duckdb::Result<String> {
    let name: Option<String> = conn
        .query_row(
            r"
            SELECT project_name FROM projects
             WHERE identity_key = ? AND project_name IS NOT NULL
             GROUP BY project_name ORDER BY count(*) DESC, project_name LIMIT 1
            ",
            [identity_key],
            |r| r.get(0),
        )
        .ok()
        .flatten();

    Ok(name.unwrap_or_else(|| {
        paths
            .iter()
            .min_by_key(|p| p.len())
            .cloned()
            .unwrap_or_else(|| identity_key.to_string())
    }))
}

/// `QueryReturnedNoRows` means "absent", which is a `None`, not an error.
/// Anything else is a real failure and stays one.
fn none_if_absent<T>(e: duckdb::Error) -> duckdb::Result<Option<T>> {
    match e {
        duckdb::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    }
}
