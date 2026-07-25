//! Derived-field extraction performed once, at ingest.
//!
//! Analytics rollups run over the whole archive, so anything they need must be
//! a queryable column or row — not a JSONB path evaluated per query. This module
//! is the single extraction implementation, shared by live ingest and the
//! backfill of already-stored rows (openspec `hub-analytics` design D4), so the
//! two can never disagree.
//!
//! Everything here reads the NORMALIZED record the daemon sends, not the
//! original provider line.

use serde_json::Value;

/// The provider's response id for a message, or `None`.
///
/// Read from `raw->>"messageId"`. Two things about that path are easy to get
/// wrong:
///
/// 1. **`raw` is flat.** The daemon sets `raw = to_value(ClaudeMessage)` — a
///    normalized record — so the provider's nested `message.id` does not exist
///    here. `raw->'message'->>'id'` yields NULL for every row.
/// 2. **The key is camelCase.** `ClaudeMessage::message_id` carries
///    `#[serde(rename = "messageId")]`.
///
/// The field is deliberately dual-purpose: `TryFrom<RawLogEntry>` computes
/// `msg.id.clone().or(log_entry.message_id)`, so an assistant `msg_…` id wins
/// and the file-history-snapshot `messageId` is the fallback. Usage dedup keys
/// on this exact precedence, matching the desktop oracle — it must not be
/// "cleaned up" into two separate fields. Snapshot rows carry no `usage`, so
/// they cannot perturb token totals.
pub fn message_id(raw: &Value) -> Option<String> {
    raw.get("messageId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// One tool invocation extracted from a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolUseRow {
    /// Ordinal within the message; with the message ref this is the idempotence
    /// key, so re-ingest overwrites rather than accumulates.
    pub seq: i32,
    pub tool_name: String,
    /// The `tool_use` item's own id, used to join to the outcome that reports on
    /// it. `None` for the top-level `toolUse` shape, whose result rides the same
    /// record and so needs no join.
    pub tool_use_id: Option<String>,
    /// `input.skill` when the tool is `Skill` (issue #321).
    pub skill_name: Option<String>,
    /// `input.subagent_type` when the tool is `Agent` (issue #321).
    pub subagent_type: Option<String>,
    /// Only ever true for the top-level shape. A content-array `tool_use` item
    /// does NOT carry its outcome — see [`tool_results`].
    pub is_error: bool,
}

/// One tool outcome extracted from a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResultRow {
    pub seq: i32,
    /// The invocation this reports on.
    pub tool_use_id: String,
    pub is_error: bool,
}

