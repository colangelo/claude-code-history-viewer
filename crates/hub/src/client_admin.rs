//! Audited client-scope actions (openspec `hub-client-read-scope`, D3, R2-D,
//! R2-F, R2-I, task 2.5). Each one names its approver and writes one audit row
//! per change; none runs automatically. The approver is attribution, not
//! authentication (security's note on R2-D): the boundary is who can run the
//! hub CLI as the pg1 writer.

use sqlx::PgPool;

use crate::client_policy::Policy;
use crate::client_scope::normalize;
use crate::client_state::{audit, record_set};

/// Accept the routing file's marker set as it is now, removals included
/// (`accept-routing`). The running hub adopts it at its next refresh.
pub async fn accept_routing(
    pool: &PgPool,
    policy: &Policy,
    approved_by: &str,
) -> anyhow::Result<()> {
    record_set(pool, policy, approved_by).await?;
    audit(
        pool,
        "accept-routing",
        Some(&policy.generation),
        Some(&format!("{} markers", policy.marker_hashes.len())),
        Some(approved_by),
    )
    .await
}

/// Clear one session's client class (a narrowing). Refuses a session that is
/// not client. The session is then lab only once a sweep stamps it.
pub async fn clear(
    pool: &PgPool,
    session: i64,
    reason: &str,
    approved_by: &str,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    let before: Option<Option<String>> =
        sqlx::query_scalar("SELECT client_class FROM sessions WHERE id = $1 FOR UPDATE")
            .bind(session)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(Some(class)) = before.filter(|c| c.as_deref().is_some_and(|c| c != "none")) else {
        anyhow::bail!("session {session} is not client: nothing to clear");
    };
    sqlx::query(
        "UPDATE sessions SET client_class = NULL, client_gen = NULL, client_rev = client_rev + 1 WHERE id = $1",
    )
    .bind(session)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO client_scope_audit (action, session_id, detail, approved_by)
         VALUES ('clear', $1, $2, $3)",
    )
    .bind(session)
    .bind(format!("was {class}; {reason}"))
    .bind(approved_by)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// What `protect` did.
#[derive(Debug, PartialEq, Eq)]
pub enum Protected {
    /// The session was lab or unscanned and is now `manual`.
    Raised,
    /// The session was already client: nothing changed, nothing audited.
    AlreadyClient,
}

/// Raise one session to client by hand (`client-scope protect`, manager-aruba
/// 2026-11-02). Raise-only: a lab or unscanned session becomes `manual`, a
/// client session is left exactly as it is. The class is sticky: ingest and the
/// sweep only raise, and only the audited `clear` lowers it (ac's word). The
/// audit row (previous class, generation, reason, approver) commits with the
/// change. Copies in the session's family are not raised here (E2).
pub async fn protect(
    pool: &PgPool,
    session: i64,
    reason: &str,
    approved_by: &str,
) -> anyhow::Result<Protected> {
    if approved_by.trim().is_empty() {
        anyhow::bail!("--approved-by is empty: name who approved the raise");
    }
    if reason.trim().is_empty() {
        anyhow::bail!("--reason is empty: say why the session is client");
    }
    let mut tx = pool.begin().await?;
    let before: Option<Option<String>> =
        sqlx::query_scalar("SELECT client_class FROM sessions WHERE id = $1 FOR UPDATE")
            .bind(session)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(class) = before else {
        anyhow::bail!("no session {session}");
    };
    if class.as_deref().is_some_and(|c| c != "none") {
        tx.rollback().await?;
        return Ok(Protected::AlreadyClient);
    }
    sqlx::query(
        "UPDATE sessions SET client_class = 'manual', client_rev = client_rev + 1 WHERE id = $1",
    )
    .bind(session)
    .execute(&mut *tx)
    .await?;
    let generation: Option<String> =
        sqlx::query_scalar("SELECT generation FROM client_scope_state")
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query(
        "INSERT INTO client_scope_audit (action, session_id, generation, detail, approved_by)
         VALUES ('protect', $1, $2, $3, $4)",
    )
    .bind(session)
    .bind(generation)
    .bind(format!(
        "was {}; {reason}",
        class.as_deref().unwrap_or("unscanned")
    ))
    .bind(approved_by)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Protected::Raised)
}

