//! Flatten a normalized message into plaintext for full-text search.
//!
//! Ported from the frontend `contentExtractor`/`extractSearchableText` idea: walk
//! the message's content, tool call, and tool result, collecting human-readable
//! text while skipping structural/metadata keys (ids, types, signatures) that
//! would only add noise to the search index.

use crate::models::ClaudeMessage;
use serde_json::Value;

/// JSON object keys whose values are structural/metadata, not searchable prose.
const SKIP_KEYS: &[&str] = &[
    "type",
    "id",
    "uuid",
    "parentUuid",
    "sessionId",
    "tool_use_id",
    "toolUseID",
    "toolUseId",
    "signature",
    "role",
    "model",
    "stop_reason",
    "index",
    "cache_control",
    "source", // base64 image payloads etc.
    "data",   // base64 blobs
];

/// Build the flattened search text for a message.
#[must_use]
pub fn search_text(message: &ClaudeMessage) -> String {
    let mut out = String::new();
    if let Some(c) = &message.content {
        collect(c, &mut out);
    }
    if let Some(t) = &message.tool_use {
        collect(t, &mut out);
    }
    if let Some(r) = &message.tool_use_result {
        collect(r, &mut out);
    }
    // Collapse runs of whitespace so the index stays compact.
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect(value: &Value, out: &mut String) {
    match value {
        Value::String(s) => push(out, s),
        Value::Array(items) => {
            for item in items {
                collect(item, out);
            }
        }
        Value::Object(map) => {
            for (key, val) in map {
                if SKIP_KEYS.contains(&key.as_str()) {
                    continue;
                }
                collect(val, out);
            }
        }
        // Numbers/bools/null carry no searchable prose.
        _ => {}
    }
}

fn push(out: &mut String, s: &str) {
    let s = s.trim();
    if s.is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(s);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Build via serde (robust to the many optional fields on ClaudeMessage).
    fn msg(
        content: Value,
        tool_use: Option<Value>,
        tool_use_result: Option<Value>,
    ) -> ClaudeMessage {
        let mut obj = json!({
            "uuid": "u",
            "sessionId": "s",
            "timestamp": "2026-01-01T00:00:00Z",
            "type": "user",
            "content": content,
        });
        if let Some(t) = tool_use {
            obj["toolUse"] = t;
        }
        if let Some(r) = tool_use_result {
            obj["toolUseResult"] = r;
        }
        serde_json::from_value(obj).expect("construct ClaudeMessage")
    }

    #[test]
    fn plain_string_content() {
        let m = msg(json!("hello world"), None, None);
        assert_eq!(search_text(&m), "hello world");
    }

    #[test]
    fn array_of_text_blocks() {
        let m = msg(
            json!([
                { "type": "text", "text": "first block" },
                { "type": "text", "text": "second block" }
            ]),
            None,
            None,
        );
        assert_eq!(search_text(&m), "first block second block");
    }

    #[test]
    fn skips_metadata_keys() {
        // `type`, `id`, `signature` must not appear in the output.
        let m = msg(
            json!([{ "type": "thinking", "thinking": "deep thought", "signature": "abc123sig" }]),
            None,
            None,
        );
        let t = search_text(&m);
        assert!(t.contains("deep thought"));
        assert!(!t.contains("thinking")); // the key/type, not present as a value
        assert!(!t.contains("abc123sig"));
    }

    #[test]
    fn includes_tool_use_and_result() {
        let m = msg(
            json!("run it"),
            Some(json!({ "name": "Bash", "input": { "command": "ls -la" } })),
            Some(json!({ "stdout": "total 0", "stderr": "" })),
        );
        let t = search_text(&m);
        assert!(t.contains("run it"));
        assert!(t.contains("Bash"));
        assert!(t.contains("ls -la"));
        assert!(t.contains("total 0"));
    }

    #[test]
    fn collapses_whitespace() {
        let m = msg(json!("  lots   of\n\n  space  "), None, None);
        assert_eq!(search_text(&m), "lots of space");
    }
}
