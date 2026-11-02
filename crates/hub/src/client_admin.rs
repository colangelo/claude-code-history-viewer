//! Audited client-scope actions (openspec `hub-client-read-scope`, D3, R2-D,
//! R2-F, R2-I, task 2.5). Each one names its approver and writes one audit row
//! per change; none runs automatically. The approver is attribution, not
//! authentication (security's note on R2-D): the boundary is who can run the
//! hub CLI as the pg1 writer.

use sqlx::PgPool;

use crate::client_policy::Policy;
use crate::client_scope::{normalize, Markers};
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

/// (id, headline, summary, topics, open questions, session ids).
type EntryRow = (
    i64,
    Option<String>,
    Option<String>,
    Vec<String>,
    Vec<String>,
    Vec<i64>,
);

/// Counts only.
#[derive(Debug, Default)]
pub struct RestampReport {
    pub checked: u64,
    pub restampable: u64,
    pub entry_text_hit: u64,
    pub session_not_lab: u64,
    pub session_term_hit: u64,
}

/// `restamp-history` (R2-F): an entry with no current route stamp becomes lab
/// again only when its own text (headline, summary, topics, questions) and
/// every one of its sessions match nothing under the FULL router: the path
/// markers and the content terms. Its sessions must also be lab under the
/// current generation. A dry run writes nothing; the write needs ac's row with
/// the client desk's word, which `approved_by` names.
pub async fn restamp_history(
    pool: &PgPool,
    policy: &Policy,
    approved_by: Option<&str>,
) -> anyhow::Result<RestampReport> {
    let mut report = RestampReport::default();
    let terms = &policy.terms;
    let hit = |m: &Markers, text: &str| {
        let t = normalize(text);
        m.matches(&t) || terms.iter().any(|w| t.contains(w.as_str()))
    };
    let rows: Vec<EntryRow> = sqlx::query_as(
        "SELECT id, headline, summary, topics, open_questions, session_ids
             FROM journal_entries
             WHERE status = 'entry'
               AND (route IS NULL OR route_at IS DISTINCT FROM generated_at)
             ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    for (id, headline, summary, topics, questions, sessions) in rows {
        report.checked += 1;
        let text = [headline.unwrap_or_default(), summary.unwrap_or_default()]
            .into_iter()
            .chain(topics)
            .chain(questions)
            .collect::<Vec<_>>()
            .join("\n");
        if hit(&policy.markers, &text) {
            report.entry_text_hit += 1;
            continue;
        }
        let lab: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sessions WHERE id = ANY($1) AND client_class = 'none' AND client_gen = $2",
        )
        .bind(&sessions)
        .bind(&policy.generation)
        .fetch_one(pool)
        .await?;
        if sessions.is_empty() || usize::try_from(lab).unwrap_or(0) != sessions.len() {
            report.session_not_lab += 1;
            continue;
        }
        // The hub's flag uses no terms (Q1), so the terms are checked here,
        // over every message of every session.
        let term_hit: bool = if terms.is_empty() {
            false
        } else {
            let pattern: Vec<String> = terms.iter().map(|t| format!("%{t}%")).collect();
            sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM messages WHERE session_id = ANY($1)
                                AND lower(coalesce(content::text, '')) LIKE ANY($2))",
            )
            .bind(&sessions)
            .bind(&pattern)
            .fetch_one(pool)
            .await?
        };
        if term_hit {
            report.session_term_hit += 1;
            continue;
        }
        report.restampable += 1;
        if let Some(by) = approved_by {
            sqlx::query(
                "UPDATE journal_entries SET route = 'general-restamped', route_at = generated_at WHERE id = $1",
            )
            .bind(id)
            .execute(pool)
            .await?;
            sqlx::query(
                "INSERT INTO client_scope_audit (action, entry_id, generation, approved_by)
                 VALUES ('restamp-history', $1, $2, $3)",
            )
            .bind(id)
            .bind(&policy.generation)
            .bind(by)
            .execute(pool)
            .await?;
        }
    }
    Ok(report)
}
