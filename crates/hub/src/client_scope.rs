//! Client scope: which sessions are client material (openspec change
//! `hub-client-read-scope`, design D1, D2, R2-A, R2-K).
//!
//! The markers are the path markers of the distiller's routing file, kept outside
//! the repo. The match rules are the distiller's (`scripts/cchv-distill.py`
//! `normalize` and `Routing.marker_in`), and the shared fixture
//! `scripts/fixtures/client-scope-cases.json` pins that both sides agree.

use serde_json::Value;

/// What the matcher compares (distiller `normalize`): percent-decoded once,
/// backslashes as `/`, lower case.
///
/// The decode is Python's `urllib.parse.unquote`: each `%XX` with two hex digits
/// becomes that byte, anything else stays as written, and the bytes are read as
/// UTF-8 with U+FFFD for an invalid sequence. Lower case is the Unicode default
/// mapping, as Python's `str.lower` (so `İ` becomes `i̇`, two code points).
pub fn normalize(text: &str) -> String {
    percent_decode(text).replace('\\', "/").to_lowercase()
}

fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// A compiled set of path markers (distiller `Routing.marker_in`).
///
/// A text holds a marker when the normalized marker occurs in it literally, or,
/// for a marker that starts with `/`, when the marker without its outer slashes
/// occurs as a segment: the character before it is not in `[a-z0-9_]` (or there
/// is none) and the character after it is not in `[a-z0-9_-]` (or there is
/// none). The literal test comes first, so `/name` inside `/name-lab` matches
/// even though `name-lab` is not a segment hit.
pub struct Markers {
    literal: Vec<String>,
    segments: Vec<String>,
}

