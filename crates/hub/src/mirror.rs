//! The statistics read model: a local `DuckDB` file mirroring the columns the
//! analytics rollups read.
//!
//! Postgres remains the system of record. This file is derived state — it can
//! be deleted and rebuilt at any time, and it is never an authority for
//! anything but statistics.
//!
//! Why it exists: the same eight rollups take ~13.7 s against Postgres and
//! ~2.2 s against `DuckDB` over identical data, and ~0.4 s once the deduplication
//! marker is precomputed here instead of per request. Full measurements in
//! `openspec/changes/hub-stats-duckdb-mirror/`.
//!
//! This module owns the file and knows nothing about statistics.
//!
//! Two properties are load-bearing and easy to get wrong:
//!
//! 1. **Refresh re-scans behind the watermark.** Row ids are assigned at insert
//!    but transactions commit out of order, so a refresh that fetched only
//!    `id > max_id` would step over rows whose transaction had not yet
//!    committed — permanently, and silently. Every refresh therefore re-reads
//!    an overlap window, and inserts are idempotent so re-reading is free.
//! 2. **Refresh handles inserts, not updates.** `usage_row` is stable under
//!    append because ids are monotonic, so an arriving row can never become the
//!    minimum of a group that already exists. It is *not* stable under a
//!    Postgres-side `UPDATE` of a mirrored column — `hub backfill-analytics`
//!    rewrites `message_id` on existing rows — which is why `rebuild` exists
//!    and why the runbook pairs the two.

use crate::config::MirrorConfig;
use chrono::{DateTime, Utc};
use duckdb::Connection;
use sqlx::PgPool;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Whether the mirror can answer questions yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirrorState {
    /// No usable mirror; a build is running. `/v1/stats/*` answers 503.
    Warming,
    Ready {
        /// Highest `messages.id` mirrored.
        max_id: i64,
        refreshed_at: DateTime<Utc>,
    },
}

impl MirrorState {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    /// Seconds since the last successful refresh, when ready.
    pub fn age_secs(&self) -> Option<i64> {
        match self {
            Self::Ready { refreshed_at, .. } => {
                Some((Utc::now() - *refreshed_at).num_seconds().max(0))
            }
            Self::Warming => None,
        }
    }
}

/// What one refresh moved.
#[derive(Debug, Default, Clone, Copy)]
pub struct RefreshReport {
    pub messages_scanned: usize,
    pub messages_inserted: usize,
    pub tool_uses_inserted: usize,
    pub tool_results_inserted: usize,
    pub max_id: i64,
}

/// How many messages to pull from Postgres per round trip.
const FETCH_CHUNK: i64 = 50_000;

pub struct Mirror {
    /// Single owning connection. `DuckDB` permits one read-write handle to a
    /// file per process; concurrent readers are served by cloning from this.
    conn: Arc<Mutex<Connection>>,
    path: PathBuf,
    cfg: MirrorConfig,
}

impl Mirror {
    /// Open (or create) the mirror, applying the resource caps.
    ///
    /// An unopenable file is moved aside under a timestamped name rather than
    /// deleted — same rule as the binary-swap playbook — and a fresh one is
    /// created in its place.
    pub fn open_or_create(cfg: &MirrorConfig) -> anyhow::Result<Self> {
        let path = cfg.resolved_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = match Self::open_at(&path, cfg) {
            Ok(c) => c,
            Err(e) => {
                let aside = aside_path(&path);
                tracing::error!(
                    error = %e,
                    moved_to = %aside.display(),
                    "stats mirror unreadable; moving aside and rebuilding"
                );
                let _ = std::fs::rename(&path, &aside);
                Self::open_at(&path, cfg)?
            }
        };

        let mirror = Self {
            conn: Arc::new(Mutex::new(conn)),
            path,
            cfg: cfg.clone(),
        };
        mirror.create_schema()?;
        Ok(mirror)
    }