/// Read `input.<key>` as a non-empty string, when the item names `tool_name`.
fn input_str(item: &Value, tool_name: &str, key: &str) -> Option<String> {
    if item.get("name").and_then(Value::as_str) != Some(tool_name) {
        return None;
    }
    item.get("input")
        .and_then(|input| input.get(key))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Tool invocations in a message.
///
/// Two shapes carry invocations, mirroring the oracle's two extraction paths:
///
/// - **Content array** (assistant messages only): items with
///   `type == "tool_use"`, carrying `id`, `name` and `input`. Their outcome
///   arrives later, via [`tool_results`].
/// - **Top-level `toolUse`**: a `{name}` object on the record itself, whose
///   outcome (`toolUseResult.is_error`) rides the same record.
pub fn tool_uses(
    message_type: Option<&str>,
    content: Option<&Value>,
    raw: &Value,
) -> Vec<ToolUseRow> {
    let mut rows = Vec::new();
    // Explicit ordinal rather than `rows.len() as i32`: the column is int4, and
    // a silent wrap is not a failure mode worth inheriting from a cast.
    let mut seq: i32 = 0;

    // Content-array path. Gated on assistant messages exactly as the oracle
    // gates it, so counts line up.
    if message_type == Some("assistant") {
        if let Some(items) = content.and_then(Value::as_array) {
            for item in items {
                if item.get("type").and_then(Value::as_str) != Some("tool_use") {
                    continue;
                }
                let Some(name) = item.get("name").and_then(Value::as_str) else {
                    continue;
                };
                rows.push(ToolUseRow {
                    seq,
                    tool_name: name.to_owned(),
                    tool_use_id: item
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                    skill_name: input_str(item, "Skill", "skill"),
                    subagent_type: input_str(item, "Agent", "subagent_type"),
                    // Never present on a tool_use item; the outcome is a
                    // separate, later tool_result.
                    is_error: false,
                });
                seq += 1;
            }
        }
    }

    // Top-level shape — a FALLBACK, not an addition.
    //
    // On Claude records it is a redundant restatement of the content-array
    // invocation on the SAME record, so emitting both would double every tool
    // count. Measured on pg1 (2% sample of 2.64M messages, 2026-07-25): of the
    // 2,551 assistant rows carrying both shapes, the top-level name matched an
    // array `tool_use` name in 2,551 — 100%, zero divergences — and every one of
    // those rows held exactly one array item. The desktop oracle runs both paths
    // unconditionally and therefore double-counts; that is not worth
    // reproducing (same reasoning as D10).
    //
    // Kept as a fallback so records that carry ONLY this shape still count.
    if rows.is_empty() {
        if let Some(name) = raw
            .get("toolUse")
            .and_then(|tu| tu.get("name"))
            .and_then(Value::as_str)
        {
            rows.push(ToolUseRow {
                seq,
                tool_name: name.to_owned(),
                tool_use_id: None,
                skill_name: None,
                subagent_type: None,
                is_error: raw
                    .get("toolUseResult")
                    .and_then(|r| r.get("is_error"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            });
        }
    }

    rows
}

/// Tool outcomes in a message.
///
/// `tool_result` items live in the *user* message that follows the invocation,
/// referencing it by `tool_use_id`. This is why success rate cannot be read off
/// an invocation: the desktop oracle tries to (`stats.rs:560`), finds nothing,
/// and scores every content-array invocation as a success.
///
/// Not gated on message type — a `tool_result` item is self-identifying, and
/// providers differ on which role carries it.
pub fn tool_results(content: Option<&Value>) -> Vec<ToolResultRow> {
    let mut rows = Vec::new();
    let mut seq: i32 = 0;
    let Some(items) = content.and_then(Value::as_array) else {
        return rows;
    };
    for item in items {
        if item.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        let Some(id) = item
            .get("tool_use_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        rows.push(ToolResultRow {
            seq,
            tool_use_id: id.to_owned(),
            is_error: item
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
        seq += 1;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn message_id_reads_flat_camel_case_key() {
        let raw = json!({ "messageId": "msg_01ABC", "type": "assistant" });
        assert_eq!(message_id(&raw), Some("msg_01ABC".to_owned()));
    }

    #[test]
    fn message_id_absent_when_nested_provider_shape_is_used() {
        // The pre-normalization shape. `raw` is never this, and reading it as if
        // it were is the mistake this test pins down.
        let raw = json!({ "message": { "id": "msg_01ABC" } });
        assert_eq!(message_id(&raw), None);
    }

    #[test]
    fn message_id_treats_empty_as_absent() {
        assert_eq!(message_id(&json!({ "messageId": "" })), None);
        assert_eq!(message_id(&json!({})), None);
    }

    #[test]
    fn no_tool_use_yields_no_rows() {
        let content = json!([{ "type": "text", "text": "hello" }]);
        assert!(tool_uses(Some("assistant"), Some(&content), &json!({})).is_empty());
        assert!(tool_results(Some(&content)).is_empty());
    }

    #[test]
    fn multiple_invocations_in_one_message_are_numbered() {
        let content = json!([
            { "type": "tool_use", "id": "toolu_1", "name": "Read", "input": {} },
            { "type": "text", "text": "then" },
            { "type": "tool_use", "id": "toolu_2", "name": "Bash", "input": {} },
        ]);
        let rows = tool_uses(Some("assistant"), Some(&content), &json!({}));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].seq, 0);
        assert_eq!(rows[0].tool_name, "Read");
        assert_eq!(rows[1].seq, 1);
        assert_eq!(rows[1].tool_name, "Bash");
        assert_eq!(rows[1].tool_use_id, Some("toolu_2".to_owned()));
    }

    #[test]
    fn skill_invocations_carry_the_skill_name() {
        let content = json!([{
            "type": "tool_use", "id": "toolu_1", "name": "Skill",
            "input": { "skill": "cchv-find" }
        }]);
        let rows = tool_uses(Some("assistant"), Some(&content), &json!({}));
        assert_eq!(rows[0].skill_name, Some("cchv-find".to_owned()));
        assert_eq!(rows[0].subagent_type, None);
    }

    #[test]
    fn agent_invocations_carry_the_subagent_type() {
        let content = json!([{
            "type": "tool_use", "id": "toolu_1", "name": "Agent",
            "input": { "subagent_type": "Explore" }
        }]);
        let rows = tool_uses(Some("assistant"), Some(&content), &json!({}));
        assert_eq!(rows[0].subagent_type, Some("Explore".to_owned()));
        assert_eq!(rows[0].skill_name, None);
    }

    #[test]
    fn content_array_invocations_are_never_self_flagged_as_errors() {
        // The oracle's bug, pinned: `is_error` is not a tool_use field. Even if
        // one appears, the outcome must come from the tool_result.
        let content = json!([{
            "type": "tool_use", "id": "toolu_1", "name": "Bash",
            "input": {}, "is_error": true
        }]);
        let rows = tool_uses(Some("assistant"), Some(&content), &json!({}));
        assert!(!rows[0].is_error);
    }

    #[test]
    fn non_assistant_messages_contribute_no_content_array_invocations() {
        let content = json!([{ "type": "tool_use", "id": "t", "name": "Read", "input": {} }]);
        assert!(tool_uses(Some("user"), Some(&content), &json!({})).is_empty());
    }

    #[test]
    fn top_level_tool_use_resolves_its_error_on_the_same_record() {
        let raw = json!({
            "toolUse": { "name": "Bash" },
            "toolUseResult": { "is_error": true }
        });
        let rows = tool_uses(Some("user"), None, &raw);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tool_name, "Bash");
        assert!(rows[0].is_error);
        assert_eq!(rows[0].tool_use_id, None);
    }

    #[test]
    fn top_level_tool_use_without_result_is_a_success() {
        let raw = json!({ "toolUse": { "name": "Bash" } });
        let rows = tool_uses(Some("user"), None, &raw);
        assert!(!rows[0].is_error);
    }

    #[test]
    fn top_level_shape_is_suppressed_when_the_content_array_already_counted_it() {
        // Measured on pg1: when both shapes are present the top-level name
        // always restates the single array invocation. Counting both doubles
        // every Claude tool count — the oracle's behavior, not reproduced here.
        let content = json!([{ "type": "tool_use", "id": "toolu_1", "name": "Read", "input": {} }]);
        let raw = json!({ "toolUse": { "name": "Read" } });
        let rows = tool_uses(Some("assistant"), Some(&content), &raw);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tool_name, "Read");
        assert_eq!(rows[0].tool_use_id, Some("toolu_1".to_owned()));
    }

    #[test]
    fn top_level_shape_still_counts_when_there_is_no_content_array_invocation() {
        let content = json!([{ "type": "text", "text": "x" }]);
        let raw = json!({ "toolUse": { "name": "Bash" } });
        let rows = tool_uses(Some("assistant"), Some(&content), &raw);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tool_name, "Bash");
    }

    #[test]
    fn tool_results_are_extracted_with_their_error_status() {
        let content = json!([
            { "type": "tool_result", "tool_use_id": "toolu_1", "content": "ok" },
            { "type": "tool_result", "tool_use_id": "toolu_2", "is_error": true },
        ]);
        let rows = tool_results(Some(&content));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].tool_use_id, "toolu_1");
        assert!(!rows[0].is_error);
        assert_eq!(rows[1].tool_use_id, "toolu_2");
        assert!(rows[1].is_error);
        assert_eq!(rows[1].seq, 1);
    }

    #[test]
    fn tool_results_without_an_invocation_id_are_skipped() {
        let content = json!([
            { "type": "tool_result", "is_error": true },
            { "type": "tool_result", "tool_use_id": "", "is_error": true },
        ]);
        assert!(tool_results(Some(&content)).is_empty());
    }

    #[test]
    fn string_content_is_tolerated() {
        let content = json!("just a string");
        assert!(tool_uses(Some("assistant"), Some(&content), &json!({})).is_empty());
        assert!(tool_results(Some(&content)).is_empty());
    }
}
