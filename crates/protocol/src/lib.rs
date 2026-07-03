//! Wire contract for daemon→hub ingestion.
//!
//! Shared by the sync daemon (producer) and the hub (consumer). These are
//! explicit, self-contained types — deliberately decoupled from
//! `history_core`'s internal normalized models — so the on-the-wire format can
//! evolve independently and carries the archive-only fields (`raw`,
//! `search_text`, `message_key`) that the internal model does not retain.
//!
//! Timestamps are RFC 3339 strings (matching what the providers emit); the hub
//! parses them leniently into `timestamptz`, storing NULL when absent/invalid.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A batch of history records pushed by one machine to `POST /v1/ingest`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestBatch {
    /// Self-reported machine identity. The hub validates `machine.machine_id`
    /// against the bearer token's mapped id and rejects a mismatch.
    pub machine: MachineInfo,
    #[serde(default)]
    pub projects: Vec<IngestProject>,
    #[serde(default)]
    pub sessions: Vec<IngestSession>,
    #[serde(default)]
    pub messages: Vec<IngestMessage>,
}

/// Provenance for the machines table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineInfo {
    pub machine_id: Uuid,
    pub hostname: String,
    #[serde(default)]
    pub os: Option<String>,
}

/// A provider project/workspace, keyed by (provider, `project_path`) on this machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestProject {
    pub provider: String,
    pub project_path: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub storage_type: Option<String>,
    #[serde(default)]
    pub session_count: Option<i32>,
    #[serde(default)]
    pub message_count: Option<i32>,
    #[serde(default)]
    pub last_modified: Option<String>,
}

/// A session, keyed by (provider, `session_id`) on this machine, linked to a
/// project by `project_path`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestSession {
    pub provider: String,
    pub session_id: String,
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub entrypoint: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub message_count: Option<i32>,
    #[serde(default)]
    pub first_message_time: Option<String>,
    #[serde(default)]
    pub last_message_time: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    #[serde(default)]
    pub has_tool_use: Option<bool>,
    #[serde(default)]
    pub has_errors: Option<bool>,
    #[serde(default)]
    pub storage_type: Option<String>,
}

/// A single message. `session_id` is the provider's session id (used to resolve
/// the hub's surrogate session row). `message_key` is the provider message UUID
/// when present, otherwise a content-derived key — it is the dedup key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestMessage {
    pub provider: String,
    pub session_id: String,
    pub message_key: String,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub parent_uuid: Option<String>,
    #[serde(default)]
    pub seq: i32,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(rename = "type", default)]
    pub message_type: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub input_tokens: Option<i64>,
    #[serde(default)]
    pub output_tokens: Option<i64>,
    #[serde(default)]
    pub cache_creation_tokens: Option<i64>,
    #[serde(default)]
    pub cache_read_tokens: Option<i64>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub is_sidechain: bool,
    /// Normalized content (as produced by history-core).
    #[serde(default)]
    pub content: Option<serde_json::Value>,
    /// EXACT original record — the fidelity guarantee.
    pub raw: serde_json::Value,
    /// Flattened plaintext for full-text search.
    #[serde(default)]
    pub search_text: Option<String>,
}

/// Per-batch counts returned by `/v1/ingest`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngestResponse {
    pub projects_inserted: u64,
    pub projects_skipped: u64,
    pub sessions_inserted: u64,
    pub sessions_skipped: u64,
    pub messages_inserted: u64,
    pub messages_skipped: u64,
}
