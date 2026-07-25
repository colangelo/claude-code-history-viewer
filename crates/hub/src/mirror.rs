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
use duckdb::types::Value;
use duckdb::{params_from_iter, Connection};
use sqlx::PgPool;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

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
    /// This refresh started from an empty mirror — the whole archive was
    /// pulled, not an increment. Distinguished because it is the only case that
    /// takes minutes, and the only one during which `/v1/stats/*` answers 503.
    pub cold_build: bool,
    /// Wall-clock the refresh took.
    pub elapsed: std::time::Duration,
}

/// Whether a refresh ran or yielded to one already in flight.
#[derive(Debug, Clone, Copy)]
pub enum RefreshOutcome {
    Ran(RefreshReport),
    /// Another refresh held the mirror. Ticks are **skipped, not queued**: a
    /// refresh that overruns its interval must not build a backlog of them.
    Skipped,
}

impl RefreshOutcome {
    pub fn report(&self) -> Option<&RefreshReport> {
        match self {
            Self::Ran(r) => Some(r),
            Self::Skipped => None,
        }
    }
}

/// How many messages to pull from Postgres per round trip.
const FETCH_CHUNK: i64 = 50_000;

/// How many rows go into one `INSERT` statement.
///
/// **Not a tuning knob — a correctness floor.** `DuckDB` pins a 256 KiB block
/// per `INSERT` statement for as long as the enclosing transaction is open, so
/// one statement per row costs ~370 KiB of memory *per row*: measured, 676
/// single-row inserts exhausted a 256 MB limit, and 2,753 rows exhausted the
/// 1 GB default. A cold build of the real archive (2.8M messages) would have
/// needed terabytes and instead failed a few thousand rows in — permanently,
/// since the mirror would never finish and `/v1/stats/*` would answer 503
/// forever. Batching amortizes that block across many rows; at 500 the
/// generated SQL and its bind list stay small while the per-statement cost
/// stops mattering. `mirror_scale_test` holds this down.
const INSERT_BATCH: usize = 500;

pub struct Mirror {
    /// Single owning connection. `DuckDB` permits one read-write handle to a
    /// file per process; concurrent readers are served by cloning from this.
    conn: Arc<Mutex<Connection>>,
    path: PathBuf,
    cfg: MirrorConfig,
    /// Single-flight latch. Held for the duration of a refresh so an interval
    /// tick arriving mid-refresh is dropped rather than stacking behind it.
    refreshing: Arc<AtomicBool>,
    /// Identity of the file this connection was opened against, so an
    /// out-of-process `hub mirror rebuild` swap is noticed rather than served
    /// past forever. See [`Mirror::adopt_replacement`].
    opened: Mutex<Option<u64>>,
}