/// Lift the enforcement latch (an emergency switch, R2-I).
pub async fn unlatch(pool: &PgPool, reason: &str, approved_by: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE client_scope_state SET enforced_at = NULL, updated_at = now()")
        .execute(pool)
        .await?;
    tracing::error!(%approved_by, "client_scope: enforcement latch lifted");
    audit(pool, "unlatch", None, Some(reason), Some(approved_by)).await
}

/// The oldest distiller whose `--restamp-history` file this hub applies: the
/// release that ships the classify step (design R3-E8).
pub const RESTAMP_MIN_DISTILLER: &str = "0.40.0";
/// A file older than this is refused (security, E8-b).
pub const RESTAMP_MAX_AGE_HOURS: i64 = 24;

#[derive(Debug, serde::Deserialize)]
struct RestampFile {
    format: u32,
    distiller_version: String,
    generation: String,
    started_at: chrono::DateTime<chrono::Utc>,
    finished_at: chrono::DateTime<chrono::Utc>,
    restampable: Vec<RestampItem>,
}

#[derive(Debug, serde::Deserialize)]
struct RestampItem {
    entry_date: chrono::NaiveDate,
    project_path: String,
    generated_at: chrono::DateTime<chrono::Utc>,
    session_ids: Vec<i64>,
}

/// Counts only: entries listed, entries restamped (or that a dry run would
/// restamp), and the entries skipped per reason.
#[derive(Debug, Default)]
pub struct RestampReport {
    pub listed: u64,
    pub restamped: u64,
    pub skipped: std::collections::BTreeMap<&'static str, u64>,
}

impl RestampReport {
    pub fn render(&self) -> String {
        let skipped = self
            .skipped
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "listed {}, restamped {}; skipped: {}",
            self.listed,
            self.restamped,
            if skipped.is_empty() { "none" } else { &skipped }
        )
    }
}

