//! Client scope at ingest (openspec `hub-client-read-scope`, R2-A, R2-B, R2-C).
//!
//! Every batch is classified inside its own transaction, so a message and its
//! session's class become visible together. The rules:
//!
//! - the class only rises: a hit sets a client class on a session whose class is
//!   unset or `none`, and nothing here ever sets a client session back;
//! - a session is stamped with the current generation only when this batch saw
//!   all of it (it was created by this batch) or it was already complete under
//!   the current generation; anything else stays unscanned for the sweep;
//! - a client session's family turns client with it, and a lab session in a
//!   client family inherits the family's class.

use std::collections::HashMap;

use archive_protocol::{IngestMessage, IngestSession};
use serde_json::Value;
use sqlx::{Postgres, Transaction};

use crate::client_scope::{classify, Class, Markers, MessageFields, SessionFields};

/// One session's classification from this batch.
pub struct SessionPlan {
    pub class: Class,
    pub provider: String,
    pub provider_session_id: String,
    pub file_path: Option<String>,
}

#[allow(clippy::implicit_hasher)] // called with ingest's own std maps only
/// Classify every session this batch touches: its row (when the batch carries
/// one) and its messages in this batch.
pub fn plan(
    markers: &Markers,
    sessions: &[IngestSession],
    session_ids: &HashMap<(String, String), i64>,
    messages: &[IngestMessage],
    session_pks: &[i64],
) -> HashMap<i64, SessionPlan> {
    let null = Value::Null;
    let mut by_pk: HashMap<i64, Vec<&IngestMessage>> = HashMap::new();
    for (m, pk) in messages.iter().zip(session_pks) {
        by_pk.entry(*pk).or_default().push(m);
    }
    let mut out = HashMap::new();
    let rows: HashMap<i64, &IngestSession> = sessions
        .iter()
        .filter_map(|s| {
            session_ids
                .get(&(s.provider.clone(), s.session_id.clone()))
                .map(|pk| (*pk, s))
        })
        .collect();
    let mut pks: Vec<i64> = rows.keys().chain(by_pk.keys()).copied().collect();
    pks.sort_unstable();
    pks.dedup();
    for pk in pks {
        let row = rows.get(&pk);
        let msgs = by_pk.get(&pk).map(Vec::as_slice).unwrap_or_default();
        let fields = SessionFields {
            project_path: row.and_then(|s| s.project_path.as_deref()).unwrap_or(""),
            summary: row.and_then(|s| s.summary.as_deref()),
            file_path: row.and_then(|s| s.file_path.as_deref()),
            entrypoint: row.and_then(|s| s.entrypoint.as_deref()),
        };
        let class = classify(
            markers,
            fields,
            msgs.iter().map(|m| MessageFields {
                content: m.content.as_ref().unwrap_or(&null),
                raw: &m.raw,
            }),
        );
        let (provider, provider_session_id) = row.map_or_else(
            || {
                let m = msgs[0];
                (m.provider.clone(), m.session_id.clone())
            },
            |s| (s.provider.clone(), s.session_id.clone()),
        );
        out.insert(
            pk,
            SessionPlan {
                class,
                provider,
                provider_session_id,
                file_path: row.and_then(|s| s.file_path.clone()),
            },
        );
    }
    out
}

/// The parent session id a subagent transcript's path names:
/// `…/<parent uuid>/subagents/<file>`.
pub fn parent_from_path(file_path: &str) -> Option<&str> {
    let idx = file_path.find("/subagents/")?;
    let parent = file_path[..idx].rsplit('/').next()?;
    (parent.len() == 36 && parent.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
        .then_some(parent)
}

/// The family predicate: same provider session id on any machine, the parent a
/// subagent path names, or children whose path names this session.
const FAMILY: &str = "provider = $1
      AND (session_id = $2 OR session_id = $3
           OR file_path LIKE '%/' || $2 || '/subagents/%')
      AND id <> $4";

fn base(class: &str) -> &str {
    class.strip_prefix("family:").unwrap_or(class)
}

#[allow(clippy::implicit_hasher)] // called with ingest's own std maps only
/// Write the plan in the batch's transaction.
pub async fn apply(
    tx: &mut Transaction<'_, Postgres>,
    plan: &HashMap<i64, SessionPlan>,
    generation: &str,
    new_sessions: &std::collections::HashSet<i64>,
) -> Result<(), sqlx::Error> {
    for (pk, p) in plan {
        let class = p.class.as_str();
        let is_new = new_sessions.contains(pk);
        let after: Option<String> = sqlx::query_scalar(
            "UPDATE sessions SET
                client_rev   = client_rev + 1,
                client_class = CASE
                    WHEN $2 <> 'none' AND (client_class IS NULL OR client_class = 'none') THEN $2
                    WHEN $4 AND client_class IS NULL THEN $2
                    ELSE client_class END,
                client_gen   = CASE WHEN $4 OR client_gen = $3 THEN $3 ELSE client_gen END
             WHERE id = $1
             RETURNING client_class",
        )
        .bind(pk)
        .bind(class)
        .bind(generation)
        .bind(is_new)
        .fetch_one(&mut **tx)
        .await?;

        let parent = p
            .file_path
            .as_deref()
            .and_then(parent_from_path)
            .unwrap_or("");
        match after.as_deref() {
            Some(c) if c != "none" => {
                sqlx::query(&format!(
                    "UPDATE sessions SET client_class = 'family:' || $5, client_rev = client_rev + 1
                     WHERE {FAMILY} AND (client_class IS NULL OR client_class = 'none')"
                ))
                .bind(&p.provider)
                .bind(&p.provider_session_id)
                .bind(parent)
                .bind(pk)
                .bind(base(c))
                .execute(&mut **tx)
                .await?;
            }
            _ => {
                let inherited: Option<String> = sqlx::query_scalar(&format!(
                    "SELECT client_class FROM sessions
                     WHERE {FAMILY} AND client_class IS NOT NULL AND client_class <> 'none'
                     LIMIT 1"
                ))
                .bind(&p.provider)
                .bind(&p.provider_session_id)
                .bind(parent)
                .bind(pk)
                .fetch_optional(&mut **tx)
                .await?;
                if let Some(c) = inherited {
                    sqlx::query("UPDATE sessions SET client_class = 'family:' || $2 WHERE id = $1")
                        .bind(pk)
                        .bind(base(&c))
                        .execute(&mut **tx)
                        .await?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parent_is_the_folder_before_subagents() {
        let p = "/Users/ac/.claude/projects/-w/0f0e8d6c-1111-2222-3333-444455556666/subagents/agent-1.jsonl";
        assert_eq!(
            parent_from_path(p),
            Some("0f0e8d6c-1111-2222-3333-444455556666")
        );
        assert_eq!(parent_from_path("/x/not-a-uuid/subagents/a.jsonl"), None);
        assert_eq!(
            parent_from_path("/x/0f0e8d6c-1111-2222-3333-444455556666/a.jsonl"),
            None
        );
    }
}