/// Releases the single-flight latch however the refresh ends — including on an
/// early `?` return or a panic, which a bare `store(false)` at the end of
/// `refresh` would leak, wedging every later tick.
struct InFlight(Arc<AtomicBool>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
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
            opened: Mutex::new(file_identity(&path)),
            path,
            cfg: cfg.clone(),
            refreshing: Arc::new(AtomicBool::new(false)),
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
        // NB: no `LOAD icu`, and no `AT TIME ZONE` anywhere downstream. The
        // extension is absent from the bundled build, cannot be statically
        // linked from the published crate, and `AT TIME ZONE` needs it for
        // *every* zone including UTC — so local-time bucketing is done from
        // Rust-computed offset spans instead (design D7, and see
        // `duckdb_capability_test`, which fails the build if either reappears).
        Ok(conn)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Configured age past which `/v1/healthz/stats` calls the mirror stale.
    /// Read from here rather than threaded into `AppState` separately, because
    /// the mirror already owns its configuration and a second copy would be a
    /// second thing to keep in step.
    pub fn stale_after_secs(&self) -> u64 {
        self.cfg.stale_after_secs
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
    ///
    /// Single-flight: a call arriving while another is in progress returns
    /// [`RefreshOutcome::Skipped`] immediately rather than waiting its turn. A
    /// queue of refreshes would only ever re-read rows the running one is
    /// already collecting, and on a cold build (minutes) the interval would
    /// stack several deep.
    pub async fn refresh(&self, pg: &PgPool) -> anyhow::Result<RefreshOutcome> {
        let Some(_in_flight) = self.begin_refresh() else {
            tracing::debug!("stats mirror: refresh already in flight, skipping tick");
            return Ok(RefreshOutcome::Skipped);
        };
        Ok(RefreshOutcome::Ran(self.refresh_inner(pg).await?))
    }

    /// Claim the single-flight latch, or `None` if someone else holds it.
    fn begin_refresh(&self) -> Option<InFlight> {
        self.refreshing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            .then(|| InFlight(self.refreshing.clone()))
    }

    /// True when nothing has been mirrored yet — the next refresh is a cold
    /// build of the whole archive rather than an increment.
    pub fn is_empty(&self) -> bool {
        self.watermark() == 0
    }

    async fn refresh_inner(&self, pg: &PgPool) -> anyhow::Result<RefreshReport> {
        let started = Instant::now();
        let watermark = self.watermark();
        let cold_build = watermark == 0;
        let from_id = (watermark - self.cfg.overlap_rows).max(0);
        let mut report = RefreshReport {
            max_id: watermark,
            cold_build,
            ..Default::default()
        };

        // On a cold build the operator is watching a 503 and needs to know
        // whether it is progressing. `max(id)` is a PK lookup, so the
        // denominator is nearly free; a `count(*)` would not be.
        let target_id: i64 = if cold_build {
            let t: i64 = sqlx::query_scalar("SELECT coalesce(max(id), 0) FROM messages")
                .fetch_one(pg)
                .await?;
            tracing::info!(
                target_max_id = t,
                path = %self.path.display(),
                "stats mirror: cold build starting — /v1/stats/* answers 503 until it finishes"
            );
            t
        } else {
            0
        };

        self.refresh_projects(pg).await?;
        self.refresh_aliases(pg).await?;
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

            if cold_build {
                tracing::info!(
                    mirrored = report.messages_inserted,
                    at_id = cursor,
                    target_max_id = target_id,
                    percent = pct(cursor, target_id),
                    elapsed_s = started.elapsed().as_secs(),
                    "stats mirror: cold build progress"
                );
            }
        }

        report.tool_uses_inserted = self.refresh_tool_uses(pg, from_id).await?;
        report.tool_results_inserted = self.refresh_tool_results(pg, from_id).await?;

        self.compute_derived()?;
        self.stamp_refreshed()?;
        // Fold the write-ahead log back into the file itself. Cheap when little
        // changed, and it keeps the mirror self-contained between passes — which
        // is what makes `hub mirror rebuild`'s rename safe: a WAL left straddling
        // a swap would belong to the file that just went away.
        self.checkpoint()?;
        report.elapsed = started.elapsed();

        if cold_build {
            tracing::info!(
                messages = report.messages_inserted,
                tool_uses = report.tool_uses_inserted,
                tool_results = report.tool_results_inserted,
                elapsed_s = report.elapsed.as_secs(),
                "stats mirror: cold build complete — /v1/stats/* now served from the mirror"
            );
        } else {
            tracing::debug!(
                scanned = report.messages_scanned,
                inserted = report.messages_inserted,
                max_id = report.max_id,
                elapsed_ms = report.elapsed.as_millis(),
                "stats mirror: refreshed"
            );
        }
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

    fn checkpoint(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("mirror connection poisoned");
        conn.execute_batch("CHECKPOINT")?;
        Ok(())
    }

    /// Notice a mirror file swapped in underneath us and adopt it.
    ///
    /// `hub mirror rebuild` runs as a separate process and renames a freshly
    /// built file over ours. Rename leaves our open handle pointing at the old
    /// inode, so without this the hub would serve the pre-rebuild data until
    /// someone restarted it — and the whole point of the rebuild is to correct
    /// data the mirror has wrong (design D2). Called from the refresher, under
    /// the single-flight latch, so no write can be in progress across the swap.
    ///
    /// Returns whether a replacement was adopted.
    pub fn adopt_replacement(&self) -> anyhow::Result<bool> {
        let current = file_identity(&self.path);
        {
            let opened = self.opened.lock().expect("mirror identity poisoned");
            if current.is_none() || *opened == current {
                return Ok(false);
            }
        }
        let fresh = Self::open_at(&self.path, &self.cfg)?;
        {
            let mut conn = self.conn.lock().expect("mirror connection poisoned");
            *conn = fresh;
        }
        *self.opened.lock().expect("mirror identity poisoned") = current;
        tracing::info!(
            path = %self.path.display(),
            "stats mirror: adopted a rebuilt file swapped in beneath us"
        );
        Ok(true)
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
        in_transaction(&conn, || insert_all(&conn, rows))?;
        let after: i64 = conn.query_row("SELECT count(*) FROM messages", [], |r| r.get(0))?;
        Ok((after - before).max(0) as usize)
    }

    async fn refresh_projects(&self, pg: &PgPool) -> anyhow::Result<()> {
        // NB: the column is `name`, not `project_name`.
        let rows = sqlx::query_as::<_, ProjectRow>(
            "SELECT id, project_path, name AS project_name, identity_key, git_worktree
               FROM projects",
        )
        .fetch_all(pg)
        .await?;
        let conn = self.conn.lock().expect("mirror connection poisoned");
        in_transaction(&conn, || insert_all(&conn, &rows))
    }

    /// Manual identity aliases — the second half of the identity expansion
    /// `/v1/stats/projects/{key}` performs. Mirrored so that expansion resolves
    /// without Postgres; without it the "stats survive a Postgres outage"
    /// property would hold for global and session scope but silently not for
    /// project scope.
    async fn refresh_aliases(&self, pg: &PgPool) -> anyhow::Result<()> {
        let rows = sqlx::query_as::<_, AliasRow>(
            "SELECT id, identity_key, project_path FROM project_identity_aliases",
        )
        .fetch_all(pg)
        .await?;
        let conn = self.conn.lock().expect("mirror connection poisoned");
        // Aliases are deleted as well as created (`DELETE /v1/identities/
        // aliases/{id}`), and the table is tiny, so it is replaced wholesale
        // rather than folded in — an incremental merge would keep serving a
        // path the operator has just detached.
        in_transaction(&conn, || {
            conn.execute_batch("DELETE FROM project_identity_aliases")?;
            insert_all(&conn, &rows)
        })
    }

    async fn refresh_sessions(&self, pg: &PgPool) -> anyhow::Result<()> {
        let rows = sqlx::query_as::<_, SessionRow>(
            "SELECT id, project_id, session_id, summary FROM sessions",
        )
        .fetch_all(pg)
        .await?;
        let conn = self.conn.lock().expect("mirror connection poisoned");
        in_transaction(&conn, || insert_all(&conn, &rows))
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
        in_transaction(&conn, || insert_all(&conn, &rows))?;
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
        in_transaction(&conn, || insert_all(&conn, &rows))?;
        let after: i64 = conn.query_row("SELECT count(*) FROM message_tool_results", [], |r| {
            r.get(0)
        })?;
        Ok((after - before).max(0) as usize)
    }
}

/// Build a complete new mirror beside the live one and swap it in.
///
/// Incremental refresh covers inserts, not updates: `usage_row` is stable under
/// append because ids are monotonic, but a Postgres-side `UPDATE` of a mirrored
/// column re-groups rows the mirror already decided about. `hub
/// backfill-analytics` is exactly that (`UPDATE messages SET message_id = …`),
/// and after one the mirror over-counts tokens. So the two operations travel
/// together, and this is the second half (design D2).
///
/// The long part — pulling the archive — happens against a staging file, so a
/// running hub keeps answering `/v1/stats/*` from the old mirror throughout,
/// rather than the four-minute `503` that deleting it would cause. The swap
/// itself is a single `rename`, and the running hub adopts the new file on its
/// next refresher tick ([`Mirror::adopt_replacement`]).
pub async fn rebuild(cfg: &MirrorConfig, pg: &PgPool) -> anyhow::Result<RefreshReport> {
    let live = cfg.resolved_path();
    let staging = {
        let stamp = Utc::now().format("%Y%m%d-%H%M%S");
        let mut name = live.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".rebuild-{stamp}"));
        live.with_file_name(name)
    };
    // A previous run killed mid-build would otherwise be resumed rather than
    // rebuilt, which is the one thing a rebuild must not do.
    for stale in [staging.clone(), wal_path(&staging)] {
        if stale.exists() {
            std::fs::remove_file(&stale)?;
        }
    }

    tracing::info!(staging = %staging.display(), "stats mirror: rebuilding");
    let report = {
        let staged_cfg = MirrorConfig {
            path: Some(staging.clone()),
            ..cfg.clone()
        };
        let staged = Mirror::open_or_create(&staged_cfg)?;
        let report = staged.refresh_inner(pg).await?;
        // Drop before the rename: closing checkpoints and clears the staging
        // WAL, so the file that lands at `live` is self-contained.
        drop(staged);
        report
    };

    std::fs::rename(&staging, &live)?;
    tracing::info!(
        path = %live.display(),
        messages = report.messages_inserted,
        elapsed_s = report.elapsed.as_secs(),
        "stats mirror: rebuild swapped in"
    );
    Ok(report)
}