    fn open_at(path: &Path, cfg: &MirrorConfig) -> anyhow::Result<Connection> {
        let conn = Connection::open(path)?;
        // Caps, not tuning: m4m also runs the sync daemon and the distiller,
        // so statistics must not be able to take the box.
        conn.execute_batch(&format!(
            "SET memory_limit='{}'; SET threads={};",
            cfg.memory_limit, cfg.threads
        ))?;
        // NB: no `LOAD icu`. IANA timezone support is in DuckDB core as of
        // 1.5.5; the extension is absent from the bundled build and loading it
        // would attempt a runtime download (design D7).
        Ok(conn)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A second handle to the same database, for concurrent reads.
    pub fn connection(&self) -> anyhow::Result<Connection> {
        let guard = self.conn.lock().expect("mirror connection poisoned");
        Ok(guard.try_clone()?)
    }

    fn create_schema(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("mirror connection poisoned");
        conn.execute_batch(SCHEMA)?;
        Ok(())
    }

    /// Current state, read from the mirror itself so it survives restarts.
    pub fn state(&self) -> MirrorState {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return MirrorState::Warming,
        };
        let refreshed_at: Option<String> = conn
            .query_row(
                "SELECT v FROM mirror_meta WHERE k = 'refreshed_at'",
                [],
                |r| r.get(0),
            )
            .ok();
        let Some(refreshed_at) = refreshed_at else {
            return MirrorState::Warming;
        };
        let Ok(refreshed_at) = refreshed_at.parse::<DateTime<Utc>>() else {
            return MirrorState::Warming;
        };
        let max_id: i64 = conn
            .query_row("SELECT coalesce(max(id), 0) FROM messages", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        MirrorState::Ready {
            max_id,
            refreshed_at,
        }
    }

    /// Highest mirrored message id, or 0 when empty.
    fn watermark(&self) -> i64 {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return 0,
        };
        conn.query_row("SELECT coalesce(max(id), 0) FROM messages", [], |r| {
            r.get(0)
        })
        .unwrap_or(0)
    }

    /// Pull everything new from Postgres and fold it in.
    ///
    /// Idempotent: safe to run repeatedly, safe to interrupt. Fetching starts
    /// an overlap window *behind* the watermark so rows that committed late are
    /// picked up rather than skipped forever.
    pub async fn refresh(&self, pg: &PgPool) -> anyhow::Result<RefreshReport> {
        let watermark = self.watermark();
        let from_id = (watermark - self.cfg.overlap_rows).max(0);
        let mut report = RefreshReport {
            max_id: watermark,
            ..Default::default()
        };

        self.refresh_projects(pg).await?;
        self.refresh_sessions(pg).await?;

        // Messages, chunked so one refresh cannot pin an unbounded result set.
        let mut cursor = from_id;
        loop {
            let rows = sqlx::query_as::<_, MessageRow>(
                r#"SELECT m.id, m.session_id, m.message_id, m.uuid::text, m.provider,
                          m.model, m.role, m."timestamp", m.input_tokens, m.output_tokens,
                          m.cache_creation_tokens, m.cache_read_tokens, m.cost_usd
                     FROM messages m
                    WHERE m.id > $1
                    ORDER BY m.id
                    LIMIT $2"#,
            )
            .bind(cursor)
            .bind(FETCH_CHUNK)
            .fetch_all(pg)
            .await?;

            if rows.is_empty() {
                break;
            }
            cursor = rows.last().map(|r| r.id).unwrap_or(cursor);
            report.messages_scanned += rows.len();
            report.messages_inserted += self.insert_messages(&rows)?;
            report.max_id = report.max_id.max(cursor);
        }

        report.tool_uses_inserted = self.refresh_tool_uses(pg, from_id).await?;
        report.tool_results_inserted = self.refresh_tool_results(pg, from_id).await?;

        self.compute_derived()?;
        self.stamp_refreshed()?;
        Ok(report)
    }

    /// Mark `usage_row` on rows that do not have it yet.
    ///
    /// `usage_row` is true when the row holds the lowest id within its logical
    /// message. Only rows inserted since the last pass are considered: an
    /// already-marked row cannot change, because a later arrival always carries
    /// a higher id and so can never displace the group minimum (design D2).
    fn compute_derived(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("mirror connection poisoned");
        conn.execute_batch(
            r"
            UPDATE messages SET usage_row = NOT EXISTS (
                SELECT 1 FROM messages older
                 WHERE older.session_id = messages.session_id
                   AND older.group_key  = messages.group_key
                   AND older.id         < messages.id)
             WHERE usage_row IS NULL;
            ",
        )?;
        Ok(())
    }

