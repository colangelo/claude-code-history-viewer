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

use crate::client_scope::{classify, Class, Markers, MessageFields, ProjectFields, SessionFields};

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
            // The project's other fields are read from its stored row by
            // `raise_client_projects`, after the batch's project writes.
            project: ProjectFields::default(),
        };
        let class = classify(
            markers,
            fields,
            msgs.iter().map(|m| MessageFields {
                content: m.content.as_ref().unwrap_or(&null),
                raw: &m.raw,
                search_text: m.search_text.as_deref(),
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
pub(crate) const FAMILY: &str = "provider = $1
      AND (session_id = $2 OR session_id = $3
           OR file_path LIKE '%/' || $2 || '/subagents/%')
      AND id <> $4";

pub(crate) fn base(class: &str) -> &str {
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

        family(
            tx,
            *pk,
            &p.provider,
            &p.provider_session_id,
            p.file_path.as_deref(),
            after.as_deref(),
        )
        .await?;
    }
    Ok(())
}

/// The family step for one session whose class is now `after`: a client
/// session's family turns client with it, and a lab session in a client family
/// inherits the family's class.
async fn family(
    tx: &mut Transaction<'_, Postgres>,
    pk: i64,
    provider: &str,
    provider_session_id: &str,
    file_path: Option<&str>,
    after: Option<&str>,
) -> Result<(), sqlx::Error> {
    let parent = file_path.and_then(parent_from_path).unwrap_or("");
    match after {
        Some(c) if c != "none" => {
            sqlx::query(&format!(
                "UPDATE sessions SET client_class = 'family:' || $5, client_rev = client_rev + 1
                 WHERE {FAMILY} AND (client_class IS NULL OR client_class = 'none')"
            ))
            .bind(provider)
            .bind(provider_session_id)
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
            .bind(provider)
            .bind(provider_session_id)
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
    Ok(())
}

/// E1: classify each project from its stored row (path, name, git remote,
/// main checkout path; the effective values after this batch's writes). A hit
/// raises every unscanned or lab session of that project to `path`, with its
/// family, in the batch's transaction. This also covers a batch that only
/// updates a project row. Raise-only: a client class is never touched.
pub async fn raise_client_projects(
    tx: &mut Transaction<'_, Postgres>,
    markers: &Markers,
    project_ids: &[i64],
) -> Result<u64, sqlx::Error> {
    if project_ids.is_empty() {
        return Ok(0);
    }
    type ProjectRow = (i64, String, Option<String>, Option<String>, Option<String>);
    let rows: Vec<ProjectRow> = sqlx::query_as(
        "SELECT id, project_path, name, git_remote_url, git_main_path
         FROM projects WHERE id = ANY($1)",
    )
    .bind(project_ids)
    .fetch_all(&mut **tx)
    .await?;
    let hits: Vec<i64> = rows
        .iter()
        .filter(|(_, path, name, remote, main)| {
            classify(
                markers,
                SessionFields {
                    project_path: path,
                    project: ProjectFields {
                        name: name.as_deref(),
                        git_remote_url: remote.as_deref(),
                        git_main_path: main.as_deref(),
                    },
                    ..Default::default()
                },
                std::iter::empty(),
            ) == Class::Path
        })
        .map(|r| r.0)
        .collect();
    if hits.is_empty() {
        return Ok(0);
    }
    let raised: Vec<(i64, String, String, Option<String>)> = sqlx::query_as(
        "UPDATE sessions SET client_class = 'path', client_rev = client_rev + 1
         WHERE project_id = ANY($1) AND (client_class IS NULL OR client_class = 'none')
         RETURNING id, provider, session_id, file_path",
    )
    .bind(&hits)
    .fetch_all(&mut **tx)
    .await?;
    for (pk, provider, sid, file_path) in &raised {
        family(tx, *pk, provider, sid, file_path.as_deref(), Some("path")).await?;
    }
    Ok(raised.len() as u64)
}

/// Drop the coverage stamp of these sessions: they are unscanned until a
/// sweep reads them whole under a usable policy. The class is kept (it only
/// rises).
pub async fn invalidate(
    tx: &mut Transaction<'_, Postgres>,
    ids: &[i64],
) -> Result<(), sqlx::Error> {
    if ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE sessions SET client_gen = NULL, client_rev = client_rev + 1 WHERE id = ANY($1)",
    )
    .bind(ids)
    .execute(&mut **tx)
    .await?;
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
