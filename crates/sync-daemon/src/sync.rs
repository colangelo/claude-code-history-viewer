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

use std::time::{SystemTime, UNIX_EPOCH};

use archive_protocol::{IngestBatch, MachineInfo};
use history_core::providers::{self, ProviderId};

use crate::checkpoint::{Checkpoint, FileState};
use crate::client::HubClient;
use crate::convert::{to_ingest_message, to_ingest_project, to_ingest_session};
use crate::identity::Identity;

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
    if messages.is_empty() {
        let batch = IngestBatch {
            machine: machine.clone(),
            projects: vec![project.clone()],
            sessions: vec![session.clone()],
            messages: vec![],
        };
        return match client.ingest(&batch).await {
            Ok(_) => true,
            Err(e) => {
                tracing::error!(error = %e, "ingest failed");
                false
            }
        };
    }
    for chunk in messages.chunks(batch_max.max(1)) {
        let batch = IngestBatch {
            machine: machine.clone(),
            projects: vec![project.clone()],
            sessions: vec![session.clone()],
            messages: chunk.to_vec(),
        };
        match client.ingest(&batch).await {
            Ok(_) => stats.messages_delivered += chunk.len(),
            Err(e) => {
                tracing::error!(error = %e, "ingest failed");
                return false;
            }
        }
    }
    true
}