fn version_triple(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim().split('.').map(|p| p.parse::<u64>().ok());
    let t = (it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(t)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// One ingest batch writes its rows with the batch's start time, so a batch
/// that began before the classify run but committed after it carries an
/// earlier stamp. Anything changed this long before the run started counts as
/// changed since (security E8 R7).
pub const RESTAMP_INGEST_MARGIN_SECS: i64 = 900;

/// When protected routing went live (v0.27.0, the distiller's
/// `ROUTING_LIVE_AT`): a Claude entry generated after it was protected by the
/// router of its day.
const ROUTING_LIVE_AT: &str = "2026-10-21T10:16:59Z";

fn is_claude(model: Option<&str>) -> bool {
    let m = model.unwrap_or_default().to_lowercase();
    ["claude", "haiku", "sonnet", "opus"]
        .iter()
        .any(|w| m.contains(w))
}

/// Read the classify step's file: a regular file, not a link, readable by its
/// owner only (security E8 P3).
pub fn read_restamp_file(path: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    // The path itself must not be a link; the file opened must be that same
    // inode (a swap between the two calls is refused).
    let link = std::fs::symlink_metadata(path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    if !link.file_type().is_file() {
        anyhow::bail!(
            "{} is not a regular file (a link is refused)",
            path.display()
        );
    }
    let mut f = std::fs::File::open(path)?;
    let meta = f.metadata()?;
    if (meta.dev(), meta.ino()) != (link.dev(), link.ino()) || !meta.is_file() {
        anyhow::bail!("{} changed while it was opened", path.display());
    }
    if meta.mode() & 0o077 != 0 {
        anyhow::bail!("{} must be mode 0600 (owner only)", path.display());
    }
    let mut out = Vec::new();
    f.read_to_end(&mut out)?;
    Ok(out)
}

/// `restamp-history --from FILE` (design R3-E8, security E8 and its code
/// review R1-R8): make lab again only the entries the distiller's live router
/// listed in `file`, each after a re-check of what may have changed since.
///
/// The whole file is refused when it does not come from `policy` (the hub's
/// own routing files), its distiller is older than [`RESTAMP_MIN_DISTILLER`],
/// it finished more than [`RESTAMP_MAX_AGE_HOURS`] before `now`, or
/// `approved_by` is empty (E7). The run stops when the accepted generation is
/// not the file's (R2, read per entry). Each entry must still carry no route
/// at all (R3), not be regenerated, name the same sessions, pass the Claude
/// cutoff and the entry-text and path checks against the policy's markers AND
/// terms (R6), and every session must be lab at the generation with no row or
/// message change since the run started, less a margin (R7). The apply locks
/// what it reads and writes each route with its audit row in one transaction;
/// `approved_by: None` is the dry run, which locks nothing and runs on a
/// read-only pool (R1).
pub async fn restamp_from_file(
    pool: &PgPool,
    policy: &Policy,
    file: &[u8],
    approved_by: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<RestampReport> {
    if approved_by.is_some_and(|by| by.trim().is_empty()) {
        anyhow::bail!("--approved-by is empty: name who approved the restamp");
    }
    let f: RestampFile =
        serde_json::from_slice(file).map_err(|e| anyhow::anyhow!("not a restamp file: {e}"))?;
    if f.format != 1 {
        anyhow::bail!("not a restamp file: format {}", f.format);
    }
    if f.generation != policy.generation {
        anyhow::bail!(
            "the file's generation is not that of the hub's routing files: re-run the classify step"
        );
    }
    let min = version_triple(RESTAMP_MIN_DISTILLER).expect("a valid minimum");
    if version_triple(&f.distiller_version).map_or(true, |v| v < min) {
        anyhow::bail!(
            "the file comes from distiller {}, older than {RESTAMP_MIN_DISTILLER}",
            f.distiller_version
        );
    }
    if now - f.finished_at > chrono::Duration::hours(RESTAMP_MAX_AGE_HOURS)
        || f.finished_at > now
        || f.started_at > f.finished_at
    {
        anyhow::bail!("the file is older than {RESTAMP_MAX_AGE_HOURS} h (or dated ahead): re-run the classify step");
    }
    let since = f.started_at - chrono::Duration::seconds(RESTAMP_INGEST_MARGIN_SECS);
    let apply = approved_by.is_some();
    let file_sha = sha256_hex(file);
    let mut report = RestampReport::default();
    for item in &f.restampable {
        report.listed += 1;
        let mut tx = pool.begin().await?;
        // R2: the accepted generation, read (and in the apply held) per entry.
        let current: Option<String> = sqlx::query_scalar(if apply {
            "SELECT generation FROM client_scope_state FOR SHARE"
        } else {
            "SELECT generation FROM client_scope_state"
        })
        .fetch_one(&mut *tx)
        .await?;
        if current.as_deref() != Some(f.generation.as_str()) {
            tx.rollback().await?;
            anyhow::bail!(
                "the accepted generation is not the file's (after {} of {} entries): re-run the classify step",
                report.listed - 1,
                f.restampable.len()
            );
        }
        match restamp_check(&mut tx, policy, item, since, apply).await? {
            Err(reason) => *report.skipped.entry(reason).or_default() += 1,
            Ok(id) => {
                report.restamped += 1;
                if let Some(by) = approved_by {
                    sqlx::query(
                        "UPDATE journal_entries SET route = 'general-restamped', route_at = generated_at WHERE id = $1",
                    )
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query(
                        "INSERT INTO client_scope_audit (action, entry_id, generation, detail, approved_by)
                         VALUES ('restamp-history', $1, $2, $3, $4)",
                    )
                    .bind(id)
                    .bind(&f.generation)
                    .bind(format!(
                        "file sha256 {file_sha}; distiller {}; classified {}",
                        f.distiller_version, f.started_at
                    ))
                    .bind(by)
                    .execute(&mut *tx)
                    .await?;
                    tx.commit().await?;
                    continue;
                }
            }
        }
        tx.rollback().await?;
    }
    Ok(report)
}

/// The per-entry re-check: the entry id, or why it is skipped. `apply` locks
/// the entry (`FOR UPDATE`) and its sessions (`FOR SHARE`); the dry run locks
/// nothing (R1).
async fn restamp_check(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    policy: &Policy,
    item: &RestampItem,
    since: chrono::DateTime<chrono::Utc>,
    apply: bool,
) -> anyhow::Result<Result<i64, &'static str>> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i64,
        generated_at: chrono::DateTime<chrono::Utc>,
        session_ids: Vec<i64>,
        route: Option<String>,
        model: Option<String>,
        project_path: String,
        headline: Option<String>,
        summary: Option<String>,
        topics: Vec<String>,
        open_questions: Vec<String>,
    }
    let lock = if apply { " FOR UPDATE" } else { "" };
    let row: Option<Row> = sqlx::query_as(&format!(
        "SELECT id, generated_at, session_ids, route, model, project_path, headline, summary,
                topics, open_questions
         FROM journal_entries
         WHERE entry_date = $1 AND project_path = $2 AND status = 'entry'{lock}"
    ))
    .bind(item.entry_date)
    .bind(&item.project_path)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(e) = row else {
        return Ok(Err("gone"));
    };
    // R3: any route at all, current or stale, is a stamp.
    if e.route.is_some() {
        return Ok(Err("stamped"));
    }
    if e.generated_at != item.generated_at {
        return Ok(Err("regenerated"));
    }
    if e.session_ids != item.session_ids || e.session_ids.is_empty() {
        return Ok(Err("sessions_differ"));
    }
    // R6: the classify step's own checks, again, against the same policy.
    let live: chrono::DateTime<chrono::Utc> = ROUTING_LIVE_AT.parse().expect("a valid cutoff");
    if e.generated_at >= live && is_claude(e.model.as_deref()) {
        return Ok(Err("was_protected"));
    }
    let hit = |text: &str| {
        let t = normalize(text);
        policy.markers.matches(&t) || policy.terms.iter().any(|w| t.contains(w.as_str()))
    };
    let texts = [e.headline.as_deref(), e.summary.as_deref()]
        .into_iter()
        .flatten()
        .chain(e.topics.iter().map(String::as_str))
        .chain(e.open_questions.iter().map(String::as_str));
    for t in texts {
        if hit(t) {
            return Ok(Err("entry_text"));
        }
    }
    if hit(&e.project_path) {
        return Ok(Err("path"));
    }
    // R7: hold the sessions (apply), then: lab at the generation, and no row
    // or message change since the run started, less one ingest batch.
    let share = if apply { " FOR SHARE" } else { "" };
    /// (id, class, generation, last row change).
    type Held = (
        i64,
        Option<String>,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
    );
    let held: Vec<Held> = sqlx::query_as(&format!(
        "SELECT id, client_class, client_gen, updated_at FROM sessions WHERE id = ANY($1){share}"
    ))
    .bind(&e.session_ids)
    .fetch_all(&mut **tx)
    .await?;
    let lab = held.iter().all(|(_, class, gen, _)| {
        class.as_deref() == Some("none") && gen.as_deref() == Some(policy.generation.as_str())
    });
    if held.len() != e.session_ids.len() || !lab {
        return Ok(Err("session_not_lab"));
    }
    let new_messages: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM messages WHERE session_id = ANY($1) AND created_at > $2)",
    )
    .bind(&e.session_ids)
    .bind(since)
    .fetch_one(&mut **tx)
    .await?;
    if new_messages || held.iter().any(|(_, _, _, updated)| *updated > since) {
        return Ok(Err("changed_since"));
    }
    Ok(Ok(e.id))
}
