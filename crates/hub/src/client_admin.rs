//! Audited client-scope actions (openspec `hub-client-read-scope`, D3, R2-D,
//! R2-F, R2-I, task 2.5). Each one names its approver and writes one audit row
//! per change; none runs automatically. The approver is attribution, not
//! authentication (security's note on R2-D): the boundary is who can run the
//! hub CLI as the pg1 writer.

use sqlx::PgPool;

use crate::client_policy::Policy;
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

/// `restamp-history --from FILE` (design R3-E8, security E8): make lab again
/// only the entries the distiller's live router listed in `file`, each after a
/// re-check of what may have changed since the router read it. The hub makes
/// no client decision of its own here.
///
/// The whole file is refused when its generation is not the accepted one, its
/// distiller is older than [`RESTAMP_MIN_DISTILLER`], it finished more than
/// [`RESTAMP_MAX_AGE_HOURS`] before `now`, or `approved_by` is empty (E7).
/// Each entry is then locked and written with its audit row in one
/// transaction, only when it is still unstamped, was not regenerated, names
/// the same sessions, and every session is lab at the current generation with
/// no row or message change after `started_at`. `approved_by: None` is the
/// dry run: the same checks, nothing written.
pub async fn restamp_from_file(
    pool: &PgPool,
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
    let current: Option<String> = sqlx::query_scalar("SELECT generation FROM client_scope_state")
        .fetch_one(pool)
        .await?;
    if current.as_deref() != Some(f.generation.as_str()) {
        anyhow::bail!("the file's generation is not the accepted one: re-run the classify step");
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
    let file_sha = sha256_hex(file);
    let mut report = RestampReport::default();
    for item in &f.restampable {
        report.listed += 1;
        let mut tx = pool.begin().await?;
        let verdict = restamp_check(&mut tx, item, &f.generation, f.started_at).await?;
        match verdict {
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

/// The per-entry re-check, under a row lock: the entry id, or why it is skipped.
async fn restamp_check(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    item: &RestampItem,
    generation: &str,
    started_at: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Result<i64, &'static str>> {
    type Row = (
        i64,
        chrono::DateTime<chrono::Utc>,
        Vec<i64>,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
    );
    let row: Option<Row> = sqlx::query_as(
        "SELECT id, generated_at, session_ids, route, route_at FROM journal_entries
         WHERE entry_date = $1 AND project_path = $2 AND status = 'entry' FOR UPDATE",
    )
    .bind(item.entry_date)
    .bind(&item.project_path)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((id, generated_at, sessions, route, route_at)) = row else {
        return Ok(Err("gone"));
    };
    if route.is_some() && route_at == Some(generated_at) {
        return Ok(Err("stamped"));
    }
    if generated_at != item.generated_at {
        return Ok(Err("regenerated"));
    }
    if sessions != item.session_ids || sessions.is_empty() {
        return Ok(Err("sessions_differ"));
    }
    let (lab, changed): (i64, bool) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE s.client_class = 'none' AND s.client_gen = $2),
                bool_or(s.updated_at > $3
                        OR EXISTS (SELECT 1 FROM messages m WHERE m.session_id = s.id AND m.created_at > $3))
         FROM sessions s WHERE s.id = ANY($1)",
    )
    .bind(&sessions)
    .bind(generation)
    .bind(started_at)
    .fetch_one(&mut **tx)
    .await?;
    if usize::try_from(lab).ok() != Some(sessions.len()) {
        return Ok(Err("session_not_lab"));
    }
    if changed {
        return Ok(Err("changed_since"));
    }
    Ok(Ok(id))
}
