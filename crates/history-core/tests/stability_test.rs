//! Determinism (task 1.8): parsing the same session file twice yields identical
//! normalized output. This underpins the daemon's stable content-hash dedup
//! keys — re-parsing a file must not change the messages.

use std::io::Write;

use history_core::providers::claude;

fn write_fixture(dir: &std::path::Path) -> std::path::PathBuf {
    let file = dir.join("sess-1.jsonl");
    let mut f = std::fs::File::create(&file).unwrap();
    writeln!(
        f,
        r#"{{"uuid":"u1","sessionId":"sess-1","timestamp":"2026-01-01T00:00:00Z","type":"user","cwd":"/Users/test/proj","message":{{"role":"user","content":"hello quick fox"}}}}"#
    )
    .unwrap();
    writeln!(
        f,
        r#"{{"uuid":"u2","parentUuid":"u1","sessionId":"sess-1","timestamp":"2026-01-01T00:01:00Z","type":"assistant","message":{{"role":"assistant","model":"claude-x","content":[{{"type":"text","text":"hi there"}}]}}}}"#
    )
    .unwrap();
    file
}

#[test]
fn load_messages_is_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let file = write_fixture(dir.path());
    let path = file.to_string_lossy();

    let first = claude::load_messages(&path).expect("first parse");
    let second = claude::load_messages(&path).expect("second parse");

    assert_eq!(first.len(), 2);
    // Compare via JSON so the test does not depend on ClaudeMessage: PartialEq.
    let a = serde_json::to_value(&first).unwrap();
    let b = serde_json::to_value(&second).unwrap();
    assert_eq!(a, b, "re-parsing the same file must be identical");
}
