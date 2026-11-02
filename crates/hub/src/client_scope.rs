//! Client scope: which sessions are client material (openspec change
//! `hub-client-read-scope`, design D1, D2, R2-A, R2-K).
//!
//! The markers are the path markers of the distiller's routing file, kept outside
//! the repo. The match rules are the distiller's (`scripts/cchv-distill.py`
//! `normalize` and `Routing.marker_in`), and the shared fixture
//! `scripts/fixtures/client-scope-cases.json` pins that both sides agree.

use std::collections::HashMap;

use serde_json::Value;
use sha2::{Digest, Sha256};

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
    /// Registered instruction text, subtracted from message strings
    /// (openspec `client-scope-instruction-subtraction`).
    instructions: InstructionBlocks,
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
        Self {
            literal,
            segments,
            instructions: InstructionBlocks::default(),
        }
    }

    /// Every needle a match can rest on: the literal markers and the segment
    /// names.
    fn needles(&self) -> impl Iterator<Item = &str> {
        self.literal
            .iter()
            .chain(self.segments.iter())
            .map(String::as_str)
    }

    /// Whether `normalized` holds any needle as a plain substring, boundaries
    /// ignored. A conservative guard: every piece left after subtraction
    /// normalizes to a substring of the whole (block edges never split an
    /// escape, see [`Self::block_edges_safe`]), and a hit needs its needle.
    fn may_match(&self, normalized: &str) -> bool {
        self.needles().any(|n| normalized.contains(n))
    }

    /// Whether a block may be subtracted where it occurs (review U5): neither
    /// edge, over the longest needle's length, holds a `%` (an escape split by
    /// the edge), and no needle can straddle an edge (the block neither ends
    /// with a needle's proper prefix nor starts with its proper suffix). An
    /// unsafe occurrence is scanned like any other text.
    pub fn block_edges_safe(&self, block: &str) -> bool {
        let n = self.needles().map(str::len).max().unwrap_or(0);
        let head = &block[..floor_boundary(block, n.min(block.len()))];
        let tail = &block[ceil_boundary(block, block.len().saturating_sub(n))..];
        if head.contains('%') || tail.contains('%') {
            return false;
        }
        let (head, tail) = (normalize(head), normalize(tail));
        !self.needles().any(|m| {
            m.char_indices()
                .skip(1)
                .any(|(k, _)| tail.ends_with(&m[..k]) || head.starts_with(&m[k..]))
        })
    }

    #[must_use]
    pub fn with_instructions(self, instructions: InstructionBlocks) -> Self {
        Self {
            instructions,
            ..self
        }
    }

    pub fn is_empty(&self) -> bool {
        self.literal.is_empty()
    }

    /// Whether `normalized` (already passed through [`normalize`]) holds a marker.
    pub fn matches(&self, normalized: &str) -> bool {
        self.literal.iter().any(|m| normalized.contains(m.as_str()))
            || self.segments.iter().any(|s| segment_in(normalized, s))
    }

    /// Whether a message string holds a marker outside its registered
    /// instruction blocks: each piece left between them is matched on its own.
    pub fn hits_message_text(&self, text: &str) -> bool {
        let normalized = normalize(text);
        if self.instructions.is_empty() {
            return self.matches(&normalized);
        }
        if !self.may_match(&normalized) {
            return false;
        }
        self.unguarded_hits(text)
    }

    /// Subtraction and piece matching with no guard: the reference the guard
    /// is tested against (review U3).
    fn unguarded_hits(&self, text: &str) -> bool {
        self.pieces(text)
            .iter()
            .any(|p| self.matches(&normalize(p)))
    }

    /// The pieces of `text` outside the union of every registered block's
    /// occurrences at a word start: offset 0 or right after ASCII whitespace
    /// (the injected file follows a blank line, its `search_text` form a
    /// space). Every occurrence counts, overlapping ones included, so the
    /// result does not depend on the order of the entries (review U4).
    fn pieces<'s>(&self, text: &'s str) -> Vec<&'s str> {
        let bytes = text.as_bytes();
        let starts = std::iter::once(0).chain(
            bytes
                .iter()
                .enumerate()
                .filter(|(_, c)| c.is_ascii_whitespace())
                .map(|(i, _)| i + 1),
        );
        let mut spans: Vec<(usize, usize)> = starts
            .filter(|&at| at < bytes.len())
            .flat_map(|at| {
                self.instructions
                    .lengths_at(text, at)
                    .into_iter()
                    .map(move |len| (at, at + len))
            })
            .filter(|&(a, b)| self.block_edges_safe(&text[a..b]))
            .collect();
        spans.sort_unstable();
        let mut pieces = Vec::new();
        let mut from = 0;
        for (a, b) in spans {
            if a > from {
                pieces.push(&text[from..a]);
            }
            from = from.max(b);
        }
        pieces.push(&text[from..]);
        pieces
    }
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_boundary(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// How many leading bytes of a block its head hash covers.
pub const HEAD_BYTES: usize = 64;
/// The shortest block the routing file may register: a marker-sized block
/// would blind the classifier to that marker.
pub const MIN_BLOCK_BYTES: usize = 256;

/// A text as the `search_text` column holds it: whitespace runs collapsed to
/// one space, ends trimmed (history-core `search_text`, the same std split).
pub fn search_form(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One registered instruction text, by hashes and length only (design D1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstructionBlock {
    pub sha256: [u8; 32],
    pub bytes: usize,
    pub head_sha256: [u8; 32],
}

impl InstructionBlock {
    /// The entry for `text` exactly as given.
    pub fn of(text: &str) -> Self {
        let b = text.as_bytes();
        Self {
            sha256: Sha256::digest(b).into(),
            bytes: b.len(),
            head_sha256: Sha256::digest(&b[..HEAD_BYTES.min(b.len())]).into(),
        }
    }
}

/// The registered blocks, indexed by the hash of their first [`HEAD_BYTES`].
#[derive(Default)]
pub struct InstructionBlocks {
    by_head: HashMap<[u8; 32], Vec<(usize, [u8; 32])>>,
}

impl InstructionBlocks {
    pub fn new(blocks: &[InstructionBlock]) -> Self {
        let mut by_head: HashMap<[u8; 32], Vec<(usize, [u8; 32])>> = HashMap::new();
        for b in blocks {
            by_head
                .entry(b.head_sha256)
                .or_default()
                .push((b.bytes, b.sha256));
        }
        Self { by_head }
    }

    pub fn is_empty(&self) -> bool {
        self.by_head.is_empty()
    }

    /// The lengths of every registered block that starts at byte `at` of `s`.
    fn lengths_at(&self, s: &str, at: usize) -> Vec<usize> {
        let b = &s.as_bytes()[at..];
        let Some(head) = b.get(..HEAD_BYTES) else {
            return Vec::new();
        };
        let Some(candidates) = self.by_head.get(&<[u8; 32]>::from(Sha256::digest(head))) else {
            return Vec::new();
        };
        candidates
            .iter()
            .filter(|&&(len, sha)| {
                len <= b.len()
                    && s.is_char_boundary(at + len)
                    && <[u8; 32]>::from(Sha256::digest(&b[..len])) == sha
            })
            .map(|&(len, _)| len)
            .collect()
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

/// The served fields of a session row and of its project.
#[derive(Debug, Default, Clone, Copy)]
pub struct SessionFields<'a> {
    pub project_path: &'a str,
    pub summary: Option<&'a str>,
    pub file_path: Option<&'a str>,
    pub entrypoint: Option<&'a str>,
    /// The project's identity besides its path (E1): a hit here is `Path`.
    pub project: ProjectFields<'a>,
}

/// A project row's served identity besides its path (E1).
#[derive(Debug, Default, Clone, Copy)]
pub struct ProjectFields<'a> {
    pub name: Option<&'a str>,
    pub git_remote_url: Option<&'a str>,
    pub git_main_path: Option<&'a str>,
}

impl ProjectFields<'_> {
    /// Whether the project's name, git remote or main checkout path holds a marker.
    pub fn hits(&self, markers: &Markers) -> bool {
        [self.name, self.git_remote_url, self.git_main_path]
            .into_iter()
            .flatten()
            .any(|s| markers.matches(&normalize(s)))
    }
}

/// One stored message: `content` and `raw` in full, and the `search_text` the
/// search route serves (E1: accepted from the client, so it is classified too).
#[derive(Debug, Clone, Copy)]
pub struct MessageFields<'a> {
    pub content: &'a Value,
    pub raw: &'a Value,
    pub search_text: Option<&'a str>,
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

/// The block types besides `tool_use` and `tool_result` that the hub serves
/// (pg1 block census, all providers, 2026-11-02; plus the API's `document`
/// and `redacted_thinking`; `opencode_step` is `history-core`'s `OpenCode` step
/// record). Any other type, or none, is unreadable (E1); the distiller's
/// `OTHER_BLOCK_TYPES` holds the same list (R9).
const READ_BLOCK_TYPES: [&str; 9] = [
    "text",
    "thinking",
    "image",
    "document",
    "redacted_thinking",
    "tool_addition",
    "tool_removal",
    "fallback",
    "opencode_step",
];

/// Every object under `v` with a string `path` and a string `pattern` gives
/// `path` + `/` + `pattern`, the distiller's join for search-tool inputs
/// (`_string_values`): a client folder split across the two keys still hits.
fn path_pattern_joins(
    v: &Value,
    depth: usize,
    budget: &mut usize,
    f: &mut dyn FnMut(&str),
) -> Result<(), ()> {
    if depth > MAX_DEPTH {
        return Err(());
    }
    match v {
        Value::Array(items) => items
            .iter()
            .try_for_each(|x| path_pattern_joins(x, depth + 1, budget, f)),
        Value::Object(map) => {
            if let (Some(Value::String(path)), Some(Value::String(pattern))) =
                (map.get("path"), map.get("pattern"))
            {
                let joined = format!("{}/{}", path.trim_end_matches(['/', '\\']), pattern);
                *budget = budget.checked_sub(joined.len()).ok_or(())?;
                f(&joined);
            }
            map.values()
                .try_for_each(|x| path_pattern_joins(x, depth + 1, budget, f))
        }
        Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
    }
}

/// A `tool_use` or `tool_result` block: its own keys and every value outside
/// `inner` are text; the strings under `inner` go to `f_inner`.
fn tool_block<'v>(
    block: &'v serde_json::Map<String, Value>,
    inner: &str,
    budget: &mut usize,
    f_text: &mut dyn FnMut(&'v str),
    f_inner: &mut dyn FnMut(&'v str),
) -> Result<(), ()> {
    block.iter().try_for_each(|(k, x)| {
        take(k, budget, f_text)?;
        if k == inner {
            leaves(x, 3, budget, f_inner)
        } else {
            leaves(x, 3, budget, f_text)
        }
    })
}

fn message_hits(m: &Markers, msg: MessageFields<'_>) -> Hits {
    let mut h = Hits::default();
    // One budget for the whole message, `content` and `raw` together (E1).
    let mut budget = MAX_MESSAGE_BYTES;
    let hit = |s: &str| m.hits_message_text(s);

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
                Some("tool_use") => {
                    tool_block(
                        block,
                        "input",
                        &mut budget,
                        &mut |s| h.text |= hit(s),
                        &mut |s| h.tool_input |= hit(s),
                    )?;
                    match block.get("input") {
                        Some(input) => path_pattern_joins(input, 3, &mut budget, &mut |s| {
                            h.tool_input |= hit(s);
                        }),
                        None => Ok(()),
                    }
                }
                Some("tool_result") => tool_block(
                    block,
                    "content",
                    &mut budget,
                    &mut |s| h.text |= hit(s),
                    &mut |s| h.tool_result |= hit(s),
                ),
                Some(t) if READ_BLOCK_TYPES.contains(&t) => {
                    leaves(item, 2, &mut budget, &mut |s| h.text |= hit(s))
                }
                // E1: an unknown type, or none, is a shape the extractor cannot vouch for.
                _ => Err(()),
            }
        }),
        Value::Bool(_) | Value::Number(_) => Err(()),
    };
    h.unreadable |= content_ok.is_err();

    // `search_text`: what the search route indexes and quotes (E1).
    if let Some(st) = msg.search_text {
        if take(st, &mut budget, &mut |s| h.text |= hit(s)).is_err() {
            h.unreadable = true;
        }
    }

    // `raw`: the provider's record as received, in full. `cwd` first, by name.
    if let Some(cwd) = msg.raw.get("cwd").and_then(Value::as_str) {
        // A path, never instruction text: matched exactly (design D3, review U7).
        h.cwd |= m.matches(&normalize(cwd));
    }
    if leaves(msg.raw, 1, &mut budget, &mut |s| h.raw |= hit(s)).is_err() {
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
    if hit(session.project_path) || session.project.hits(markers) {
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
        project_name: Option<String>,
        git_remote_url: Option<String>,
        git_main_path: Option<String>,
    }

    #[derive(Deserialize)]
    struct MessageCase {
        content: Value,
        raw: Value,
        #[serde(default)]
        search_text: Option<String>,
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
                    project: ProjectFields {
                        name: c.session.project_name.as_deref(),
                        git_remote_url: c.session.git_remote_url.as_deref(),
                        git_main_path: c.session.git_main_path.as_deref(),
                    },
                };
                let got = classify(
                    &m,
                    session,
                    c.messages.iter().map(|x| MessageFields {
                        content: &x.content,
                        raw: &x.raw,
                        search_text: x.search_text.as_deref(),
                    }),
                );
                (got.as_str() != c.class).then(|| format!("{}: got {}", c.name, got.as_str()))
            })
            .collect();
        assert!(wrong.is_empty(), "cases that disagree: {wrong:?}");
    }

    /// E1: the 16 MiB budget is per message, `content` and `raw` together.
    #[test]
    fn the_byte_budget_spans_content_and_raw() {
        let half = "a".repeat(MAX_MESSAGE_BYTES / 2 + 1);
        let content = serde_json::json!([{"type": "text", "text": half}]);
        let raw = serde_json::json!({"note": half});
        let classify_one = |raw: &Value| {
            classify(
                &Markers::new(&["/clientco".to_string()]),
                SessionFields {
                    project_path: "/w/lab",
                    ..Default::default()
                },
                [MessageFields {
                    content: &content,
                    raw,
                    search_text: None,
                }],
            )
        };
        assert_eq!(
            classify_one(&serde_json::json!({})),
            Class::None,
            "content alone fits"
        );
        assert_eq!(classify_one(&raw), Class::Unreadable);
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
                search_text: None,
            }],
        );
        assert_eq!(got, Class::Unreadable);
    }

    /// An instructions file as Claude Code injects it: a client folder named in
    /// one of its lines, as in ac's global file.
    fn block_text() -> String {
        format!(
            "## RULE #1\n{}\nClient questions go to the desk; their notes live in /ClientCo/notes.\n{}",
            "Lab line about the desk. ".repeat(8),
            "Closing lab line. ".repeat(6)
        )
    }

    fn registered(blocks: &[&str]) -> Markers {
        Markers::new(&["/clientco".to_string()]).with_instructions(InstructionBlocks::new(
            &blocks
                .iter()
                .map(|b| InstructionBlock::of(b))
                .collect::<Vec<_>>(),
        ))
    }

    fn one(m: &Markers, content: &Value, raw: &Value, search_text: Option<&str>) -> Class {
        classify(
            m,
            SessionFields {
                project_path: "/w/lab",
                ..Default::default()
            },
            [MessageFields {
                content,
                raw,
                search_text,
            }],
        )
    }

    /// The request copy: the file after its heading line, at a line start.
    fn request_copy(body: &str) -> Value {
        serde_json::json!([{"type": "text", "text": format!(
            "<system-reminder>\nContents of /Users/ac/.config/claude/CLAUDE.md (global):\n\n{body}\n\nContents of /w/lab/AGENTS.md:\n\nlab rules\n</system-reminder>"
        )}])
    }

    fn instructions_attachment(files: &[&str]) -> Value {
        serde_json::json!({
            "type": "attachment",
            "attachment": {"type": "instructions", "files": files.iter().enumerate()
                .map(|(i, c)| serde_json::json!({"path": format!("/w/f{i}/CLAUDE.md"), "content": c, "type": "User"}))
                .collect::<Vec<_>>()}
        })
    }

    #[test]
    fn a_registered_block_is_subtracted() {
        let b = block_text();
        assert!(b.len() >= MIN_BLOCK_BYTES);
        let content = request_copy(&b);
        let raw = instructions_attachment(&[&b]);
        let none = serde_json::json!({});
        // The control: unregistered, the injected copies mark the session.
        assert_eq!(
            one(&registered(&[]), &content, &none, None),
            Class::TextMarker
        );
        assert_eq!(one(&registered(&[]), &Value::Null, &raw, None), Class::Raw);
        let m = registered(&[&b]);
        assert_eq!(one(&m, &content, &none, None), Class::None);
        assert_eq!(one(&m, &Value::Null, &raw, None), Class::None);
        // The same text in `search_text` and in `raw`'s copy of the content.
        let raw_copy = serde_json::json!({"content": content});
        let st = content[0]["text"].as_str().unwrap();
        assert_eq!(one(&m, &Value::Null, &raw_copy, Some(st)), Class::None);
        // Twice in one string: both copies go.
        let twice = request_copy(&format!("{b}\n{b}"));
        assert_eq!(one(&m, &twice, &none, None), Class::None);
    }

    #[test]
    fn one_changed_byte_is_still_scanned() {
        let b = block_text();
        let m = registered(&[&b]);
        let changed = b.replacen("Closing", "closing", 1);
        assert_eq!(changed.len(), b.len());
        assert_eq!(
            one(&m, &request_copy(&changed), &serde_json::json!({}), None),
            Class::TextMarker
        );
        assert_eq!(
            one(
                &m,
                &Value::Null,
                &instructions_attachment(&[&changed]),
                None
            ),
            Class::Raw
        );
    }

    #[test]
    fn client_text_appended_after_the_block_still_marks() {
        let b = block_text();
        let m = registered(&[&b]);
        let none = serde_json::json!({});
        for tail in [
            "\nnow open /clientco/q3.xlsx",
            "/clientco/q3.xlsx",
            " see /CLIENTCO",
        ] {
            assert_eq!(
                one(&m, &request_copy(&format!("{b}{tail}")), &none, None),
                Class::TextMarker,
                "tail {tail:?}"
            );
        }
        // Before the block, on its own line.
        assert_eq!(
            one(
                &m,
                &request_copy(&format!("cd /clientco\n{b}")),
                &none,
                None
            ),
            Class::TextMarker
        );
    }

    #[test]
    fn an_attachment_holding_a_client_file_still_raises() {
        let b = block_text();
        let m = registered(&[&b]);
        let raw = instructions_attachment(&[
            &b,
            "# Project rules\nThe data sits in /clientco/exports.\n",
        ]);
        assert_eq!(one(&m, &Value::Null, &raw, None), Class::Raw);
    }

    /// A block counts only where a word starts: right after whitespace or at
    /// the start. Glued to a word, it is scanned.
    #[test]
    fn a_block_glued_to_a_word_is_scanned() {
        let b = block_text();
        let m = registered(&[&b]);
        let none = serde_json::json!({});
        let glued = serde_json::json!([{"type": "text", "text": format!("quoted:{b}")}]);
        assert_eq!(one(&m, &glued, &none, None), Class::TextMarker);
        let spaced = serde_json::json!([{"type": "text", "text": format!("quoted: {b}")}]);
        assert_eq!(one(&m, &spaced, &none, None), Class::None);
    }

    /// `search_text` holds the message with whitespace runs collapsed
    /// (history-core `search_text`): its form of the block is registered too.
    #[test]
    fn the_search_text_form_is_subtracted_when_registered() {
        let b = block_text();
        let content = request_copy(&b);
        let flat = search_form(content[0]["text"].as_str().unwrap());
        assert!(!flat.contains('\n'));
        let none = serde_json::json!({});
        assert_eq!(
            one(&registered(&[&b]), &Value::Null, &none, Some(&flat)),
            Class::TextMarker,
            "the injected form alone does not cover search_text"
        );
        let both = registered(&[&b, &search_form(&b)]);
        assert_eq!(one(&both, &content, &none, Some(&flat)), Class::None);
        assert_eq!(
            one(
                &both,
                &Value::Null,
                &none,
                Some(&format!("{flat} /clientco/x"))
            ),
            Class::TextMarker
        );
    }

    #[test]
    fn versions_sharing_a_head_are_each_found() {
        let old = block_text();
        let new = format!("{old}\nOne more lab line.");
        let m = registered(&[&old, &new]);
        let none = serde_json::json!({});
        assert_eq!(one(&m, &request_copy(&old), &none, None), Class::None);
        assert_eq!(one(&m, &request_copy(&new), &none, None), Class::None);
        // A tool input that only quotes the block is subtracted too (D3).
        let tool =
            serde_json::json!([{"type": "tool_use", "name": "Write", "input": {"content": new}}]);
        assert_eq!(one(&m, &tool, &none, None), Class::None);
    }

    /// Review U3: a hit that exists only after subtraction (a segment glued to
    /// the block's end) marks.
    #[test]
    fn a_segment_glued_after_the_block_marks() {
        let b = block_text();
        let m = registered(&[&b]);
        let glued = serde_json::json!([{"type": "text", "text": format!("{b}clientco/private")}]);
        assert_eq!(
            one(&m, &glued, &serde_json::json!({}), None),
            Class::TextMarker
        );
    }

    /// Review U3: the guarded matcher agrees with subtraction and piece
    /// matching run on every string, over the combinations of a block with
    /// marker fragments and separators on each side.
    #[test]
    fn the_guard_agrees_with_unguarded_subtraction() {
        let b = block_text();
        let flat = search_form(&b);
        let m = registered(&[&b, &flat]);
        let sides = [
            "",
            " ",
            "\n",
            "x",
            "/",
            "%2F",
            "clientco",
            "/clientco",
            "clientco/private",
            " clientco/p",
            "cl",
            "ientco",
            "/CLIENTCO/x",
            "%2fclientco",
            "_clientco",
            "clientco-x",
        ];
        let mut checked = 0;
        for body in [b.as_str(), flat.as_str(), "plain lab text"] {
            for pre in sides {
                for post in sides {
                    for mid in ["", " ", "\n"] {
                        let text = format!("{pre}{mid}{body}{post}");
                        assert_eq!(
                            m.hits_message_text(&text),
                            m.unguarded_hits(&text),
                            "{pre:?} {mid:?} {post:?}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 3 * 16 * 16 * 3);
    }

    /// Review U4: the union of every occurrence, whatever the entry order,
    /// with a version that is a prefix of another and blocks that overlap.
    #[test]
    fn subtraction_does_not_depend_on_entry_order() {
        let old = block_text();
        let new = format!("{old}And one more lab line for the newer version.");
        let shifted = format!(
            "{}{}",
            &old[old.find("Lab line").unwrap()..],
            "Tail lab text, long enough to stand alone. ".repeat(4)
        );
        assert!(shifted.len() >= MIN_BLOCK_BYTES);
        let texts = [
            format!("{new} /clientco"),
            format!("{new}\n{old}"),
            old.clone(),
            format!("## RULE #1\n{shifted}"),
        ];
        let orders: [[&str; 3]; 3] = [
            [&old, &new, &shifted],
            [&new, &shifted, &old],
            [&shifted, &old, &new],
        ];
        for t in &texts {
            let got: Vec<bool> = orders
                .iter()
                .map(|o| registered(o).hits_message_text(t))
                .collect();
            assert!(got.windows(2).all(|w| w[0] == w[1]), "{got:?}");
        }
        // The newer version is removed whole, not only its older prefix.
        let m = registered(&[&old, &new]);
        assert!(!m.hits_message_text(&new));
        assert!(m.hits_message_text(&format!("{new} /clientco")));
    }

    /// Review U5: a block whose edge could carry part of a marker, or an
    /// escape, is scanned where it occurs.
    #[test]
    fn a_block_with_an_unsafe_edge_is_scanned() {
        let m = Markers::new(&["/clientco".to_string()]);
        let body = "Lab line about the desk. ".repeat(12);
        assert!(m.block_edges_safe(&format!("## Head\n{body}end.")));
        for unsafe_block in [
            format!("## Head\n{body}/cli"),
            format!("## Head\n{body}see /"),
            format!("## Head\n{body}%2"),
            format!("ntco/x\n{body}end."),
            format!("%41 head\n{body}end."),
        ] {
            assert!(!m.block_edges_safe(&unsafe_block), "{unsafe_block:?}");
            let r = registered(&[&unsafe_block]);
            let text = format!("{unsafe_block}entco/notes");
            assert!(r.hits_message_text(&text) || !m.matches(&normalize(&text)));
        }
        // Straddling: the block ends in "/cli", the text goes on "entco".
        let straddle = format!("## Head\n{body}/cli");
        assert_eq!(
            one(
                &registered(&[&straddle]),
                &serde_json::json!([{"type": "text", "text": format!("{straddle}entco")}]),
                &serde_json::json!({}),
                None
            ),
            Class::TextMarker
        );
    }

    /// Review U7: `cwd` is a path and is matched exactly, never subtracted.
    #[test]
    fn cwd_is_matched_exactly() {
        let b = block_text();
        let m = registered(&[&b]);
        let raw = serde_json::json!({"cwd": b});
        assert_eq!(one(&m, &Value::Null, &raw, None), Class::Cwd);
    }
}