impl Markers {
    pub fn new(markers: &[String]) -> Self {
        let literal: Vec<String> = markers.iter().map(|m| normalize(m)).collect();
        let segments = literal
            .iter()
            .filter(|m| m.starts_with('/'))
            .map(|m| m.trim_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        Self { literal, segments }
    }

    pub fn is_empty(&self) -> bool {
        self.literal.is_empty()
    }

    /// Whether `normalized` (already passed through [`normalize`]) holds a marker.
    pub fn matches(&self, normalized: &str) -> bool {
        self.literal.iter().any(|m| normalized.contains(m.as_str()))
            || self.segments.iter().any(|s| segment_in(normalized, s))
    }
}

fn segment_in(text: &str, needle: &str) -> bool {
    let before_ok = |c: Option<char>| {
        !c.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    let after_ok = |c: Option<char>| {
        !c.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    };
    // Every occurrence, overlapping ones included: the regex search in the
    // distiller tries each start position.
    let mut from = 0;
    while let Some(pos) = text[from..].find(needle) {
        let start = from + pos;
        let end = start + needle.len();
        if before_ok(text[..start].chars().next_back()) && after_ok(text[end..].chars().next()) {
            return true;
        }
        from = start + text[start..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

/// A session's class: the first match in the order of the variants, else
/// `Unreadable` when any part could not be read, else `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Path,
    Metadata,
    Cwd,
    ToolInput,
    ToolResult,
    TextMarker,
    Raw,
    Unreadable,
    None,
}

impl Class {
    /// Position in the order classes are picked (lower wins).
    pub fn rank(self) -> u8 {
        self as u8
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Class::Path => "path",
            Class::Metadata => "metadata",
            Class::Cwd => "cwd",
            Class::ToolInput => "tool_input",
            Class::ToolResult => "tool_result",
            Class::TextMarker => "text_marker",
            Class::Raw => "raw",
            Class::Unreadable => "unreadable",
            Class::None => "none",
        }
    }
}

/// The served fields of a session row.
#[derive(Debug, Default, Clone, Copy)]
pub struct SessionFields<'a> {
    pub project_path: &'a str,
    pub summary: Option<&'a str>,
    pub file_path: Option<&'a str>,
    pub entrypoint: Option<&'a str>,
}

/// One stored message, in full (never the clamped `search_text`).
#[derive(Debug, Clone, Copy)]
pub struct MessageFields<'a> {
    pub content: &'a Value,
    pub raw: &'a Value,
}

/// Past this depth of JSON nesting a value is not read: unreadable.
pub const MAX_DEPTH: usize = 64;
/// Past this many bytes of extracted strings in one message: unreadable.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// What one message's parts hit, before the order picks one class.
#[derive(Default)]
#[allow(clippy::struct_excessive_bools)] // one flag per class, read in the order of `Class`
struct Hits {
    cwd: bool,
    tool_input: bool,
    tool_result: bool,
    text: bool,
    raw: bool,
    unreadable: bool,
}

/// Every string leaf and object key of `v`, depth-first. `Err` past the depth
/// or byte limit.
fn leaves<'v>(
    v: &'v Value,
    depth: usize,
    budget: &mut usize,
    f: &mut dyn FnMut(&'v str),
) -> Result<(), ()> {
    if depth > MAX_DEPTH {
        return Err(());
    }
    match v {
        Value::String(s) => take(s, budget, f)?,
        Value::Array(items) => {
            for x in items {
                leaves(x, depth + 1, budget, f)?;
            }
        }
        Value::Object(map) => {
            for (k, x) in map {
                take(k, budget, f)?;
                leaves(x, depth + 1, budget, f)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

fn take<'v>(s: &'v str, budget: &mut usize, f: &mut dyn FnMut(&'v str)) -> Result<(), ()> {
    *budget = budget.checked_sub(s.len()).ok_or(())?;
    f(s);
    Ok(())
}

fn message_hits(m: &Markers, msg: MessageFields<'_>) -> Hits {
    let mut h = Hits::default();
    let mut budget = MAX_MESSAGE_BYTES;
    let hit = |s: &str| m.matches(&normalize(s));

    // `content`: the normalized message, served by the browse and search routes.
    let content_ok = match msg.content {
        Value::Null => Ok(()),
        Value::String(_) | Value::Object(_) => {
            leaves(msg.content, 1, &mut budget, &mut |s| h.text |= hit(s))
        }
        Value::Array(items) => items.iter().try_for_each(|item| {
            let Value::Object(block) = item else {
                return Err(());
            };
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => block.iter().try_for_each(|(k, x)| {
                    let into_input = k == "input";
                    let mut f = |s: &str| {
                        if into_input {
                            h.tool_input |= hit(s);
                        } else {
                            h.text |= hit(s);
                        }
                    };
                    leaves(x, 3, &mut budget, &mut f)
                }),
                Some("tool_result") => block.iter().try_for_each(|(k, x)| {
                    let into_result = k == "content";
                    let mut f = |s: &str| {
                        if into_result {
                            h.tool_result |= hit(s);
                        } else {
                            h.text |= hit(s);
                        }
                    };
                    leaves(x, 3, &mut budget, &mut f)
                }),
                _ => leaves(item, 2, &mut budget, &mut |s| h.text |= hit(s)),
            }
        }),
        Value::Bool(_) | Value::Number(_) => Err(()),
    };
    h.unreadable |= content_ok.is_err();

    // `raw`: the provider's record as received, in full. `cwd` first, by name.
    if let Some(cwd) = msg.raw.get("cwd").and_then(Value::as_str) {
        h.cwd |= hit(cwd);
    }
    let mut raw_budget = MAX_MESSAGE_BYTES;
    if leaves(msg.raw, 1, &mut raw_budget, &mut |s| h.raw |= hit(s)).is_err() {
        h.unreadable = true;
    }
    h
}

/// Classify a session from its row and its messages: the first class hit in
/// the order of [`Class`], else `Unreadable` when any part could not be read,
/// else `None`.
pub fn classify<'a>(
    markers: &Markers,
    session: SessionFields<'_>,
    messages: impl IntoIterator<Item = MessageFields<'a>>,
) -> Class {
    let hit = |s: &str| markers.matches(&normalize(s));
    if hit(session.project_path) {
        return Class::Path;
    }
    if [session.summary, session.file_path, session.entrypoint]
        .into_iter()
        .flatten()
        .any(hit)
    {
        return Class::Metadata;
    }
    let mut all = Hits::default();
    for msg in messages {
        let h = message_hits(markers, msg);
        all.cwd |= h.cwd;
        all.tool_input |= h.tool_input;
        all.tool_result |= h.tool_result;
        all.text |= h.text;
        all.raw |= h.raw;
        all.unreadable |= h.unreadable;
    }
    [
        (all.cwd, Class::Cwd),
        (all.tool_input, Class::ToolInput),
        (all.tool_result, Class::ToolResult),
        (all.text, Class::TextMarker),
        (all.raw, Class::Raw),
        (all.unreadable, Class::Unreadable),
    ]
    .into_iter()
    .find_map(|(hit, class)| hit.then_some(class))
    .unwrap_or(Class::None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Fixture {
        markers: Vec<String>,
        match_cases: Vec<MatchCase>,
        extract_cases: Vec<ExtractCase>,
    }

    #[derive(Deserialize)]
    struct MatchCase {
        name: String,
        text: String,
        #[serde(rename = "match")]
        expect: bool,
    }

    #[derive(Deserialize)]
    struct ExtractCase {
        name: String,
        project_path: String,
        session: SessionCase,
        messages: Vec<MessageCase>,
        class: String,
    }

    #[derive(Deserialize, Default)]
    struct SessionCase {
        summary: Option<String>,
        file_path: Option<String>,
        entrypoint: Option<String>,
    }

    #[derive(Deserialize)]
    struct MessageCase {
        content: Value,
        raw: Value,
    }

    fn fixture() -> Fixture {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/fixtures/client-scope-cases.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// Shared with the distiller: the same inputs give the same answer.
    #[test]
    fn match_cases_agree_with_the_distiller() {
        let f = fixture();
        let m = Markers::new(&f.markers);
        let wrong: Vec<&str> = f
            .match_cases
            .iter()
            .filter(|c| m.matches(&normalize(&c.text)) != c.expect)
            .map(|c| c.name.as_str())
            .collect();
        assert!(wrong.is_empty(), "cases that disagree: {wrong:?}");
    }

    #[test]
    fn extract_cases_classify_the_whole_stored_form() {
        let f = fixture();
        let m = Markers::new(&f.markers);
        let wrong: Vec<String> = f
            .extract_cases
            .iter()
            .filter_map(|c| {
                let session = SessionFields {
                    project_path: &c.project_path,
                    summary: c.session.summary.as_deref(),
                    file_path: c.session.file_path.as_deref(),
                    entrypoint: c.session.entrypoint.as_deref(),
                };
                let got = classify(
                    &m,
                    session,
                    c.messages.iter().map(|x| MessageFields {
                        content: &x.content,
                        raw: &x.raw,
                    }),
                );
                (got.as_str() != c.class).then(|| format!("{}: got {}", c.name, got.as_str()))
            })
            .collect();
        assert!(wrong.is_empty(), "cases that disagree: {wrong:?}");
    }

    /// Past the nesting limit the extractor gives up, and giving up is client.
    #[test]
    fn nesting_past_the_limit_is_unreadable() {
        let mut v = Value::String("lab".into());
        for _ in 0..80 {
            v = Value::Array(vec![v]);
        }
        let content = serde_json::json!([{"type": "tool_use", "name": "x", "input": v}]);
        let raw = serde_json::json!({});
        let got = classify(
            &Markers::new(&["/clientco".to_string()]),
            SessionFields {
                project_path: "/w/lab",
                ..Default::default()
            },
            [MessageFields {
                content: &content,
                raw: &raw,
            }],
        );
        assert_eq!(got, Class::Unreadable);
    }
}
