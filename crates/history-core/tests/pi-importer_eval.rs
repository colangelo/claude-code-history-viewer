//! Acceptance evals for the Pi coding-agent provider (spec: "Pi coding agent provider").
//!
//! These evals target the *dynamic* provider-registry surface
//! (`ProviderId::parse`/`as_str`/`display_name`, `providers::scan_all_projects`,
//! `providers::load_sessions`, `providers::load_messages`) rather than a
//! not-yet-existing `pi` module, so this file compiles against the unmodified
//! crate today and only fails at runtime (no `pi` provider is registered yet).
//!
//! Fixture stores are built under a fresh `TempDir` per test and pointed to via
//! `HOME`, mirroring the `~/.pi/agent/sessions/<escaped-cwd>/<file>.jsonl` layout
//! described in the spec. `HOME` is process-global, so every test uses a
//! `HomeGuard` and the crate's tests already run single-threaded
//! (`--test-threads=1`, see `.config/nextest.toml` / RUNBOOK).

use history_core::providers::{self, ProviderId};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Saves/restores `HOME` around a test so fixture stores under a `TempDir`
/// are picked up by providers that resolve their default location from it.
struct HomeGuard {
    original: Option<String>,
}

impl HomeGuard {
    fn set(path: &Path) -> Self {
        let original = std::env::var("HOME").ok();
        std::env::set_var("HOME", path);
        Self { original }
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}

fn pi_sessions_root(home: &Path) -> PathBuf {
    home.join(".pi").join("agent").join("sessions")
}

fn write_session_file(root: &Path, dir_name: &str, file_name: &str, lines: &[String]) -> PathBuf {
    let dir = root.join(dir_name);
    fs::create_dir_all(&dir).expect("create pi project dir");
    let file = dir.join(file_name);
    let mut f = fs::File::create(&file).expect("create pi session file");
    for line in lines {
        writeln!(f, "{line}").expect("write pi session line");
    }
    file
}

fn header_line(id: &str, cwd: &str, timestamp: &str) -> String {
    format!(
        r#"{{"type":"session","version":3,"id":"{id}","timestamp":"{timestamp}","cwd":"{cwd}"}}"#
    )
}

fn model_change_line(id: &str, parent_id: &str, timestamp: &str) -> String {
    format!(
        r#"{{"type":"model_change","id":"{id}","parentId":"{parent_id}","timestamp":"{timestamp}","provider":"anthropic","modelId":"claude-opus-4-8"}}"#
    )
}

fn thinking_level_change_line(id: &str, parent_id: &str, timestamp: &str) -> String {
    format!(
        r#"{{"type":"thinking_level_change","id":"{id}","parentId":"{parent_id}","timestamp":"{timestamp}","thinkingLevel":"high"}}"#
    )
}

fn user_message_line(
    id: &str,
    parent_id: &str,
    timestamp: &str,
    text: &str,
    epoch_ms: u64,
) -> String {
    format!(
        r#"{{"type":"message","id":"{id}","parentId":"{parent_id}","timestamp":"{timestamp}","message":{{"role":"user","content":[{{"type":"text","text":"{text}"}}],"timestamp":{epoch_ms}}}}}"#
    )
}

fn assistant_message_line(
    id: &str,
    parent_id: &str,
    timestamp: &str,
    thinking: &str,
    text: &str,
    epoch_ms: u64,
) -> String {
    format!(
        r#"{{"type":"message","id":"{id}","parentId":"{parent_id}","timestamp":"{timestamp}","message":{{"role":"assistant","api":"anthropic-messages","provider":"anthropic","model":"claude-opus-4-8","stopReason":"stop","content":[{{"type":"thinking","thinking":"{thinking}","thinkingSignature":"sig-123"}},{{"type":"text","text":"{text}"}}],"usage":{{"input":1200,"output":340,"cacheRead":800,"cacheWrite":0,"totalTokens":1540,"cost":{{"total":0.012}}}},"timestamp":{epoch_ms}}}}}"#
    )
}

fn assistant_error_message_line(
    id: &str,
    parent_id: &str,
    timestamp: &str,
    error_message: &str,
    epoch_ms: u64,
) -> String {
    format!(
        r#"{{"type":"message","id":"{id}","parentId":"{parent_id}","timestamp":"{timestamp}","message":{{"role":"assistant","api":"anthropic-messages","provider":"anthropic","model":"claude-opus-4-8","stopReason":"error","errorMessage":"{error_message}","content":[],"usage":{{"input":100,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":100,"cost":{{"total":0.001}}}},"timestamp":{epoch_ms}}}}}"#
    )
}

/// AC1: scanning a Pi session store yields one project per session subdirectory,
/// and each project's real path comes from the session header's `cwd` field —
/// never decoded from the (arbitrarily escaped) directory name.
#[test]
fn ac1_scan_yields_one_project_per_session_dir_with_cwd_from_header() {
    let home = tempfile::tempdir().expect("tempdir");
    let _guard = HomeGuard::set(home.path());
    let root = pi_sessions_root(home.path());

    // Directory names are deliberately WRONG decodings of the real cwd, to
    // prove the provider must read `cwd` from the header, not the dir name.
    write_session_file(
        &root,
        "--Users-ac-dev-wrong-name-one--",
        "2026-06-08T20-31-45-261Z_019ea8ef-246d-76ee-9e21-449fc7bfade9.jsonl",
        &[
            header_line(
                "019ea8ef-246d-76ee-9e21-449fc7bfade9",
                "/Users/ac/dev/real-project-one",
                "2026-06-08T20:31:45.261Z",
            ),
            user_message_line(
                "u1",
                "019ea8ef-246d-76ee-9e21-449fc7bfade9",
                "2026-06-08T20:31:50.000Z",
                "Please refactor the auth module",
                1749412310000,
            ),
        ],
    );
    write_session_file(
        &root,
        "--Users-ac-dev-wrong-name-two--",
        "2026-06-09T09-00-00-000Z_019ea9aa-246d-76ee-9e21-449fc7bfadea.jsonl",
        &[
            header_line(
                "019ea9aa-246d-76ee-9e21-449fc7bfadea",
                "/Users/ac/dev/real-project-two",
                "2026-06-09T09:00:00.000Z",
            ),
            user_message_line(
                "u1",
                "019ea9aa-246d-76ee-9e21-449fc7bfadea",
                "2026-06-09T09:00:05.000Z",
                "Add a health check endpoint",
                1749460805000,
            ),
        ],
    );

    let projects = providers::scan_all_projects();
    let pi_projects: Vec<_> = projects
        .into_iter()
        .filter(|p| p.provider.as_deref() == Some("pi"))
        .collect();

    assert_eq!(
        pi_projects.len(),
        2,
        "expected one Pi project per session subdirectory"
    );

    let mut actual_paths: Vec<String> = pi_projects.iter().map(|p| p.actual_path.clone()).collect();
    actual_paths.sort();
    assert_eq!(
        actual_paths,
        vec![
            "/Users/ac/dev/real-project-one".to_string(),
            "/Users/ac/dev/real-project-two".to_string(),
        ],
        "project real path must come from the session header's cwd, not the escaped directory name"
    );
}

/// AC2: listing a Pi project's sessions yields, per JSONL file, a summary
/// derived from the first user text message, a message count that counts only
/// `type:"message"` records (excluding `header`/`model_change`/`thinking_level_change`),
/// and session timestamps.
#[test]
fn ac2_load_sessions_reports_summary_message_count_and_timestamps() {
    let home = tempfile::tempdir().expect("tempdir");
    let _guard = HomeGuard::set(home.path());
    let root = pi_sessions_root(home.path());

    write_session_file(
        &root,
        "--Users-ac-dev-project-a--",
        "2026-06-08T20-31-45-261Z_019ea8ef-246d-76ee-9e21-449fc7bfade9.jsonl",
        &[
            header_line(
                "019ea8ef-246d-76ee-9e21-449fc7bfade9",
                "/Users/ac/dev/project-a",
                "2026-06-08T20:31:45.261Z",
            ),
            model_change_line(
                "m1",
                "019ea8ef-246d-76ee-9e21-449fc7bfade9",
                "2026-06-08T20:31:46.000Z",
            ),
            thinking_level_change_line("t1", "m1", "2026-06-08T20:31:46.500Z"),
            user_message_line(
                "u1",
                "t1",
                "2026-06-08T20:31:50.000Z",
                "Please refactor the auth module",
                1749412310000,
            ),
            assistant_message_line(
                "a1",
                "u1",
                "2026-06-08T20:32:10.000Z",
                "Let me look at the auth module first.",
                "I refactored the auth module.",
                1749412330000,
            ),
        ],
    );

    let projects = providers::scan_all_projects();
    let pi_provider = ProviderId::parse("pi").expect("pi provider must be registered");
    let project = projects
        .into_iter()
        .find(|p| p.provider.as_deref() == Some("pi"))
        .expect("expected a Pi project to be scanned");

    let sessions = providers::load_sessions(pi_provider, &project.path, false)
        .expect("loading Pi sessions must not error");

    assert_eq!(sessions.len(), 1, "expected exactly one session");
    let session = &sessions[0];

    assert_eq!(
        session.message_count, 2,
        "message count must include only type:\"message\" records (header/model_change/thinking_level_change excluded)"
    );
    let summary = session
        .summary
        .as_ref()
        .expect("summary must be derived from the first user text message");
    assert!(
        summary.contains("refactor the auth module"),
        "summary should reflect the first user text message, got: {summary}"
    );
    assert!(
        !session.first_message_time.is_empty(),
        "first_message_time must be populated from the session"
    );
    assert!(
        !session.last_message_time.is_empty(),
        "last_message_time must be populated from the session"
    );
}

/// AC3: loading a Pi session's messages maps user/assistant text to normalized
/// content, assistant thinking items become thinking content, and assistant
/// model + usage (input/output/cacheRead/cacheWrite) map into normalized
/// message metadata and token-usage fields.
#[test]
fn ac3_load_messages_maps_text_thinking_model_and_usage() {
    let home = tempfile::tempdir().expect("tempdir");
    let _guard = HomeGuard::set(home.path());
    let root = pi_sessions_root(home.path());

    let file = write_session_file(
        &root,
        "--Users-ac-dev-project-b--",
        "2026-06-08T20-31-45-261Z_019ea8ef-246d-76ee-9e21-449fc7bfadeb.jsonl",
        &[
            header_line(
                "019ea8ef-246d-76ee-9e21-449fc7bfadeb",
                "/Users/ac/dev/project-b",
                "2026-06-08T20:31:45.261Z",
            ),
            user_message_line(
                "u1",
                "019ea8ef-246d-76ee-9e21-449fc7bfadeb",
                "2026-06-08T20:31:50.000Z",
                "Please refactor the auth module",
                1749412310000,
            ),
            assistant_message_line(
                "a1",
                "u1",
                "2026-06-08T20:32:10.000Z",
                "Let me look at the auth module first.",
                "I refactored the auth module.",
                1749412330000,
            ),
        ],
    );

    // Force the registry to prove it can route to the Pi module (AC5 also
    // covers this directly; here we just need a valid ProviderId to reuse the
    // registry's dispatch for load_messages).
    let _pi_provider = ProviderId::parse("pi").expect("pi provider must be registered");

    let messages = providers::load_messages(
        ProviderId::parse("pi").expect("pi provider must be registered"),
        &file.to_string_lossy(),
    )
    .expect("loading Pi messages must not error");

    assert_eq!(
        messages.len(),
        2,
        "expected the user and assistant message records (header excluded)"
    );

    let user_msg = messages
        .iter()
        .find(|m| m.role.as_deref() == Some("user"))
        .expect("expected a normalized user message");
    let user_content = serde_json::to_string(&user_msg.content).unwrap_or_default();
    assert!(
        user_content.contains("Please refactor the auth module"),
        "user text must be mapped into normalized content, got: {user_content}"
    );

    let assistant_msg = messages
        .iter()
        .find(|m| m.role.as_deref() == Some("assistant"))
        .expect("expected a normalized assistant message");
    assert_eq!(
        assistant_msg.model.as_deref(),
        Some("claude-opus-4-8"),
        "assistant model must be mapped into message metadata"
    );

    let assistant_content = serde_json::to_string(&assistant_msg.content).unwrap_or_default();
    assert!(
        assistant_content.contains("Let me look at the auth module first."),
        "assistant thinking item must become thinking content, got: {assistant_content}"
    );
    assert!(
        assistant_content.contains("I refactored the auth module."),
        "assistant text item must be mapped into normalized content, got: {assistant_content}"
    );

    let usage = assistant_msg
        .usage
        .as_ref()
        .expect("assistant usage must be mapped into token-usage fields");
    assert_eq!(
        usage.input_tokens,
        Some(1200),
        "usage.input -> input_tokens"
    );
    assert_eq!(
        usage.output_tokens,
        Some(340),
        "usage.output -> output_tokens"
    );
    assert_eq!(
        usage.cache_read_input_tokens,
        Some(800),
        "usage.cacheRead -> cache_read_input_tokens"
    );
    assert_eq!(
        usage.cache_creation_input_tokens,
        Some(0),
        "usage.cacheWrite -> cache_creation_input_tokens"
    );
}

/// AC4: a session whose assistant turn has `stopReason:"error"` and an
/// `errorMessage` loads without a parse failure — the session and its other
/// messages are returned, and the failed turn carries an error indication.
#[test]
fn ac4_error_turn_loads_without_parse_failure_and_carries_error_indication() {
    let home = tempfile::tempdir().expect("tempdir");
    let _guard = HomeGuard::set(home.path());
    let root = pi_sessions_root(home.path());

    let file = write_session_file(
        &root,
        "--Users-ac-dev-project-c--",
        "2026-06-08T20-31-45-261Z_019ea8ef-246d-76ee-9e21-449fc7bfaded.jsonl",
        &[
            header_line(
                "019ea8ef-246d-76ee-9e21-449fc7bfaded",
                "/Users/ac/dev/project-c",
                "2026-06-08T20:31:45.261Z",
            ),
            user_message_line(
                "u1",
                "019ea8ef-246d-76ee-9e21-449fc7bfaded",
                "2026-06-08T20:31:50.000Z",
                "Deploy the service",
                1749412310000,
            ),
            assistant_error_message_line(
                "a1",
                "u1",
                "2026-06-08T20:32:10.000Z",
                "boom: connection reset",
                1749412330000,
            ),
            user_message_line(
                "u2",
                "a1",
                "2026-06-08T20:33:00.000Z",
                "Try again please",
                1749412380000,
            ),
        ],
    );

    let pi_provider = ProviderId::parse("pi").expect("pi provider must be registered");

    let projects = providers::scan_all_projects();
    let project = projects
        .into_iter()
        .find(|p| p.provider.as_deref() == Some("pi"))
        .expect("expected a Pi project to be scanned despite the error turn");
    let sessions = providers::load_sessions(pi_provider, &project.path, false)
        .expect("loading sessions for a session containing an error turn must not fail");
    assert_eq!(sessions.len(), 1);
    assert!(
        sessions[0].has_errors,
        "a session containing an errored assistant turn must be flagged has_errors"
    );

    let messages = providers::load_messages(pi_provider, &file.to_string_lossy()).expect(
        "loading messages for a session containing an error turn must not fail (no parse failure)",
    );
    assert_eq!(
        messages.len(),
        3,
        "the session's other messages (both user turns) must still be returned"
    );

    let error_turn = messages
        .iter()
        .find(|m| m.role.as_deref() == Some("assistant"))
        .expect("expected the errored assistant turn to be present");
    assert_eq!(
        error_turn.stop_reason.as_deref(),
        Some("error"),
        "the failed turn's stop_reason must indicate the error"
    );
    let error_repr = serde_json::to_string(error_turn).unwrap_or_default();
    assert!(
        error_repr.contains("boom: connection reset"),
        "the failed turn must carry the error message content, got: {error_repr}"
    );
}

/// AC5: `ProviderId::parse("pi")` returns `Some`, round-trips through
/// `as_str()` to `"pi"` and `display_name()` to `"Pi"`, and the registry
/// dispatch routes to the Pi module instead of erroring.
#[test]
fn ac5_provider_id_parses_round_trips_and_dispatches() {
    let id = ProviderId::parse("pi").expect("ProviderId::parse(\"pi\") must return Some");
    assert_eq!(id.as_str(), "pi", "as_str() must round-trip to \"pi\"");
    assert_eq!(id.display_name(), "Pi", "display_name() must be \"Pi\"");

    // Dispatch must route to the Pi module rather than error out, even for a
    // project path that doesn't exist -- this only proves routing, not I/O
    // success, so we accept either an empty-but-Ok result or a genuine I/O
    // error; what it must NOT do is return the "unsupported provider" shape
    // that would occur if the match arm were missing (in which case this
    // whole function already fails to compile against the unmodified crate,
    // since `ProviderId::Pi` does not exist yet).
    let home = tempfile::tempdir().expect("tempdir");
    let _guard = HomeGuard::set(home.path());
    let root = pi_sessions_root(home.path());
    let file = write_session_file(
        &root,
        "--Users-ac-dev-project-e--",
        "2026-06-08T20-31-45-261Z_019ea8ef-246d-76ee-9e21-449fc7bfadee.jsonl",
        &[
            header_line(
                "019ea8ef-246d-76ee-9e21-449fc7bfadee",
                "/Users/ac/dev/project-e",
                "2026-06-08T20:31:45.261Z",
            ),
            user_message_line(
                "u1",
                "019ea8ef-246d-76ee-9e21-449fc7bfadee",
                "2026-06-08T20:31:50.000Z",
                "Hello there",
                1749412310000,
            ),
        ],
    );

    let sessions = providers::load_sessions(
        id,
        &root.join("--Users-ac-dev-project-e--").to_string_lossy(),
        false,
    );
    assert!(
        sessions.is_ok(),
        "load_sessions dispatched with the parsed Pi id must route to the Pi module and succeed, got: {:?}",
        sessions.err()
    );
    assert_eq!(sessions.unwrap().len(), 1);

    let messages = providers::load_messages(id, &file.to_string_lossy());
    assert!(
        messages.is_ok(),
        "load_messages dispatched with the parsed Pi id must route to the Pi module and succeed, got: {:?}",
        messages.err()
    );
    assert_eq!(messages.unwrap().len(), 1);
}
