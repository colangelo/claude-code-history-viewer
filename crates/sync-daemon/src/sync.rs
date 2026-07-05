//! The sync pass: enumerate every provider via history-core, deliver each
//! changed session's records to the hub (which dedups), and advance the
//! checkpoint only after the hub acknowledges.
//!
//! Backfill and incremental are the same pass: a session with no checkpoint, or
//! whose file size/mtime changed, is (re)delivered in full; the hub's idempotent
//! ingest makes re-delivery free of duplicates. A file that disappears is simply
//! not seen again — the daemon never issues deletes (cumulative archive).
//!
//! NOTE: byte-offset "parse only appended lines" is a future perf optimization;
//! today a changed JSONL file is re-parsed in full and re-sent (hub dedups).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use archive_protocol::{IngestBatch, IngestResponse, MachineInfo};
use history_core::providers::{self, ProviderId};

use crate::checkpoint::{Checkpoint, FileState};
use crate::client::HubClient;
use crate::convert::{to_ingest_message, to_ingest_project, to_ingest_session};
use crate::identity::Identity;

/// Default hard deadline for a single `client.ingest()` call, independent of
/// whatever the `HubClient` implementation does internally — defense in depth
/// against a `HubClient` that hangs for any reason. Overridable via
/// `CCHV_INGEST_DEADLINE_SECS`.
const DEFAULT_INGEST_DEADLINE_SECS: u64 = 600;

/// Read a positive-integer-seconds env var, falling back to `default_secs`
/// when unset or invalid.
fn env_duration_secs(var: &str, default_secs: u64) -> Duration {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&secs| secs > 0)
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(default_secs))
}

/// Run `client.ingest(batch)` under a hard deadline. A `HubClient` that never
/// resolves (for any reason) must never wedge the sync loop — this is what
/// bounds it regardless of the client implementation.
async fn ingest_with_deadline<C: HubClient>(
    client: &C,
    batch: &IngestBatch,
    deadline: Duration,
) -> anyhow::Result<IngestResponse> {
    if let Ok(result) = tokio::time::timeout(deadline, client.ingest(batch)).await {
        result
    } else {
        tracing::warn!(?deadline, "ingest exceeded per-batch deadline");
        anyhow::bail!("ingest exceeded per-batch deadline of {deadline:?}")
    }
}

#[derive(Debug, Default, Clone)]
pub struct SyncStats {
    pub sessions_scanned: usize,
    pub sessions_synced: usize,
    pub sessions_skipped: usize,
    pub messages_delivered: usize,
    pub errors: usize,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// File size + mtime(ms), or `None` if the path can't be stat'd (e.g. a virtual
/// path) — in which case the session is treated as always-changed.
fn file_meta(path: &str) -> Option<(u64, u64)> {
    let md = std::fs::metadata(path).ok()?;
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Some((md.len(), mtime))
}

/// Run one full sync pass and return what happened. `exclude` lists providers
/// whose discovery is skipped entirely on this machine (see
/// `DaemonConfig::providers_exclude`).
pub async fn run_once<C: HubClient>(
    client: &C,
    identity: &Identity,
    checkpoint: &mut Checkpoint,
    batch_max: usize,
    exclude: &[ProviderId],
) -> SyncStats {
    let mut stats = SyncStats::default();
    let machine = MachineInfo {
        machine_id: identity.machine_id,
        hostname: identity.hostname.clone(),
        os: Some(std::env::consts::OS.to_string()),
    };

    for project in providers::scan_all_projects_except(exclude) {
        let Some(provider_str) = project.provider.clone() else {
            continue;
        };
        let Some(provider) = ProviderId::parse(&provider_str) else {
            continue;
        };
        let project_path = if project.actual_path.is_empty() {
            project.path.clone()
        } else {
            project.actual_path.clone()
        };

        let sessions = match providers::load_sessions(provider, &project.path, false) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(provider = %provider_str, error = %e, "load_sessions failed");
                stats.errors += 1;
                continue;
            }
        };
        let ing_project = to_ingest_project(&provider_str, &project_path, &project);

        for session in &sessions {
            stats.sessions_scanned += 1;
            let file = &session.file_path;
            let meta = file_meta(file);
            let (size, mtime) = meta.unwrap_or((0, 0));
            if meta.is_some() && checkpoint.is_unchanged(file, size, mtime) {
                stats.sessions_skipped += 1;
                continue;
            }

            let messages = match providers::load_messages(provider, file) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(file = %file, error = %e, "load_messages failed");
                    stats.errors += 1;
                    continue;
                }
            };

            // Canonical session id: prefer the messages' own id so they link to
            // the session row; fall back to the session's ids.
            let sid = messages
                .first()
                .map(|m| m.session_id.clone())
                .filter(|s| !s.is_empty())
                .or_else(|| Some(session.actual_session_id.clone()).filter(|s| !s.is_empty()))
                .unwrap_or_else(|| session.session_id.clone());

            let ing_session = to_ingest_session(&provider_str, &project_path, &sid, session);
            let ing_messages: Vec<_> = messages
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let seq = i32::try_from(i).unwrap_or(i32::MAX);
                    to_ingest_message(&provider_str, &sid, seq, m)
                })
                .collect();

            let ok = deliver_session(
                client,
                &machine,
                &ing_project,
                &ing_session,
                &ing_messages,
                batch_max,
                &mut stats,
            )
            .await;

            if ok {
                checkpoint.record(
                    file.clone(),
                    FileState {
                        size,
                        mtime_ms: mtime,
                        message_count: messages.len(),
                        last_synced_ms: now_ms(),
                    },
                );
                if let Err(e) = checkpoint.save() {
                    tracing::error!(error = %e, "checkpoint save failed");
                }
                stats.sessions_synced += 1;
            } else {
                stats.errors += 1;
            }
        }
    }
    stats
}