/// `DuckDB` writes its write-ahead log beside the database, same name + `.wal`.
fn wal_path(db: &Path) -> PathBuf {
    let mut name = db.file_name().unwrap_or_default().to_os_string();
    name.push(".wal");
    db.with_file_name(name)
}

/// Identity of the file currently at `path`, for noticing an out-of-process
/// swap. The inode changes exactly when the file is replaced; off unix it
/// degrades to size-and-mtime, which is weaker but still moves on a rebuild.
fn file_identity(path: &Path) -> Option<u64> {
    let md = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(md.ino())
    }
    #[cfg(not(unix))]
    {
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos() as u64);
        Some(md.len() ^ mtime.rotate_left(17))
    }
}

/// Long-running refresher: one pass immediately (the cold build, when no mirror
/// file exists yet), then one per `interval`.
///
/// **It never exits and never propagates an error** (design D6). A failed
/// refresh leaves the mirror exactly as it was and statistics keep being served
/// from it with the age header climbing — a stale answer beats no answer, and
/// the operator sees the staleness on `/v1/healthz/stats`.
///
/// It also stays structurally clear of the credential watchdog: that runs its
/// own probe on its own fresh connections ([`crate::db_watchdog`]) and counts
/// only `28P01`. Nothing here can add a strike, so a Postgres blip during a
/// refresh can never restart the process. The log line says so explicitly,
/// because "hub can't reach Postgres" is the shape operators reflexively read
/// as a credential problem.
pub async fn run_refresher(mirror: Arc<Mirror>, pool: PgPool, interval: std::time::Duration) {
    loop {
        // Before anything else: a `hub mirror rebuild` may have swapped the
        // file since the last tick.
        if let Err(e) = mirror.adopt_replacement() {
            tracing::error!(error = %e, "stats mirror: could not adopt the rebuilt file");
        }
        match mirror.refresh(&pool).await {
            Ok(RefreshOutcome::Ran(_) | RefreshOutcome::Skipped) => {}
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "stats mirror: refresh failed — keeping the existing mirror and \
                     continuing to serve from it. This is a refresh fault, NOT a \
                     credential rejection; it does not count toward the database \
                     watchdog's exit strikes."
                );
            }
        }
        tokio::time::sleep(interval).await;
    }
}

