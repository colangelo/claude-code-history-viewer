//! Convert history-core's normalized models into the hub wire types.
//!
//! The dedup `message_key` is a content hash over the message's position and
//! stable fields, EXCLUDING `uuid`: history-core fills a missing provider uuid
//! with a random v4, so the uuid is not stable across re-parses and cannot be
//! the dedup key. The session id is supplied by the caller (derived once per
//! session) so every message links to its session.

use std::fmt::Write as _;

use archive_protocol::{IngestMessage, IngestProject, IngestSession};
use history_core::models::{ClaudeMessage, ClaudeProject, ClaudeSession};
use sha2::{Digest, Sha256};

use crate::git_fingerprint::GitFingerprint;

fn nonempty(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

fn token(v: Option<u32>) -> Option<i64> {
    v.map(i64::from)
}

/// True when history-core made up this record's uuid: `<random v4>-line-<n>`.
///
/// It does that for records the provider wrote without one — Claude Code's
/// state records (`permission-mode`, `custom-title`, `mode`, …). Those records
/// carry no `timestamp` either, and history-core fills that with `now()`, so
/// for them the timestamp is exactly as unstable as the uuid.
fn has_synthesized_uuid(m: &ClaudeMessage) -> bool {
    m.uuid
        .rsplit_once("-line-")
        .is_some_and(|(head, line)| head.len() == 36 && line.parse::<u64>().is_ok())
}

/// Deterministic dedup key for a message at a stable position in its session.
///
/// The timestamp is left out when the uuid was synthesized: there it is a
/// parse-time `now()`, so hashing it gave every re-parse of a growing file a
/// fresh key for every state record, and the hub stored each one again
/// (17.96M of 19.32M archived rows on 2026-09-23). Position, type and content
/// still identify the record.
pub fn message_key(provider: &str, session_id: &str, seq: i32, m: &ClaudeMessage) -> String {
    let timestamp = if has_synthesized_uuid(m) {
        ""
    } else {
        m.timestamp.as_str()
    };
    let mut h = Sha256::new();
    for field in [provider, session_id, timestamp, &m.message_type] {
        h.update(field.as_bytes());
        h.update([0]);
    }
    h.update(seq.to_le_bytes());
    if let Some(c) = &m.content {
        h.update(serde_json::to_vec(c).unwrap_or_default());
        h.update([1]);
    }
    if let Some(t) = &m.tool_use {
        h.update(serde_json::to_vec(t).unwrap_or_default());
        h.update([2]);
    }
    if let Some(r) = &m.tool_use_result {
        h.update(serde_json::to_vec(r).unwrap_or_default());
        h.update([3]);
    }
    let digest = h.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        let _ = write!(out, "{b:02x}");
    }
    out
}

pub fn to_ingest_message(
    provider: &str,
    session_id: &str,
    seq: i32,
    m: &ClaudeMessage,
) -> IngestMessage {
    let (input, output, cache_creation, cache_read) =
        m.usage.as_ref().map_or((None, None, None, None), |u| {
            (
                token(u.input_tokens),
                token(u.output_tokens),
                token(u.cache_creation_input_tokens),
                token(u.cache_read_input_tokens),
            )
        });
    IngestMessage {
        provider: provider.to_string(),
        session_id: session_id.to_string(),
        message_key: message_key(provider, session_id, seq, m),
        uuid: nonempty(&m.uuid),
        parent_uuid: m.parent_uuid.clone(),
        seq,
        timestamp: nonempty(&m.timestamp),
        message_type: nonempty(&m.message_type),
        role: m.role.clone(),
        model: m.model.clone(),
        stop_reason: m.stop_reason.clone(),
        input_tokens: input,
        output_tokens: output,
        cache_creation_tokens: cache_creation,
        cache_read_tokens: cache_read,
        cost_usd: m.cost_usd,
        duration_ms: m.duration_ms.and_then(|d| i64::try_from(d).ok()),
        is_sidechain: m.is_sidechain.unwrap_or(false),
        content: m.content.clone(),
        // MVP: `raw` is the normalized record (lossless for all modeled data).
        // Byte-exact original passthrough is a documented future enhancement.
        raw: serde_json::to_value(m).unwrap_or(serde_json::Value::Null),
        search_text: Some(clamped_search_text(m)),
    }
}

