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
///
/// 64 KiB, not the original 512 KiB, because the binding cost is GIN
/// maintenance, not the size limit. m4m measured ingest on pg1 (2026-07-19):
/// insert time tracks TEXT LENGTH, not row count — 434 average messages cost
/// 79 ms, the 434 largest cost 12.9 s, because each big row merges ~9.6k buffers
/// into `messages_text_search_idx`. The corpus is brutally skewed: 812 messages
/// (0.04%) hold 112 MB of 805 MB of `search_text`. Capping at 64 KiB keeps 86%
/// of the total indexed text while cutting that worst-case batch to 6.4 s on its
/// own (4.3 s combined with pg1's `fastupdate`/`gin_pending_list_limit` change).
/// A 500 kB message contributes ~200k lexemes nobody searches for; the snippet
/// (`ts_headline` over `search_text`) can only quote indexed text anyway.
const SEARCH_TEXT_MAX_BYTES: usize = 64 * 1024;

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
    // Ingest used to be entirely silent, so "the hub logged nothing" could not
    // distinguish "the request never arrived" from "the request arrived and was
    // slow" (2026-07-19 daemon retry-backlog investigation). Log both edges.
    let started = std::time::Instant::now();
    tracing::info!(
        machine = %batch.machine.hostname,
        sessions = batch.sessions.len(),
        messages = batch.messages.len(),
        "ingest start"
    );
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
        // Fingerprint facts are validated/normalized hub-side (never trust the
        // daemon's normalization) and COALESCE-guarded on update: a batch that
        // omits them (old daemon, transient capture failure) never clobbers
        // stored non-null facts.
        let root_commit = p
            .git_root_commit
            .as_deref()
            .map(str::trim)
            .filter(|r| r.len() == 40 && r.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(str::to_ascii_lowercase);
        let remote_url = p
            .git_remote_url
            .as_deref()
            .and_then(archive_protocol::identity::normalize_remote_url);
        let row = sqlx::query!(
            r#"
            INSERT INTO projects
                (machine_id, provider, project_path, name, storage_type,
                 session_count, message_count, last_modified, updated_at,
                 git_root_commit, git_remote_url, git_worktree, git_main_path)
            VALUES ($1, $2, $3, $4, $5,
                    COALESCE($6, 0), COALESCE($7, 0), $8, now(),
                    $9, $10, COALESCE($11, false), $12)
            ON CONFLICT (machine_id, provider, project_path)
            DO UPDATE SET name = excluded.name,
                          storage_type = excluded.storage_type,
                          last_modified = excluded.last_modified,
                          updated_at = now(),
                          git_root_commit = COALESCE($9, projects.git_root_commit),
                          git_remote_url = COALESCE($10, projects.git_remote_url),
                          git_worktree = COALESCE($11, projects.git_worktree),
                          git_main_path = COALESCE($12, projects.git_main_path)
            RETURNING id, (xmax = 0) AS "inserted!: bool",
                      git_root_commit, git_remote_url
            "#,
            token_machine,
            p.provider,
            p.project_path,
            p.name,
            p.storage_type,
            p.session_count,
            p.message_count,
            parse_ts(p.last_modified.as_deref()),
            root_commit.as_deref(),
            remote_url.as_deref(),
            p.git_is_worktree,
            p.git_main_path.as_deref(),
        )
        .fetch_one(&mut *tx)
        .await?;
        // Identity key derives from the EFFECTIVE (post-COALESCE) facts, so a
        // partial capture can't flap a project out of its group; always-set is
        // self-healing and a no-op when the facts didn't change.
        let identity_key = archive_protocol::identity::derive_identity_key(
            row.git_root_commit.as_deref(),
            row.git_remote_url.as_deref(),
        );
        sqlx::query!(
            "UPDATE projects SET identity_key = $1 WHERE id = $2",
            identity_key.as_deref(),
            row.id,
        )
        .execute(&mut *tx)
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
    // A project's message_count rolls up its sessions' already-exact counts
    // instead of re-counting the project's messages. The old form
    // (`count(*) FROM messages JOIN sessions WHERE s.project_id = p.id`) was
    // O(messages in the project) on EVERY batch that touched it, executed as one
    // index scan of `messages_session_id_message_key_key` per session of the
    // project — which is where pg1's 6.91B idx_tup_read over 34.9M idx_scan came
    // from (m4m 2026-07-19: 198 tuples read per scan on a unique index whose
    // point lookups should read ~1). Rolling up is O(sessions in the project) on
    // `sessions_project_id_idx` and is still EXACT: every session that gained
    // messages had `message_count` recomputed from `messages` immediately above,
    // and an untouched session's stored count was exact when it was last
    // touched. Runtime query (not `query!`): the offline build has no `.sqlx`
    // metadata for new statements.
    for project_pk in &touched_projects {
        sqlx::query(
            "UPDATE projects p
             SET session_count = sub.session_count,
                 message_count = sub.message_count
             FROM (
                 SELECT count(*)::int                        AS session_count,
                        COALESCE(sum(message_count), 0)::int AS message_count
                 FROM sessions WHERE project_id = $1
             ) sub
             WHERE p.id = $1",
        )
        .bind(project_pk)
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
    tracing::info!(
        messages = batch.messages.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "ingest done"
    );
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