/// A table the mirror pulls from Postgres, from the insert side.
///
/// Exists so every mirrored table goes through [`insert_all`] and inherits its
/// batching. The alternative — a hand-written loop per table — is what allowed
/// the per-row-statement memory blowup to be fixed in one place and survive in
/// four others.
trait Mirrored {
    /// Everything up to the rows: `INSERT OR IGNORE INTO t (a, b, …) VALUES `.
    /// The conflict clause is part of it because it differs per table: append-only
    /// tables IGNORE (idempotent re-reads over the overlap window), small
    /// wholly-refreshed ones REPLACE.
    const INSERT_PREFIX: &'static str;
    /// One row's placeholder tuple, repeated once per row in a batch.
    const ROW: &'static str;
    /// This row's binds, in `ROW` order.
    fn binds(&self) -> Vec<Value>;
}

/// Insert `rows` in [`INSERT_BATCH`]-sized multi-row statements.
fn insert_all<T: Mirrored>(conn: &Connection, rows: &[T]) -> anyhow::Result<()> {
    for chunk in rows.chunks(INSERT_BATCH) {
        let mut sql = String::from(T::INSERT_PREFIX);
        for i in 0..chunk.len() {
            if i > 0 {
                sql.push(',');
            }
            sql.push_str(T::ROW);
        }
        let binds: Vec<Value> = chunk.iter().flat_map(Mirrored::binds).collect();
        conn.prepare(&sql)?.execute(params_from_iter(binds))?;
    }
    Ok(())
}