/// The hub clamps `search_text` at ingest; sending more than it will keep is
/// pure wire waste — a 40 MiB tool result would triple-ship (raw + content +
/// `search_text`) and blow the body limit.
///
/// Must match `hub::ingest::SEARCH_TEXT_MAX_BYTES` (64 KiB since 2026-07-19,
/// when the binding cost turned out to be GIN maintenance rather than the
/// tsvector size limit — see the note there). The hub re-clamps regardless, so
/// a stale daemon is only wasteful, never wrong; keeping the two equal means
/// the bytes are dropped before they cross the network.
const SEARCH_TEXT_MAX_BYTES: usize = 64 * 1024;

fn clamped_search_text(m: &ClaudeMessage) -> String {
    let mut text = history_core::search_text::search_text(m);
    if text.len() > SEARCH_TEXT_MAX_BYTES {
        let mut end = SEARCH_TEXT_MAX_BYTES;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

pub fn to_ingest_session(
    provider: &str,
    project_path: &str,
    session_id: &str,
    s: &ClaudeSession,
) -> IngestSession {
    IngestSession {
        provider: provider.to_string(),
        session_id: session_id.to_string(),
        project_path: Some(project_path.to_string()),
        file_path: nonempty(&s.file_path),
        entrypoint: s.entrypoint.clone(),
        summary: s.summary.clone(),
        message_count: i32::try_from(s.message_count).ok(),
        first_message_time: nonempty(&s.first_message_time),
        last_message_time: nonempty(&s.last_message_time),
        last_modified: nonempty(&s.last_modified),
        has_tool_use: Some(s.has_tool_use),
        has_errors: Some(s.has_errors),
        storage_type: s.storage_type.clone(),
    }
}

pub fn to_ingest_project(
    provider: &str,
    project_path: &str,
    p: &ClaudeProject,
    git: Option<&GitFingerprint>,
) -> IngestProject {
    IngestProject {
        provider: provider.to_string(),
        project_path: project_path.to_string(),
        name: nonempty(&p.name),
        storage_type: p.storage_type.clone(),
        session_count: i32::try_from(p.session_count).ok(),
        message_count: i32::try_from(p.message_count).ok(),
        last_modified: nonempty(&p.last_modified),
        git_root_commit: git.and_then(|g| g.root_commit.clone()),
        git_remote_url: git.and_then(|g| g.remote_url.clone()),
        git_is_worktree: git.map(|g| g.is_worktree),
        git_main_path: git.and_then(|g| g.main_path.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn attachment_record(payload: Option<serde_json::Value>) -> ClaudeMessage {
        let line = json!({
            "uuid": "uuid-a",
            "parentUuid": "uuid-1",
            "sessionId": "session-1",
            "timestamp": "2026-10-10T00:00:00Z",
            "type": "attachment",
        });
        let mut m: ClaudeMessage = serde_json::from_value(line).unwrap();
        m.attachment = payload;
        m
    }

    /// #46: the payload reaches the hub inside `raw`…
    #[test]
    fn attachment_payload_is_archived_in_raw() {
        let payload = json!({ "type": "hook_additional_context", "content": ["x"] });
        let ingest = to_ingest_message("claude", "s", 3, &attachment_record(Some(payload.clone())));
        assert_eq!(ingest.raw["attachment"], payload);
        assert!(ingest.content.is_none());
    }

    /// …but never into the dedup key. Records archived before #46 were keyed
    /// without a payload; if the payload were hashed, the next re-parse of the
    /// same file would give each of them a new key and the hub would store all
    /// of them again (741,137 attachment rows on pg1 at 2026-10-01).
    #[test]
    fn attachment_payload_does_not_change_message_key() {
        let without = attachment_record(None);
        let with = attachment_record(Some(json!({ "type": "file", "content": "y" })));
        assert_eq!(
            message_key("claude", "s", 3, &without),
            message_key("claude", "s", 3, &with)
        );
        // Control: the key does react to a field it hashes.
        assert_ne!(
            message_key("claude", "s", 3, &without),
            message_key("claude", "s", 4, &without)
        );
    }
}