    fn stamp_refreshed(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("mirror connection poisoned");
        conn.execute(
            "INSERT OR REPLACE INTO mirror_meta (k, v) VALUES ('refreshed_at', ?)",
            [Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    fn insert_messages(&self, rows: &[MessageRow]) -> anyhow::Result<usize> {
        let conn = self.conn.lock().expect("mirror connection poisoned");
        let before: i64 = conn.query_row("SELECT count(*) FROM messages", [], |r| r.get(0))?;
        let mut stmt = conn.prepare(
            r#"INSERT OR IGNORE INTO messages
               (id, session_id, message_id, uuid, provider, model, role, "timestamp",
                input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens,
                cost_usd, group_key, conversational, usage_row)
               VALUES (?, ?, ?, ?, ?, ?, ?, CAST(? AS TIMESTAMPTZ), ?, ?, ?, ?, ?,
                       coalesce(?, ?, CAST(? AS VARCHAR)), ? IS NOT NULL, NULL)"#,
        )?;
        for r in rows {
            stmt.execute(duckdb::params![
                r.id,
                r.session_id,
                r.message_id,
                r.uuid,
                r.provider,
                r.model,
                r.role,
                // Bound as RFC3339 text and cast: DuckDB has no BIGINT ->
                // TIMESTAMPTZ conversion, so epoch micros would fail here.
                r.timestamp.map(|t| t.to_rfc3339()),
                r.input_tokens,
                r.output_tokens,
                r.cache_creation_tokens,
                r.cache_read_tokens,
                r.cost_usd,
                // group_key = coalesce(message_id, uuid, id)
                r.message_id,
                r.uuid,
                r.id,
                // conversational = role IS NOT NULL
                r.role,
            ])?;
        }
        let after: i64 = conn.query_row("SELECT count(*) FROM messages", [], |r| r.get(0))?;
        Ok((after - before).max(0) as usize)
    }

    async fn refresh_projects(&self, pg: &PgPool) -> anyhow::Result<()> {
        // NB: the column is `name`, not `project_name`.
        let rows = sqlx::query_as::<_, ProjectRow>(
            "SELECT id, project_path, name AS project_name, identity_key FROM projects",
        )
        .fetch_all(pg)
        .await?;
        let conn = self.conn.lock().expect("mirror connection poisoned");
        let mut stmt = conn.prepare(
            "INSERT OR REPLACE INTO projects (id, project_path, project_name, identity_key)
             VALUES (?, ?, ?, ?)",
        )?;
        for r in &rows {
            stmt.execute(duckdb::params![
                r.id,
                r.project_path,
                r.project_name,
                r.identity_key
            ])?;
        }
        Ok(())
    }

    async fn refresh_sessions(&self, pg: &PgPool) -> anyhow::Result<()> {
        let rows = sqlx::query_as::<_, SessionRow>(
            "SELECT id, project_id, session_id, summary FROM sessions",
        )
        .fetch_all(pg)
        .await?;
        let conn = self.conn.lock().expect("mirror connection poisoned");
        let mut stmt = conn.prepare(
            "INSERT OR REPLACE INTO sessions (id, project_id, session_id, summary)
             VALUES (?, ?, ?, ?)",
        )?;
        for r in &rows {
            stmt.execute(duckdb::params![r.id, r.project_id, r.session_id, r.summary])?;
        }
        Ok(())
    }

    async fn refresh_tool_uses(&self, pg: &PgPool, from_id: i64) -> anyhow::Result<usize> {
        let rows = sqlx::query_as::<_, ToolUseRow>(
            r"SELECT message_ref, session_id, tool_name, skill_name, subagent_type,
                      tool_use_id, is_error, seq
                 FROM message_tool_uses WHERE message_ref > $1",
        )
        .bind(from_id)
        .fetch_all(pg)
        .await?;
        let conn = self.conn.lock().expect("mirror connection poisoned");
        let before: i64 =
            conn.query_row("SELECT count(*) FROM message_tool_uses", [], |r| r.get(0))?;
        let mut stmt = conn.prepare(
            r"INSERT OR IGNORE INTO message_tool_uses
               (message_ref, seq, session_id, tool_name, skill_name, subagent_type,
                tool_use_id, is_error)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )?;
        for r in &rows {
            stmt.execute(duckdb::params![
                r.message_ref,
                r.seq,
                r.session_id,
                r.tool_name,
                r.skill_name,
                r.subagent_type,
                r.tool_use_id,
                r.is_error
            ])?;
        }
        let after: i64 =
            conn.query_row("SELECT count(*) FROM message_tool_uses", [], |r| r.get(0))?;
        Ok((after - before).max(0) as usize)
    }

    async fn refresh_tool_results(&self, pg: &PgPool, from_id: i64) -> anyhow::Result<usize> {
        let rows = sqlx::query_as::<_, ToolResultRow>(
            r"SELECT message_ref, session_id, tool_use_id, is_error, seq
                 FROM message_tool_results WHERE message_ref > $1",
        )
        .bind(from_id)
        .fetch_all(pg)
        .await?;
        let conn = self.conn.lock().expect("mirror connection poisoned");
        let before: i64 = conn.query_row("SELECT count(*) FROM message_tool_results", [], |r| {
            r.get(0)
        })?;
        let mut stmt = conn.prepare(
            r"INSERT OR IGNORE INTO message_tool_results
               (message_ref, seq, session_id, tool_use_id, is_error)
               VALUES (?, ?, ?, ?, ?)",
        )?;
        for r in &rows {
            stmt.execute(duckdb::params![
                r.message_ref,
                r.seq,
                r.session_id,
                r.tool_use_id,
                r.is_error
            ])?;
        }
        let after: i64 = conn.query_row("SELECT count(*) FROM message_tool_results", [], |r| {
            r.get(0)
        })?;
        Ok((after - before).max(0) as usize)
    }
}

/// `<path>.aside-<utc timestamp>` — corrupt mirrors are preserved, never
/// deleted, so a bad state can still be inspected afterwards.
fn aside_path(path: &Path) -> PathBuf {
    let stamp = Utc::now().format("%Y%m%d-%H%M%S");
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".aside-{stamp}"));
    path.with_file_name(name)
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS messages (
    id                    BIGINT PRIMARY KEY,
    session_id            BIGINT,
    message_id            VARCHAR,
    uuid                  VARCHAR,
    provider              VARCHAR,
    model                 VARCHAR,
    role                  VARCHAR,
    "timestamp"           TIMESTAMPTZ,
    input_tokens          BIGINT,
    output_tokens         BIGINT,
    cache_creation_tokens BIGINT,
    cache_read_tokens     BIGINT,
    cost_usd              DOUBLE,
    -- Stored rather than derived per query: it is what the dedup groups on,
    -- and the index below makes the usage_row pass an indexed lookup.
    group_key             VARCHAR,
    conversational        BOOLEAN,
    -- NULL means "not yet decided"; compute_derived() fills it once.
    usage_row             BOOLEAN
);
CREATE INDEX IF NOT EXISTS messages_group ON messages (session_id, group_key, id);
CREATE INDEX IF NOT EXISTS messages_ts ON messages ("timestamp");

CREATE TABLE IF NOT EXISTS sessions (
    id         BIGINT PRIMARY KEY,
    project_id BIGINT,
    -- Mirrored so /v1/stats/sessions/{id} can accept a UUID without Postgres.
    session_id VARCHAR,
    summary    VARCHAR
);
CREATE INDEX IF NOT EXISTS sessions_uuid ON sessions (session_id);

CREATE TABLE IF NOT EXISTS projects (
    id           BIGINT PRIMARY KEY,
    project_path VARCHAR,
    project_name VARCHAR,
    -- Mirrored so per-project stats can fold by identity without Postgres.
    identity_key VARCHAR
);
CREATE INDEX IF NOT EXISTS projects_identity ON projects (identity_key);

CREATE TABLE IF NOT EXISTS message_tool_uses (
    message_ref   BIGINT,
    seq           INTEGER,
    session_id    BIGINT,
    tool_name     VARCHAR,
    skill_name    VARCHAR,
    subagent_type VARCHAR,
    tool_use_id   VARCHAR,
    is_error      BOOLEAN,
    PRIMARY KEY (message_ref, seq)
);

CREATE TABLE IF NOT EXISTS message_tool_results (
    message_ref BIGINT,
    seq         INTEGER,
    session_id  BIGINT,
    tool_use_id VARCHAR,
    is_error    BOOLEAN,
    PRIMARY KEY (message_ref, seq)
);

CREATE TABLE IF NOT EXISTS mirror_meta (k VARCHAR PRIMARY KEY, v VARCHAR);
"#;

#[derive(sqlx::FromRow)]
struct MessageRow {
    id: i64,
    session_id: i64,
    message_id: Option<String>,
    uuid: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    role: Option<String>,
    timestamp: Option<DateTime<Utc>>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_creation_tokens: Option<i64>,
    cache_read_tokens: Option<i64>,
    cost_usd: Option<f64>,
}

#[derive(sqlx::FromRow)]
struct ProjectRow {
    id: i64,
    project_path: Option<String>,
    project_name: Option<String>,
    identity_key: Option<String>,
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: i64,
    project_id: Option<i64>,
    session_id: Option<String>,
    summary: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ToolUseRow {
    message_ref: i64,
    session_id: Option<i64>,
    tool_name: Option<String>,
    skill_name: Option<String>,
    subagent_type: Option<String>,
    tool_use_id: Option<String>,
    is_error: Option<bool>,
    seq: i32,
}

#[derive(sqlx::FromRow)]
struct ToolResultRow {
    message_ref: i64,
    session_id: Option<i64>,
    tool_use_id: Option<String>,
    is_error: Option<bool>,
    seq: i32,
}