/// Byte budget for one ingest batch's messages. The hub caps request bodies
/// (32 MiB today); count-only chunking blew through it on old sessions with
/// huge messages (413s during Time Machine backfills). Overridable via
/// `CCHV_INGEST_MAX_BATCH_BYTES`.
const DEFAULT_MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;

fn max_batch_bytes() -> usize {
    std::env::var("CCHV_INGEST_MAX_BATCH_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&b| b > 0)
        .unwrap_or(DEFAULT_MAX_BATCH_BYTES)
}

/// Split messages into chunks bounded by count AND serialized size. A single
/// message over the byte budget still ships (alone) — the hub is the arbiter
/// of a hard reject, and hiding it here would silently drop history.
fn chunk_by_count_and_bytes(
    messages: &[archive_protocol::IngestMessage],
    batch_max: usize,
    byte_max: usize,
) -> Vec<&[archive_protocol::IngestMessage]> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut bytes = 0usize;
    for (i, m) in messages.iter().enumerate() {
        let size = serde_json::to_vec(m).map(|v| v.len()).unwrap_or(0);
        let count_full = i - start >= batch_max.max(1);
        let bytes_full = i > start && bytes + size > byte_max;
        if count_full || bytes_full {
            chunks.push(&messages[start..i]);
            start = i;
            bytes = 0;
        }
        bytes += size;
    }
    if start < messages.len() {
        chunks.push(&messages[start..]);
    }
    chunks
}

/// Deliver one session's project+session+messages, chunked. Returns false if any
/// batch failed (so the caller leaves the checkpoint un-advanced and retries
/// next pass — the hub dedups the re-delivery).
async fn deliver_session<C: HubClient>(
    client: &C,
    machine: &MachineInfo,
    project: &archive_protocol::IngestProject,
    session: &archive_protocol::IngestSession,
    messages: &[archive_protocol::IngestMessage],
    batch_max: usize,
    stats: &mut SyncStats,
) -> bool {
    let deadline = env_duration_secs("CCHV_INGEST_DEADLINE_SECS", DEFAULT_INGEST_DEADLINE_SECS);
    if messages.is_empty() {
        let batch = IngestBatch {
            machine: machine.clone(),
            projects: vec![project.clone()],
            sessions: vec![session.clone()],
            messages: vec![],
        };
        return match ingest_with_deadline(client, &batch, deadline).await {
            Ok(_) => true,
            Err(e) => {
                tracing::error!(error = %e, "ingest failed");
                false
            }
        };
    }
    for chunk in chunk_by_count_and_bytes(messages, batch_max, max_batch_bytes()) {
        let batch = IngestBatch {
            machine: machine.clone(),
            projects: vec![project.clone()],
            sessions: vec![session.clone()],
            messages: chunk.to_vec(),
        };
        match ingest_with_deadline(client, &batch, deadline).await {
            Ok(_) => stats.messages_delivered += chunk.len(),
            Err(e) => {
                tracing::error!(error = %e, "ingest failed");
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(text_len: usize) -> archive_protocol::IngestMessage {
        archive_protocol::IngestMessage {
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
            search_text: Some("x".repeat(text_len)),
        }
    }

    #[test]
    fn chunks_respect_count_cap() {
        let msgs: Vec<_> = (0..10).map(|_| msg(10)).collect();
        let chunks = chunk_by_count_and_bytes(&msgs, 4, usize::MAX);
        let sizes: Vec<usize> = chunks.iter().map(|c| c.len()).collect();
        assert_eq!(sizes, vec![4, 4, 2]);
    }

    #[test]
    fn chunks_respect_byte_cap() {
        // Each message serializes to a bit over 1000 bytes; cap at ~2 messages.
        let msgs: Vec<_> = (0..5).map(|_| msg(1000)).collect();
        let per = serde_json::to_vec(&msgs[0]).unwrap().len();
        let chunks = chunk_by_count_and_bytes(&msgs, 500, per * 2);
        assert!(chunks.iter().all(|c| c.len() <= 2), "chunks: {chunks:?}");
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(total, 5, "no message lost");
    }

    #[test]
    fn oversized_single_message_ships_alone() {
        let msgs = vec![msg(10), msg(100_000), msg(10)];
        let chunks = chunk_by_count_and_bytes(&msgs, 500, 1000);
        let sizes: Vec<usize> = chunks.iter().map(|c| c.len()).collect();
        assert_eq!(sizes, vec![1, 1, 1]);
    }

    #[test]
    fn empty_input_yields_no_chunks() {
        let chunks = chunk_by_count_and_bytes(&[], 500, 1000);
        assert!(chunks.is_empty());
    }
}
