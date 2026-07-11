//! `POST /v1/ingest` — authenticated, idempotent batch ingestion.
//!
//! Upserts machines/projects/sessions and inserts messages with
//! `ON CONFLICT DO NOTHING` (dedup on `(session_id, message_key)`), then
//! recomputes session/project aggregates from the archived rows so they reflect
//! the cumulative archive, not just the current batch. The whole batch runs in
//! one transaction.

use std::collections::{HashMap, HashSet};

use archive_protocol::{IngestBatch, IngestResponse};
use axum::extract::State;
use axum::Json;
use chrono::{DateTime, Utc};

use crate::auth::AuthedMachine;
use crate::error::HubError;
use crate::state::AppState;

/// Parse an RFC 3339 timestamp leniently; `None`/invalid → `None` (stored NULL).
fn parse_ts(s: Option<&str>) -> Option<DateTime<Utc>> {
    s.and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

/// Replace U+0000 with U+FFFD. Postgres can store NUL in neither `jsonb`
/// ("unsupported Unicode escape sequence") nor `TEXT`, and real transcripts do
/// contain it (raw terminal output), which 500-failed whole batches.
fn strip_nul(s: &mut String) {
    if s.contains('\0') {
        *s = s.replace('\0', "\u{FFFD}");
    }
}

fn strip_nul_opt(s: &mut Option<String>) {
    if let Some(v) = s {
        strip_nul(v);
    }
}

fn strip_nul_value(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::String(s) => strip_nul(s),
        serde_json::Value::Array(items) => items.iter_mut().for_each(strip_nul_value),
        serde_json::Value::Object(map) => {
            let bad_keys: Vec<String> = map.keys().filter(|k| k.contains('\0')).cloned().collect();
            for k in bad_keys {
                if let Some(inner) = map.remove(&k) {
                    map.insert(k.replace('\0', "\u{FFFD}"), inner);
                }
            }
            map.values_mut().for_each(strip_nul_value);
        }
        _ => {}
    }
}

/// Postgres rejects tsvectors over 1 MiB of lexeme data, and `text_search` is
/// GENERATED from `search_text` — an over-long message would fail the whole
/// batch (gitea #7; hit in practice by Time Machine backfills of old sessions).
/// Clamp well under the limit: only FTS on the tail is lost; `raw`/`content`
/// keep full fidelity.
const SEARCH_TEXT_MAX_BYTES: usize = 512 * 1024;