// Bind helpers: `NULL` in Postgres has to arrive as `Value::Null`, not as an
// empty string or a zero.
fn text(v: Option<&str>) -> Value {
    v.map_or(Value::Null, |s| Value::Text(s.to_string()))
}

fn int(v: Option<i64>) -> Value {
    v.map_or(Value::Null, Value::BigInt)
}

fn dbl(v: Option<f64>) -> Value {
    v.map_or(Value::Null, Value::Double)
}

fn boolean(v: Option<bool>) -> Value {
    v.map_or(Value::Null, Value::Boolean)
}

/// Run `body` inside an explicit `DuckDB` transaction, rolling back if it fails.
///
/// Not an optimization detail worth burying: `DuckDB` autocommits every
/// statement, so a bulk insert without this pays one commit per row and the
/// 2.8M-row cold build goes from minutes to hours. The rollback arm matters as
/// much — the connection is long-lived and shared, so an aborted insert that
/// left a transaction open would poison every later refresh and read.
fn in_transaction<F>(conn: &Connection, body: F) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    conn.execute_batch("BEGIN TRANSACTION")?;
    match body() {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(e) => {
            if let Err(rollback) = conn.execute_batch("ROLLBACK") {
                tracing::error!(error = %rollback, "stats mirror: rollback failed");
            }
            Err(e)
        }
    }
}

/// Progress percentage for the cold-build log, guarding the empty-archive case.
fn pct(at: i64, target: i64) -> u32 {
    if target <= 0 {
        return 100;
    }
    ((at.max(0) as f64 / target as f64) * 100.0).clamp(0.0, 100.0) as u32
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
    -- UTC wall clock as a plain TIMESTAMP, deliberately not TIMESTAMPTZ: every
    -- conversion path off TIMESTAMPTZ in DuckDB routes through the `icu`
    -- extension, which is not in the bundled build. Local-time bucketing is
    -- applied at query time from Rust-computed offset spans (design D7).
    ts_utc                TIMESTAMP,
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
CREATE INDEX IF NOT EXISTS messages_ts ON messages (ts_utc);

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
    identity_key VARCHAR,
    -- Linked `git worktree` member, so ?include_worktrees=false resolves here
    -- too rather than falling back to Postgres.
    git_worktree BOOLEAN
);
CREATE INDEX IF NOT EXISTS projects_identity ON projects (identity_key);

-- The manual half of identity expansion. Small and fully replaced each
-- refresh; see refresh_aliases for why it is not merged incrementally.
CREATE TABLE IF NOT EXISTS project_identity_aliases (
    id           BIGINT PRIMARY KEY,
    identity_key VARCHAR,
    project_path VARCHAR
);
CREATE INDEX IF NOT EXISTS aliases_identity ON project_identity_aliases (identity_key);

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

impl Mirrored for MessageRow {
    const INSERT_PREFIX: &'static str = "INSERT OR IGNORE INTO messages
        (id, session_id, message_id, uuid, provider, model, role, ts_utc,
         input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens,
         cost_usd, group_key, conversational, usage_row) VALUES ";
    /// The last three expressions are the derived columns: `group_key` is the
    /// dedup key, `conversational` is "this row is a turn, not bookkeeping", and
    /// `usage_row` starts NULL for `compute_derived` to decide.
    const ROW: &'static str = "(?, ?, ?, ?, ?, ?, ?, CAST(? AS TIMESTAMP), ?, ?, ?, ?, ?,
         coalesce(?, ?, CAST(? AS VARCHAR)), ? IS NOT NULL, NULL)";

    fn binds(&self) -> Vec<Value> {
        vec![
            Value::BigInt(self.id),
            Value::BigInt(self.session_id),
            text(self.message_id.as_deref()),
            text(self.uuid.as_deref()),
            text(self.provider.as_deref()),
            text(self.model.as_deref()),
            text(self.role.as_deref()),
            // Bound as text and cast: DuckDB has no BIGINT -> TIMESTAMP
            // conversion, so epoch micros would fail here. Naive UTC, not
            // RFC3339 — the column is a plain TIMESTAMP and a zone suffix would
            // have to be interpreted by the ICU code path this design avoids.
            text(
                self.timestamp
                    .map(|t| t.naive_utc().format("%Y-%m-%d %H:%M:%S%.6f").to_string())
                    .as_deref(),
            ),
            int(self.input_tokens),
            int(self.output_tokens),
            int(self.cache_creation_tokens),
            int(self.cache_read_tokens),
            dbl(self.cost_usd),
            // group_key = coalesce(message_id, uuid, id)
            text(self.message_id.as_deref()),
            text(self.uuid.as_deref()),
            Value::BigInt(self.id),
            // conversational = role IS NOT NULL
            text(self.role.as_deref()),
        ]
    }
}

#[derive(sqlx::FromRow)]
struct ProjectRow {
    id: i64,
    project_path: Option<String>,
    project_name: Option<String>,
    identity_key: Option<String>,
    git_worktree: bool,
}

impl Mirrored for ProjectRow {
    const INSERT_PREFIX: &'static str = "INSERT OR REPLACE INTO projects
        (id, project_path, project_name, identity_key, git_worktree) VALUES ";
    const ROW: &'static str = "(?, ?, ?, ?, ?)";

    fn binds(&self) -> Vec<Value> {
        vec![
            Value::BigInt(self.id),
            text(self.project_path.as_deref()),
            text(self.project_name.as_deref()),
            text(self.identity_key.as_deref()),
            Value::Boolean(self.git_worktree),
        ]
    }
}

#[derive(sqlx::FromRow)]
struct AliasRow {
    id: i64,
    identity_key: String,
    project_path: String,
}

impl Mirrored for AliasRow {
    const INSERT_PREFIX: &'static str =
        "INSERT OR REPLACE INTO project_identity_aliases (id, identity_key, project_path) VALUES ";
    const ROW: &'static str = "(?, ?, ?)";

    fn binds(&self) -> Vec<Value> {
        vec![
            Value::BigInt(self.id),
            Value::Text(self.identity_key.clone()),
            Value::Text(self.project_path.clone()),
        ]
    }
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: i64,
    project_id: Option<i64>,
    session_id: Option<String>,
    summary: Option<String>,
}

impl Mirrored for SessionRow {
    const INSERT_PREFIX: &'static str =
        "INSERT OR REPLACE INTO sessions (id, project_id, session_id, summary) VALUES ";
    const ROW: &'static str = "(?, ?, ?, ?)";

    fn binds(&self) -> Vec<Value> {
        vec![
            Value::BigInt(self.id),
            int(self.project_id),
            text(self.session_id.as_deref()),
            text(self.summary.as_deref()),
        ]
    }
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

impl Mirrored for ToolUseRow {
    const INSERT_PREFIX: &'static str = "INSERT OR IGNORE INTO message_tool_uses
        (message_ref, seq, session_id, tool_name, skill_name, subagent_type,
         tool_use_id, is_error) VALUES ";
    const ROW: &'static str = "(?, ?, ?, ?, ?, ?, ?, ?)";

    fn binds(&self) -> Vec<Value> {
        vec![
            Value::BigInt(self.message_ref),
            Value::Int(self.seq),
            int(self.session_id),
            text(self.tool_name.as_deref()),
            text(self.skill_name.as_deref()),
            text(self.subagent_type.as_deref()),
            text(self.tool_use_id.as_deref()),
            boolean(self.is_error),
        ]
    }
}

#[derive(sqlx::FromRow)]
struct ToolResultRow {
    message_ref: i64,
    session_id: Option<i64>,
    tool_use_id: Option<String>,
    is_error: Option<bool>,
    seq: i32,
}

impl Mirrored for ToolResultRow {
    const INSERT_PREFIX: &'static str = "INSERT OR IGNORE INTO message_tool_results
        (message_ref, seq, session_id, tool_use_id, is_error) VALUES ";
    const ROW: &'static str = "(?, ?, ?, ?, ?)";

    fn binds(&self) -> Vec<Value> {
        vec![
            Value::BigInt(self.message_ref),
            Value::Int(self.seq),
            int(self.session_id),
            text(self.tool_use_id.as_deref()),
            boolean(self.is_error),
        ]
    }
}