/// Truncate a string in place to at most `max` bytes on a char boundary.
fn clamp_utf8(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

/// Sanitize every free-text/JSON field Postgres will reject NULs in, and
/// clamp `search_text` under the tsvector size limit.
fn sanitize_batch(batch: &mut IngestBatch) {
    for s in &mut batch.sessions {
        strip_nul_opt(&mut s.summary);
    }
    for m in &mut batch.messages {
        strip_nul_value(&mut m.raw);
        if let Some(c) = &mut m.content {
            strip_nul_value(c);
        }
        strip_nul_opt(&mut m.search_text);
        if let Some(st) = &mut m.search_text {
            clamp_utf8(st, SEARCH_TEXT_MAX_BYTES);
        }
    }
}

pub async fn ingest(
    AuthedMachine(token_machine): AuthedMachine,
    State(state): State<AppState>,
    Json(mut batch): Json<IngestBatch>,
) -> Result<Json<IngestResponse>, HubError> {
    sanitize_batch(&mut batch);
    // A machine may only ingest under its own (token-bound) identity.
    if batch.machine.machine_id != token_machine {
        return Err(HubError::Unauthorized);
    }

    let mut resp = IngestResponse::default();
    let mut tx = state.pool.begin().await?;

    // -- machine -----------------------------------------------------------
    sqlx::query!(
        r#"
        INSERT INTO machines (machine_id, hostname, os, first_seen, last_seen)
        VALUES ($1, $2, $3, now(), now())
        ON CONFLICT (machine_id)
        DO UPDATE SET hostname = excluded.hostname, os = excluded.os, last_seen = now()
        "#,
        batch.machine.machine_id,
        batch.machine.hostname,
        batch.machine.os,
    )
    .execute(&mut *tx)
    .await?;

    // -- projects ----------------------------------------------------------
    // (provider, project_path) -> surrogate project id, for session linkage.
    let mut project_ids: HashMap<(String, String), i64> = HashMap::new();
    for p in &batch.projects {
        let row = sqlx::query!(
            r#"
            INSERT INTO projects
                (machine_id, provider, project_path, name, storage_type,
                 session_count, message_count, last_modified, updated_at)
            VALUES ($1, $2, $3, $4, $5,
                    COALESCE($6, 0), COALESCE($7, 0), $8, now())
            ON CONFLICT (machine_id, provider, project_path)
            DO UPDATE SET name = excluded.name,
                          storage_type = excluded.storage_type,
                          last_modified = excluded.last_modified,
                          updated_at = now()
            RETURNING id, (xmax = 0) AS "inserted!: bool"
            "#,
            token_machine,
            p.provider,
            p.project_path,
            p.name,
            p.storage_type,
            p.session_count,
            p.message_count,
            parse_ts(p.last_modified.as_deref()),
        )
        .fetch_one(&mut *tx)
        .await?;
        project_ids.insert((p.provider.clone(), p.project_path.clone()), row.id);
        if row.inserted {
            resp.projects_inserted += 1;
        } else {
            resp.projects_skipped += 1;
        }
    }

    // -- sessions ----------------------------------------------------------
    // (provider, session_id) -> surrogate session id, for message linkage.
    let mut session_ids: HashMap<(String, String), i64> = HashMap::new();
    for s in &batch.sessions {
        let project_id = s
            .project_path
            .as_ref()
            .and_then(|pp| project_ids.get(&(s.provider.clone(), pp.clone())))
            .copied();
        let row = sqlx::query!(
            r#"
            INSERT INTO sessions
                (machine_id, provider, session_id, project_id, file_path, entrypoint,
                 summary, has_tool_use, has_errors, storage_type, last_modified, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7,
                    COALESCE($8, false), COALESCE($9, false), $10, $11, now())
            ON CONFLICT (machine_id, provider, session_id)
            DO UPDATE SET project_id = COALESCE(excluded.project_id, sessions.project_id),
                          file_path = excluded.file_path,
                          entrypoint = excluded.entrypoint,
                          summary = excluded.summary,
                          has_tool_use = sessions.has_tool_use OR excluded.has_tool_use,
                          has_errors = sessions.has_errors OR excluded.has_errors,
                          storage_type = excluded.storage_type,
                          last_modified = excluded.last_modified,
                          updated_at = now()
            RETURNING id, (xmax = 0) AS "inserted!: bool"
            "#,
            token_machine,
            s.provider,
            s.session_id,
            project_id,
            s.file_path,
            s.entrypoint,
            s.summary,
            s.has_tool_use,
            s.has_errors,
            s.storage_type,
            parse_ts(s.last_modified.as_deref()),
        )
        .fetch_one(&mut *tx)
        .await?;
        session_ids.insert((s.provider.clone(), s.session_id.clone()), row.id);
        if row.inserted {
            resp.sessions_inserted += 1;
        } else {
            resp.sessions_skipped += 1;
        }
    }

    // -- messages ----------------------------------------------------------
    let mut touched_sessions: HashSet<i64> = HashSet::new();
    for m in &batch.messages {
        // Resolve the session surrogate id: from this batch first, then the DB
        // (the message may belong to a session ingested in an earlier batch).
        let key = (m.provider.clone(), m.session_id.clone());
        let session_pk = if let Some(id) = session_ids.get(&key) {
            *id
        } else {
            let found = sqlx::query_scalar!(
                "SELECT id FROM sessions WHERE machine_id = $1 AND provider = $2 AND session_id = $3",
                token_machine,
                m.provider,
                m.session_id,
            )
            .fetch_optional(&mut *tx)
            .await?;
            let Some(id) = found else {
                return Err(HubError::BadRequest(format!(
                    "message references unknown session {}/{} (no session in batch or archive)",
                    m.provider, m.session_id
                )));
            };
            session_ids.insert(key, id);
            id
        };

        let result = sqlx::query!(
            r#"
            INSERT INTO messages
                (session_id, machine_id, provider, message_key, uuid, parent_uuid, seq,
                 "timestamp", type, role, model, stop_reason,
                 input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens,
                 cost_usd, duration_ms, is_sidechain, content, raw, search_text)
            VALUES ($1, $2, $3, $4, $5, $6, $7,
                    $8, $9, $10, $11, $12,
                    $13, $14, $15, $16,
                    $17, $18, $19, $20, $21, $22)
            ON CONFLICT (session_id, message_key) DO NOTHING
            "#,
            session_pk,
            token_machine,
            m.provider,
            m.message_key,
            m.uuid,
            m.parent_uuid,
            m.seq,
            parse_ts(m.timestamp.as_deref()),
            m.message_type,
            m.role,
            m.model,
            m.stop_reason,
            m.input_tokens,
            m.output_tokens,
            m.cache_creation_tokens,
            m.cache_read_tokens,
            m.cost_usd,
            m.duration_ms,
            m.is_sidechain,
            m.content,
            m.raw,
            m.search_text,
        )
        .execute(&mut *tx)
        .await?;

        if result.rows_affected() == 1 {
            resp.messages_inserted += 1;
            touched_sessions.insert(session_pk);
        } else {
            resp.messages_skipped += 1;
        }
    }

    // -- aggregates --------------------------------------------------------
    // Recompute from the archived rows (cumulative), for every session that
    // gained messages, and then for their projects.
    let mut touched_projects: HashSet<i64> = HashSet::new();
    for session_pk in &touched_sessions {
        let row = sqlx::query!(
            r#"
            UPDATE sessions s
            SET message_count = sub.cnt,
                first_message_time = sub.first_ts,
                last_message_time = sub.last_ts
            FROM (
                SELECT count(*) AS cnt,
                       min("timestamp") AS first_ts,
                       max("timestamp") AS last_ts
                FROM messages WHERE session_id = $1
            ) sub
            WHERE s.id = $1
            RETURNING s.project_id
            "#,
            session_pk,
        )
        .fetch_one(&mut *tx)
        .await?;
        if let Some(pid) = row.project_id {
            touched_projects.insert(pid);
        }
    }
    for project_pk in &touched_projects {
        sqlx::query!(
            r#"
            UPDATE projects p
            SET session_count = (SELECT count(*) FROM sessions WHERE project_id = p.id),
                message_count = (
                    SELECT count(*) FROM messages m
                    JOIN sessions s ON m.session_id = s.id
                    WHERE s.project_id = p.id
                )
            WHERE p.id = $1
            "#,
            project_pk,
        )
        .execute(&mut *tx)
        .await?;
    }

    // Journal dirty-detection watermark. Only sessions that actually gained
    // messages are stamped (`touched_sessions`), so replaying an already-fully-
    // archived batch is a no-op for journal purposes: the session upsert above
    // may bump `updated_at`, but `ingest_xid` — the value pending compares —
    // stays put. Dirtiness is transaction-VISIBILITY, not wall-clock: pending
    // checks `NOT pg_visible_in_snapshot(ingest_xid, generated_snapshot)`, so
    // an ingest that commits after an entry's snapshot was taken is dirty by
    // construction, however the timestamps interleave. `updated_at` is bumped
    // alongside for human observability only. Runtime query (not `query!`): the
    // offline build has no `.sqlx` metadata for new statements.
    if !touched_sessions.is_empty() {
        let touched: Vec<i64> = touched_sessions.iter().copied().collect();
        sqlx::query(
            "UPDATE sessions
             SET updated_at = clock_timestamp(), ingest_xid = pg_current_xact_id()
             WHERE id = ANY($1)",
        )
        .bind(&touched)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(Json(resp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_utf8_respects_char_boundaries() {
        let mut s = "abcdef".to_string();
        clamp_utf8(&mut s, 4);
        assert_eq!(s, "abcd");

        // 'é' is 2 bytes; a cut inside it must back off to the boundary.
        let mut s = "aéé".to_string(); // bytes: a(1) é(2) é(2) = 5
        clamp_utf8(&mut s, 2);
        assert_eq!(s, "a");

        let mut s = "short".to_string();
        clamp_utf8(&mut s, 100);
        assert_eq!(s, "short");
    }

    #[test]
    fn sanitize_clamps_oversized_search_text() {
        let mut batch = IngestBatch {
            machine: archive_protocol::MachineInfo {
                machine_id: uuid::Uuid::nil(),
                hostname: "h".into(),
                os: None,
            },
            projects: vec![],
            sessions: vec![],
            messages: vec![archive_protocol::IngestMessage {
                provider: "claude".into(),
                session_id: "s".into(),
                message_key: "k".into(),
                uuid: None,
                parent_uuid: None,
                seq: 0,
                timestamp: None,
                message_type: None,
                role: None,
                model: None,
                stop_reason: None,
                input_tokens: None,
                output_tokens: None,
                cache_creation_tokens: None,
                cache_read_tokens: None,
                cost_usd: None,
                duration_ms: None,
                is_sidechain: false,
                content: None,
                raw: serde_json::json!({}),
                search_text: Some("x".repeat(SEARCH_TEXT_MAX_BYTES + 1000)),
            }],
        };
        sanitize_batch(&mut batch);
        assert_eq!(
            batch.messages[0].search_text.as_ref().unwrap().len(),
            SEARCH_TEXT_MAX_BYTES
        );
    }
}
