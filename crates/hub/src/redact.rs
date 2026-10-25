//! Credential-shape detector for Gitea #34, independent of ingest and storage.
//!
//! Hard rule: nothing here may render, log or hash a value. Findings retain only
//! rule ids, secret-named keys and coarse shape metadata; spans stay internal.

use regex::{Regex, RegexSet};
use serde_json::Value;
use std::sync::LazyLock;

use crate::pair;

pub const DETECTOR_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rule {
    Pem,
    Prefix,
    Bearer,
    Assign,
    /// An `assign` hit with a strong value shape (ac/security G3, 2026-10-22):
    /// derived, not a pattern of its own.
    AssignStrong,
    /// A value field paired with a secret-named name field or a sensitivity
    /// flag in the same object (ac/security F7, 2026-10-22): `crate::pair`.
    Pair,
}

impl Rule {
    pub const ALL: [Rule; 6] = [
        Rule::Pem,
        Rule::Prefix,
        Rule::Bearer,
        Rule::Assign,
        Rule::AssignStrong,
        Rule::Pair,
    ];

    pub fn from_id(id: &str) -> Option<Rule> {
        Self::ALL.into_iter().find(|r| r.id() == id)
    }

    /// Whether `rules` selects this rule. Selecting `assign` also selects
    /// `assign-strong`, which is a subset of it.
    pub fn selected(self, rules: &[Rule]) -> bool {
        rules.contains(&self) || (self == Rule::AssignStrong && rules.contains(&Rule::Assign))
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Pem => "pem",
            Self::Prefix => "prefix",
            Self::Bearer => "bearer",
            Self::Assign => "assign",
            Self::AssignStrong => "assign-strong",
            Self::Pair => "pair",
        }
    }
}

/// Contains no value, borrowed value fragment, or digest.
#[derive(Debug, PartialEq, Eq)]
pub struct Finding {
    pub rule: Rule,
    pub key: Option<String>,
    pub value_len_bucket: &'static str,
    /// Bits 0..3 indicate lowercase, uppercase, ASCII digit and symbol,
    /// respectively. Whitespace and non-letter/non-digit characters are symbols.
    pub value_classes: u8,
    /// `assign` family only: the value holds an unexpanded reference
    /// ([`is_reference`]), so it is never `assign-strong` (ac/security policy
    /// 2026-10-22).
    pub reference: bool,
    /// `assign` family only: the value holds a placeholder ([`is_placeholder`]).
    /// Reported apart, never exempted.
    pub placeholder: bool,
    /// `pair` only: `name`, `flag` or `name+flag` ([`crate::pair::Basis`]).
    pub pair_basis: Option<&'static str>,
    /// Where in a JSON message the value sat ([`block_kind`]): `text`,
    /// `tool_result`, `tool_input`, `tool_use` or `other`. `None` for plain
    /// text, which has no blocks.
    pub origin: Option<&'static str>,
}

const RULES: [Rule; 4] = [Rule::Pem, Rule::Prefix, Rule::Bearer, Rule::Assign];
const PATTERNS: [&str; 4] = [
    r"-----BEGIN (?P<label>(?:[A-Z0-9]+ )*PRIVATE KEY)-----[\s\S]*?-----END (?P<end_label>(?:[A-Z0-9]+ )*PRIVATE KEY)-----",
    r"\b(?:gh[pousr]_[A-Za-z0-9_\-]{36,}|github_pat_[A-Za-z0-9_\-]{22,}|sk-ant-[A-Za-z0-9_\-]{20,}|sk-lm-[A-Za-z0-9]{8}:[A-Za-z0-9]{16,}|sk-(?:proj-)?[A-Za-z0-9_\-]{20,}|xox[abposr]-[A-Za-z0-9_\-]{10,}|AKIA[0-9A-Z]{16}\b|hv[sbr]\.[A-Za-z0-9_\-]{20,}|gl(?:pat|rt)-[A-Za-z0-9_\-]{20,}(?:\.[A-Za-z0-9_\-]+)*)",
    r"\b(?i:Bearer)[ \t]+(?P<value>[A-Za-z0-9._~+/=\-]{20,})",
    r#"\b(?P<key>[A-Za-z0-9_.\-]*(?i:PASSWORD|PASSWD|SECRET|TOKEN|API_KEY|APIKEY|PRIVATE_KEY|CREDENTIAL)[A-Za-z0-9_.\-]*)["']?[ \t]*[=:][ \t]*(?:"(?P<double>(?:\\.|[^"\\])*)"|'(?P<single>(?:\\.|[^'\\])*)'|(?P<bare>\$\{[^}\r\n]*\}|[^\s,;"'{}]+))"#,
];

struct Detector {
    prefilter: RegexSet,
    patterns: [Regex; 4],
}

static DETECTOR: LazyLock<Detector> = LazyLock::new(|| Detector {
    prefilter: RegexSet::new(PATTERNS).expect("static credential prefilter must compile"),
    patterns: PATTERNS
        .map(|pattern| Regex::new(pattern).expect("static credential pattern must compile")),
});

const SECRET_NAMES: [&str; 8] = [
    "PASSWORD",
    "PASSWD",
    "SECRET",
    "TOKEN",
    "API_KEY",
    "APIKEY",
    "PRIVATE_KEY",
    "CREDENTIAL",
];
const DENIED_KEYS: [&str; 9] = [
    "tokenizer",
    "token_count",
    "tokens",
    "input_tokens",
    "output_tokens",
    "max_tokens",
    "cache_creation_input_tokens",
    "cache_read_input_tokens",
    "secret_name",
];

fn secret_key(key: &str) -> bool {
    if DENIED_KEYS
        .iter()
        .any(|denied| key.eq_ignore_ascii_case(denied))
    {
        return false;
    }
    let upper = key.to_ascii_uppercase();
    SECRET_NAMES.iter().any(|name| upper.contains(name))
}

pub(crate) fn placeholder(value: &str) -> bool {
    let value = value.trim();
    if value.starts_with("[REDACTED:")
        || value.starts_with("${{") // CI template: `${{ secrets.GITHUB_TOKEN }}`
        || value.starts_with("***")
        || value
            .get(..3)
            .is_some_and(|s| s.eq_ignore_ascii_case("xxx"))
        || (value.starts_with('<') && value.ends_with('>'))
    {
        return true;
    }
    let variable = value
        .strip_prefix("${")
        .and_then(|s| s.strip_suffix('}'))
        .or_else(|| value.strip_prefix('$'));
    variable.is_some_and(|name| {
        let mut chars = name.chars();
        chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

fn finding(rule: Rule, key: Option<&str>, value: &str) -> Finding {
    let mut len = 0;
    let mut classes = 0;
    for c in value.chars() {
        len += 1;
        classes |= if c.is_lowercase() {
            1
        } else if c.is_uppercase() {
            2
        } else if c.is_ascii_digit() {
            4
        } else {
            8
        };
    }
    let assign = rule == Rule::Assign;
    Finding {
        rule,
        reference: assign && is_reference(value),
        placeholder: assign && is_placeholder(value),
        pair_basis: None,
        origin: None,
        key: key.map(str::to_owned),
        value_len_bucket: match len {
            0..=15 => "8-15",
            16..=31 => "16-31",
            32..=63 => "32-63",
            _ => "64+",
        },
        value_classes: classes,
    }
}

/// An unquoted value that is a call, an index or a dotted identifier path is
/// source code assigning an expression (`let token = generate_token();`,
/// `const apiKey = process.env.API_KEY;`), not a literal secret. Transcripts are
/// full of Edit/Write tool calls, and those keys are exactly the names real
/// leaks use, so key-name counts could not tell the two apart later.
fn code_expression(value: &str) -> bool {
    let identifier = |part: &str| {
        let mut chars = part.chars();
        chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    value.contains(['(', ')', '[']) || (value.contains('.') && value.split('.').all(identifier))
}

/// `bare`: the value was unquoted text, so it may be code rather than a literal.
fn assignment_finding(key: &str, value: &str, bare: bool) -> Option<Finding> {
    (secret_key(key)
        && value.chars().count() >= 8
        && !value.chars().all(|c| c.is_ascii_digit())
        && !placeholder(value)
        && !(bare && code_expression(value)))
    .then(|| {
        let hit = finding(Rule::Assign, Some(key), value);
        if strong(&hit) && !hit.reference {
            Finding {
                rule: Rule::AssignStrong,
                ..hit
            }
        } else {
            hit
        }
    })
}

/// 32+ characters from all four classes, or 64+ from at least three: the shape
/// a random secret takes and code, paths and digests rarely do.
fn strong(hit: &Finding) -> bool {
    let classes = hit.value_classes.count_ones();
    match hit.value_len_bucket {
        "32-63" => classes == 4,
        "64+" => classes >= 3,
        _ => false,
    }
}

// Byte offsets never escape the detector. No secret is copied into a span.
struct Span {
    start: usize,
    end: usize,
    finding: Finding,
    /// False for a repeat of a `pair` value already counted in this text: it is
    /// still replaced, but reported once (security policy § 5).
    counted: bool,
}

/// A file name that merely starts with a GitLab prefix, not a token: lowercase
/// words and numbers joined by hyphens (at least two segments, the first a word),
/// with an optional lowercase extension. A real tail is random mixed case.
fn gitlab_file_name(value: &str) -> bool {
    let Some(tail) = value
        .strip_prefix("glpat-")
        .or_else(|| value.strip_prefix("glrt-"))
    else {
        return false;
    };
    let lower_alnum = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    };
    let (stem, extensions) = tail.split_once('.').unwrap_or((tail, ""));
    let segments: Vec<&str> = stem.split('-').collect();
    segments.len() >= 2
        && segments[0].len() >= 3
        && segments[0].bytes().all(|b| b.is_ascii_lowercase())
        && segments.iter().all(|s| lower_alnum(s))
        && (extensions.is_empty() || extensions.split('.').all(lower_alnum))
}

/// AWS's documentation example access keys (`AKIA…EXAMPLE`), printed on every
/// AWS docs page that shows a credential. AWS issues random key ids, so a real
/// one ending in the word `EXAMPLE` is a 1-in-32^7 accident.
fn aws_documentation_example(value: &str) -> bool {
    value.starts_with("AKIA") && value.ends_with("EXAMPLE")
}

/// The prefix rule's leading token marker, stripped before judging the payload.
static PREFIX_HEAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:gh[pousr]_|github_pat_|sk-ant-|sk-lm-|sk-proj-|sk-|xox[abposr]-|AKIA|hv[sbr]\.|gl(?:pat|rt)-)")
        .expect("static prefix head must compile")
});

/// A placeholder (`XXXX…`) or a same-length mask of a token (letters to `A`/`a`,
/// digits to `9`): 16+ payload characters drawn from three or fewer distinct
/// ones. A random payload that long essentially never does (below 1e-13).
fn mask_or_placeholder(value: &str) -> bool {
    let payload = PREFIX_HEAD
        .find(value)
        .map_or(value, |head| &value[head.end()..]);
    let chars: Vec<u8> = payload.bytes().filter(u8::is_ascii_alphanumeric).collect();
    let mut distinct = chars.clone();
    distinct.sort_unstable();
    distinct.dedup();
    chars.len() >= 16 && distinct.len() <= 3
}

fn spans(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    for index in DETECTOR.prefilter.matches(text) {
        let rule = RULES[index];
        for captures in DETECTOR.patterns[index].captures_iter(text) {
            let (value, hit) = match rule {
                Rule::Pem => {
                    if captures["label"] != captures["end_label"] {
                        continue;
                    }
                    let value = captures.get(0).expect("whole PEM match");
                    (value, finding(rule, None, value.as_str()))
                }
                Rule::Prefix => {
                    let value = captures.get(0).expect("whole prefix match");
                    // A short specialised token must not fall back to the generic
                    // sk- alternative (whose optional proj- can also backtrack).
                    if ["sk-ant-", "sk-proj-"].iter().any(|prefix| {
                        value
                            .as_str()
                            .strip_prefix(prefix)
                            .is_some_and(|v| v.len() < 20)
                    }) || gitlab_file_name(value.as_str())
                        || mask_or_placeholder(value.as_str())
                        || aws_documentation_example(value.as_str())
                    {
                        continue;
                    }
                    (value, finding(rule, None, value.as_str()))
                }
                Rule::Bearer => {
                    let value = captures.name("value").expect("bearer value capture");
                    (value, finding(rule, None, value.as_str()))
                }
                // Derived from an `assign` hit below; never a pattern of its own.
                Rule::AssignStrong => unreachable!("assign-strong has no pattern"),
                Rule::Pair => unreachable!("pair spans come from crate::pair"),
                Rule::Assign => {
                    let value = captures
                        .name("double")
                        .or_else(|| captures.name("single"))
                        .or_else(|| captures.name("bare"))
                        .expect("assignment value capture");
                    let bare = captures.name("bare").is_some();
                    let Some(hit) = assignment_finding(&captures["key"], value.as_str(), bare)
                    else {
                        continue;
                    };
                    (value, hit)
                }
            };
            spans.push(Span {
                start: value.start(),
                end: value.end(),
                finding: hit,
                counted: true,
            });
        }
    }
    pair_spans(text, &mut spans);
    // Enclosing spans first; ties retain pem/prefix/bearer/assign precedence.
    spans.sort_by_key(|span| (span.start, std::cmp::Reverse(span.end)));
    spans
}

/// `pair` spans: each detected value, plus every other occurrence of it in the
/// text (replaced, not counted again).
fn pair_spans(text: &str, spans: &mut Vec<Span>) {
    let hits = pair::text_pairs(text);
    let mut counted: Vec<&str> = Vec::new();
    for hit in &hits {
        let value = &text[hit.start..hit.end];
        if counted.contains(&value) {
            continue;
        }
        counted.push(value);
        for (at, _) in text.match_indices(value) {
            spans.push(Span {
                start: at,
                end: at + value.len(),
                finding: pair_finding(&hit.name, value, hit.basis),
                counted: at == hit.start,
            });
        }
    }
}

fn pair_finding(name: &str, value: &str, basis: pair::Basis) -> Finding {
    Finding {
        pair_basis: Some(basis.id()),
        ..finding(Rule::Pair, Some(name), value)
    }
}

/// Scan every rule; return metadata only, including overlapping detections.
/// Detection paired with each hit's matched value, for the keyed fingerprints of
/// `findings dry-run --fingerprint-key-file` only: the caller hashes the value
/// at once and keeps nothing else. `Finding` itself stays free of values.
pub(crate) fn scan_values(text: &str) -> Vec<(Finding, &str)> {
    spans(text)
        .into_iter()
        .filter(|span| span.counted)
        .map(|span| {
            let value = &text[span.start..span.end];
            (span.finding, value)
        })
        .collect()
}

/// Where in a message a value sat, for the dry run's inventory: the kind of the
/// nearest typed content block, and the `tool_use_id` that links a tool result
/// back to the call that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    /// `tool_result`, `tool_input`, `text` or `other`.
    pub kind: &'static str,
    pub tool_use_id: Option<String>,
}

impl Origin {
    const OTHER: Origin = Origin {
        kind: "other",
        tool_use_id: None,
    };
}

/// [`scan_values`] over a JSON value, walked as [`redact_json`] walks it, with
/// the [`Origin`] of each hit.
pub(crate) fn json_values(v: &Value) -> Vec<(Finding, String, Origin)> {
    fn walk(v: &Value, key: Option<&str>, at: &Origin, out: &mut Vec<(Finding, String, Origin)>) {
        match v {
            Value::String(text) => {
                if let Some(hit) = key.and_then(|key| assignment_finding(key, text, false)) {
                    out.push((hit, text.clone(), at.clone()));
                }
                out.extend(
                    scan_values(text)
                        .into_iter()
                        .map(|(f, value)| (f, value.to_owned(), at.clone())),
                );
            }
            Value::Array(values) => {
                for value in values {
                    walk(value, None, at, out);
                }
            }
            Value::Object(values) => {
                let here = block_origin(values).unwrap_or_else(|| at.clone());
                if let Some(p) = pair::object_pair(values) {
                    for key in &p.value_keys {
                        if let Some(text) = values.get(key).and_then(Value::as_str) {
                            out.push((
                                pair_finding(&p.name, text, p.basis),
                                text.to_owned(),
                                here.clone(),
                            ));
                        }
                    }
                }
                for (key, value) in values {
                    let child = Origin {
                        kind: child_kind(here.kind, key),
                        tool_use_id: if key == "toolUseResult" {
                            only_tool_use_id(values)
                        } else {
                            here.tool_use_id.clone()
                        },
                    };
                    walk(value, Some(key), &child, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(v, None, &Origin::OTHER, &mut out);
    out
}

/// The origin kind a typed content block sets for everything inside it.
fn block_kind(block: &serde_json::Map<String, Value>) -> Option<&'static str> {
    match block.get("type").and_then(Value::as_str)? {
        "tool_result" => Some("tool_result"),
        "tool_use" => Some("tool_use"),
        "text" | "thinking" => Some("text"),
        _ => None,
    }
}

/// The origin kind of the value under `key` in an object of kind `here`.
fn child_kind(here: &'static str, key: &str) -> &'static str {
    match (here, key) {
        // A Claude transcript line keeps the structured tool result beside the
        // message, not inside its tool_result block.
        (_, "toolUseResult") => "tool_result",
        ("tool_use", "input") => "tool_input",
        ("tool_use", _) => "other",
        _ => here,
    }
}

/// [`block_kind`] with the `tool_use_id` that links a result to its call.
fn block_origin(block: &serde_json::Map<String, Value>) -> Option<Origin> {
    let kind = block_kind(block)?;
    let field = match kind {
        "tool_result" => "tool_use_id",
        "tool_use" => "id",
        _ => {
            return Some(Origin {
                kind,
                tool_use_id: None,
            })
        }
    };
    Some(Origin {
        kind,
        tool_use_id: block.get(field).and_then(Value::as_str).map(str::to_owned),
    })
}

/// The single `tool_use_id` among a message's `tool_result` blocks, if there is
/// exactly one. Stored `raw` keeps the blocks at `content`; a transcript line
/// keeps them at `message.content`.
fn only_tool_use_id(line: &serde_json::Map<String, Value>) -> Option<String> {
    let blocks = line
        .get("content")
        .or_else(|| line.get("message")?.get("content"))?
        .as_array()?;
    let mut ids = blocks.iter().filter_map(|b| {
        (b.get("type")? == "tool_result")
            .then(|| b.get("tool_use_id")?.as_str())
            .flatten()
    });
    let first = ids.next()?;
    ids.next().is_none().then(|| first.to_owned())
}

/// An unexpanded reference to a value held elsewhere: `${VAR}`, `$(cmd)`,
/// `%VAR%`, `{{ var }}` or `#{var}`. ac/security policy 2026-10-22: such a value
/// is not itself the secret.
pub fn is_reference(value: &str) -> bool {
    static PERCENT_VAR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"%[A-Za-z_][A-Za-z0-9_]*%").expect("percent var pattern"));
    ["${", "$(", "{{", "#{"].iter().any(|m| value.contains(m)) || PERCENT_VAR.is_match(value)
}

/// A stand-in for a value: `xxx`, `changeme`, `example`, `dummy`, `redacted`,
/// `<...>` or `***`, case-insensitive. Reported, never exempted (ac/security
/// policy 2026-10-22): a real secret can sit next to the word.
pub fn is_placeholder(value: &str) -> bool {
    static ANGLE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<[A-Za-z][^<>\n]*>").expect("angle placeholder pattern"));
    let lower = value.to_ascii_lowercase();
    ["xxx", "changeme", "example", "dummy", "redacted", "***"]
        .iter()
        .any(|m| lower.contains(m))
        || ANGLE.is_match(value)
}

pub fn scan(text: &str) -> Vec<Finding> {
    spans(text)
        .into_iter()
        .filter(|span| span.counted)
        .map(|span| span.finding)
        .collect()
}

/// Replace values for the selected rules and return findings for ALL rules.
/// Overlapping replacements share the enclosing/leftmost span's marker.
pub fn redact(text: &str, rules: &[Rule]) -> (String, Vec<Finding>) {
    let spans = spans(text);
    let mut replacements: Vec<(usize, usize, Rule)> = Vec::new();
    for span in &spans {
        if !span.finding.rule.selected(rules) {
            continue;
        }
        if let Some(last) = replacements.last_mut() {
            if span.start < last.1 {
                last.1 = last.1.max(span.end);
                continue;
            }
        }
        replacements.push((span.start, span.end, span.finding.rule));
    }
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end, rule) in replacements {
        output.push_str(&text[cursor..start]);
        output.push_str("[REDACTED:");
        output.push_str(rule.id());
        output.push(']');
        cursor = end;
    }
    output.push_str(&text[cursor..]);
    let findings = spans
        .into_iter()
        .filter(|span| span.counted)
        .map(|span| span.finding)
        .collect();
    (output, findings)
}

/// Walk string leaves and secret-named object keys paired with string values.
/// Keys and non-string leaves are preserved. Findings include flag-only rules.
pub fn redact_json(v: &mut Value, rules: &[Rule]) -> Vec<Finding> {
    fn walk(
        v: &mut Value,
        key: Option<&str>,
        at: &'static str,
        rules: &[Rule],
        hits: &mut Vec<Finding>,
    ) {
        let start = hits.len();
        match v {
            Value::String(text) => {
                let assignment = key.and_then(|key| assignment_finding(key, text, false));
                let replace_whole = assignment
                    .as_ref()
                    .filter(|a| a.rule.selected(rules))
                    .map(|a| a.rule);
                // Flag-only (the ingest default) must not rebuild every string
                // leaf of every message: scan without allocating a copy.
                let leaf_hits = if replace_whole.is_some() || rules.is_empty() {
                    scan(text)
                } else {
                    let (redacted, leaf_hits) = redact(text, rules);
                    *text = redacted;
                    leaf_hits
                };
                if let Some(rule) = replace_whole {
                    *text = format!("[REDACTED:{}]", rule.id());
                }
                hits.extend(assignment);
                hits.extend(leaf_hits);
            }
            Value::Array(values) => {
                for value in values {
                    walk(value, None, at, rules, hits);
                }
            }
            Value::Object(values) => {
                let here = block_kind(values).unwrap_or(at);
                // A structured name/value object: judge its value fields as a pair.
                let pair = pair::object_pair(values);
                for (key, value) in values.iter_mut() {
                    if let (Some(p), Value::String(text)) = (&pair, &mut *value) {
                        if p.value_keys.contains(key) {
                            hits.push(Finding {
                                origin: Some(here),
                                ..pair_finding(&p.name, text, p.basis)
                            });
                            if Rule::Pair.selected(rules) {
                                "[REDACTED:pair]".clone_into(text);
                                continue;
                            }
                        }
                    }
                    walk(value, Some(key), child_kind(here, key), rules, hits);
                }
            }
            _ => {}
        }
        if matches!(v, Value::String(_)) {
            for hit in &mut hits[start..] {
                hit.origin = Some(at);
            }
        }
    }
    let mut hits = Vec::new();
    walk(v, None, "other", rules, &mut hits);
    hits
}

/// One stored field's detections for one rule, as ingest records them.
/// Like `Finding`, it carries no value, fragment or digest.
#[derive(Debug, PartialEq, Eq)]
pub struct FieldHits {
    /// `raw`, `content` or `search_text`.
    pub field: &'static str,
    pub rule: Rule,
    pub hits: u32,
    /// Of `hits`, those whose value holds a reference or a placeholder.
    pub reference_hits: u32,
    pub placeholder_hits: u32,
    /// Secret-NAMED keys (`assign` only), deduplicated, at most `MAX_KEY_NAMES`.
    pub key_names: Vec<String>,
    /// Whether this rule replaced the value(s) before storage.
    pub redacted: bool,
    /// The distinct [`Finding::origin`]s of the hits, sorted; empty for
    /// `search_text`.
    pub origins: Vec<&'static str>,
}

const MAX_KEY_NAMES: usize = 8;

fn group(field: &'static str, findings: Vec<Finding>, redact_rules: &[Rule]) -> Vec<FieldHits> {
    let mut out: Vec<FieldHits> = Vec::new();
    for f in findings {
        let entry = if let Some(e) = out.iter_mut().find(|e| e.rule == f.rule) {
            e
        } else {
            out.push(FieldHits {
                field,
                rule: f.rule,
                hits: 0,
                reference_hits: 0,
                placeholder_hits: 0,
                key_names: Vec::new(),
                redacted: f.rule.selected(redact_rules),
                origins: Vec::new(),
            });
            out.last_mut().expect("just pushed")
        };
        entry.hits += 1;
        entry.reference_hits += u32::from(f.reference);
        entry.placeholder_hits += u32::from(f.placeholder);
        if let Some(o) = f.origin {
            if let Err(at) = entry.origins.binary_search(&o) {
                entry.origins.insert(at, o);
            }
        }
        if let Some(k) = f.key {
            if entry.key_names.len() < MAX_KEY_NAMES && !entry.key_names.contains(&k) {
                entry.key_names.push(k);
            }
        }
    }
    out
}

/// Scan the three fields ingest stores for a message, replacing values in
/// place for `redact_rules` only, and return what was found per field and
/// rule. With no redact rules nothing is modified (flag-only).
pub fn apply_to_message(
    raw: &mut Value,
    content: Option<&mut Value>,
    search_text: Option<&mut String>,
    redact_rules: &[Rule],
) -> Vec<FieldHits> {
    let mut out = group("raw", redact_json(raw, redact_rules), redact_rules);
    if let Some(c) = content {
        out.extend(group("content", redact_json(c, redact_rules), redact_rules));
    }
    if let Some(t) = search_text {
        let (redacted, findings) = redact(t, redact_rules);
        if !redact_rules.is_empty() {
            *t = redacted;
        }
        out.extend(group("search_text", findings, redact_rules));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    const PREFIXES: [(&str, usize); 22] = [
        ("ghp_", 36),
        ("gho_", 36),
        ("ghu_", 36),
        ("ghs_", 36),
        ("ghr_", 36),
        ("github_pat_", 22),
        ("sk-ant-", 20),
        ("sk-", 20),
        ("sk-proj-", 20),
        ("xoxa-", 10),
        ("xoxb-", 10),
        ("xoxp-", 10),
        ("xoxo-", 10),
        ("xoxs-", 10),
        ("xoxr-", 10),
        ("AKIA", 16),
        ("hvs.", 20),
        ("hvb.", 20),
        ("hvr.", 20),
        ("glpat-", 20),
        ("glrt-", 20),
        ("sk-proj-", 64),
    ];

    // Fixed-seed LCG: all credential-shaped payloads are built at runtime.
    fn synthetic(len: usize, alphabet: &[u8]) -> String {
        let mut seed = 0x34_cc_u64;
        (0..len)
            .map(|_| {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                char::from(alphabet[((seed >> 32) as usize) % alphabet.len()])
            })
            .collect()
    }

    fn token(prefix: &str, len: usize) -> String {
        let alphabet = if prefix == "AKIA" {
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".as_slice()
        } else {
            ALPHABET
        };
        format!("{prefix}{}", synthetic(len, alphabet))
    }

    fn pem(label: &str) -> String {
        format!(
            "-----BEGIN {label} PRIVATE KEY-----\n{}\n-----END {label} PRIVATE KEY-----",
            synthetic(96, ALPHABET)
        )
    }

    fn check(condition: bool, rule: Rule, fixture: &str) {
        assert!(condition, "rule {} fixture {fixture}", rule.id());
    }

    fn no_value(hits: &[Finding], value: &str, fixture: &str) {
        // Inspect fields directly, so even a regressed finding cannot render a
        // value. Check complete values and every meaningful eight-byte fragment.
        for hit in hits {
            for metadata in [
                hit.rule.id(),
                hit.key.as_deref().unwrap_or(""),
                hit.value_len_bucket,
            ] {
                check(!metadata.contains(value), hit.rule, fixture);
                for fragment in value.as_bytes().windows(8) {
                    if let Ok(fragment) = std::str::from_utf8(fragment) {
                        check(!metadata.contains(fragment), hit.rule, fixture);
                    }
                }
            }
        }
    }

    fn positive(text: &str, value: &str, rule: Rule, fixture: &str, expected: &str) {
        let scanned = scan(text);
        check(scanned.iter().any(|hit| hit.rule == rule), rule, fixture);
        no_value(&scanned, value, fixture);
        let (output, hits) = redact(text, &RULES);
        check(output == expected, rule, fixture);
        check(!output.contains(value), rule, fixture);
        check(hits == scanned, rule, fixture);
        no_value(&hits, value, fixture);
        let (again, again_hits) = redact(&output, &RULES);
        check(again == output, rule, fixture);
        assert_eq!(again_hits.len(), 0, "rule {} fixture {fixture}", rule.id());
    }

    #[test]
    fn pem_positive() {
        for label in ["OPENSSH", "RSA"] {
            let value = pem(label);
            positive(
                &format!("before\n{value}\nafter"),
                &value,
                Rule::Pem,
                label,
                "before\n[REDACTED:pem]\nafter",
            );
        }
    }

    #[test]
    fn prefix_positive() {
        for (prefix, len) in PREFIXES {
            let value = token(prefix, len);
            positive(
                &format!("before {value} after"),
                &value,
                Rule::Prefix,
                prefix,
                "before [REDACTED:prefix] after",
            );
        }
    }

    // LM Studio keys are `sk-lm-<8>:<20>`: the colon ends the generic sk- run at
    // 11 chars, under its 20-char floor, so one was stored in clear (Gitea #64).
    #[test]
    fn lm_studio_colon_form() {
        let value = format!(
            "sk-lm-{}:{}",
            synthetic(8, ALPHABET),
            synthetic(20, ALPHABET)
        );
        positive(
            &format!(r#"{{"apiKey":"{value}"}}"#),
            &value,
            Rule::Prefix,
            "sk-lm-",
            r#"{"apiKey":"[REDACTED:prefix]"}"#,
        );
        positive(
            &format!("export LM_KEY {value} now"),
            &value,
            Rule::Prefix,
            "sk-lm- bare",
            "export LM_KEY [REDACTED:prefix] now",
        );
        for (fixture, text) in [
            (
                "short tail",
                format!(
                    "sk-lm-{}:{}",
                    synthetic(8, ALPHABET),
                    synthetic(15, ALPHABET)
                ),
            ),
            (
                "short head",
                format!(
                    "sk-lm-{}:{}",
                    synthetic(7, ALPHABET),
                    synthetic(20, ALPHABET)
                ),
            ),
            ("prose", "the sk-lm-prefix: a key format".to_owned()),
        ] {
            assert_eq!(scan(&text).len(), 0, "rule prefix fixture {fixture}");
        }
    }

    // GitLab tokens: legacy `glpat-<20>`/`glrt-<20>`, and the routable form
    // `<prefix><payload>.<version>.<check>` whose dotted tail must go with it.
    // A runner list printed two runner tokens into a transcript in clear
    // (Gitea #70, 2026-10-17).
    #[test]
    fn gitlab_routable_form() {
        for prefix in ["glpat-", "glrt-"] {
            let value = format!(
                "{prefix}t1_{}.01.{}",
                synthetic(27, ALPHABET),
                synthetic(9, ALPHABET)
            );
            positive(
                &format!("runner Token={value} URL=https://gitlab.example"),
                &value,
                Rule::Prefix,
                prefix,
                "runner Token=[REDACTED:prefix] URL=https://gitlab.example",
            );
            positive(
                &format!("ends a sentence {value}."),
                &value,
                Rule::Prefix,
                prefix,
                "ends a sentence [REDACTED:prefix].",
            );
        }
        for (fixture, text) in [
            (
                "glpat prose",
                "the glpat- prefix marks a GitLab PAT".to_owned(),
            ),
            ("glrt prose", "glrt-runner tokens rotate".to_owned()),
            ("glrt short", format!("glrt-{}", synthetic(19, ALPHABET))),
        ] {
            assert_eq!(scan(&text).len(), 0, "rule prefix fixture {fixture}");
        }
    }

    // A file name that merely starts with a GitLab prefix is not a token: cchv's
    // own backup names tripped the rule on 2026-10-21, and every message that
    // quoted them re-tripped it. Real tails are random mixed case; these are
    // lowercase words and numbers joined by hyphens, maybe with an extension.
    // Fixtures are assembled from parts so this source never holds the literal.
    #[test]
    fn gitlab_prefix_file_names_are_not_tokens() {
        let date = "2026".to_owned() + "1019";
        for prefix in ["glpat-", "glrt-"] {
            for tail in [
                format!("redact-{date}-record"),
                format!("redact-{date}-record.txt"),
                format!("redact-{date}-113091590.tsv"),
            ] {
                let text = format!("backup in ~/.config/cchv/backups/{prefix}{tail} (mode 600)");
                assert_eq!(scan(&text).len(), 0, "file name {prefix}{tail}");
            }
        }
    }

    // A placeholder (`XXXX…`) or a same-length mask of a token (letters to
    // `A`/`a`, digits to `9`) is not a token, though it keeps the token's shape:
    // posting such masks on 2026-10-21 created new findings. Real payloads are
    // random, so 16+ characters drawn from three or fewer distinct ones is the tell.
    fn mask(payload: &str) -> String {
        payload
            .chars()
            .map(|c| match c {
                'A'..='Z' => 'A',
                'a'..='z' => 'a',
                '0'..='9' => '9',
                other => other,
            })
            .collect()
    }

    #[test]
    fn placeholders_and_masks_are_not_tokens() {
        for (prefix, len) in PREFIXES {
            if len < 16 {
                continue;
            }
            let real = token(prefix, len);
            for (fixture, text) in [
                ("mask", format!("{prefix}{}", mask(&real[prefix.len()..]))),
                ("placeholder", format!("{prefix}{}", "X".repeat(len))),
            ] {
                assert_eq!(
                    scan(&format!("value {text} here")).len(),
                    0,
                    "{fixture} after {prefix}"
                );
            }
        }
    }

    #[test]
    fn gitlab_hyphenated_tail_with_upper_case_is_still_a_token() {
        for prefix in ["glpat-", "glrt-"] {
            let value = format!("{prefix}aB3d-{}", synthetic(20, ALPHABET));
            positive(
                &format!("token {value} here"),
                &value,
                Rule::Prefix,
                prefix,
                "token [REDACTED:prefix] here",
            );
        }
    }

    // G3, ac/security 2026-10-22: an `assign` value of 32+ chars with all four
    // character classes, or 64+ with three, is `assign-strong`, redacted with a
    // fixed marker (never a same-length mask, which stays secret-shaped).
    fn strong_value() -> String {
        format!("aB3-{}", synthetic(36, ALPHABET))
    }

    fn long_three_class_value() -> String {
        format!(
            "a1-{}",
            synthetic(61, b"abcdefghijklmnopqrstuvwxyz0123456789")
        )
    }

    fn hex_sha() -> String {
        synthetic(40, b"0123456789abcdef")
    }

    #[test]
    fn assign_strong_is_a_rule_id() {
        assert_eq!(Rule::from_id("assign-strong"), Some(Rule::AssignStrong));
        assert!(Rule::ALL.contains(&Rule::AssignStrong));
    }

    #[test]
    fn assign_strong_is_redacted_with_a_fixed_marker() {
        for value in [strong_value(), long_three_class_value()] {
            let text = format!("API_TOKEN={value} next");
            let (out, hits) = redact(&text, &[Rule::AssignStrong]);
            assert_eq!(out, "API_TOKEN=[REDACTED:assign-strong] next");
            assert_eq!(
                hits.iter().map(|h| h.rule).collect::<Vec<_>>(),
                vec![Rule::AssignStrong]
            );
        }
    }

    #[test]
    fn assign_strong_controls_are_untouched() {
        let sha = format!("COMMIT_TOKEN={}", hex_sha());
        let (out, hits) = redact(&sha, &[Rule::AssignStrong]);
        assert_eq!(out, sha, "a 40-char hex digest has 2 classes");
        assert_eq!(
            hits.iter().map(|h| h.rule).collect::<Vec<_>>(),
            vec![Rule::Assign]
        );
        let short = "TOKEN=abc123 here";
        assert_eq!(redact(short, &[Rule::AssignStrong]).0, short);
    }

    #[test]
    fn selecting_assign_covers_assign_strong() {
        let text = format!("API_TOKEN={} next", strong_value());
        assert_eq!(
            redact(&text, &[Rule::Assign]).0,
            "API_TOKEN=[REDACTED:assign-strong] next"
        );
        assert!(Rule::AssignStrong.selected(&[Rule::Assign]));
        assert!(!Rule::Assign.selected(&[Rule::AssignStrong]));
    }

    #[test]
    fn assign_strong_replaces_a_whole_json_value() {
        let mut strong = serde_json::json!({ "api_key": strong_value() });
        let hits = redact_json(&mut strong, &[Rule::AssignStrong]);
        assert_eq!(strong["api_key"], "[REDACTED:assign-strong]");
        assert!(hits.iter().any(|h| h.rule == Rule::AssignStrong));
        let mut weak = serde_json::json!({ "api_key": hex_sha() });
        let before = weak.clone();
        redact_json(&mut weak, &[Rule::AssignStrong]);
        assert_eq!(weak, before);
    }

    #[test]
    fn bearer_positive() {
        let value = synthetic(
            48,
            b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._~+/=-",
        );
        for header in ["Bearer", "bearer", "BEARER"] {
            positive(
                &format!("Authorization: {header} {value}"),
                &value,
                Rule::Bearer,
                header,
                &format!("Authorization: {header} [REDACTED:bearer]"),
            );
        }
    }

    #[test]
    fn assign_positive() {
        let value = synthetic(24, ALPHABET);
        let fixtures = [
            (
                "env",
                format!("NATS_PASSWORD={value}"),
                "NATS_PASSWORD",
                "NATS_PASSWORD=[REDACTED:assign]",
            ),
            (
                "yaml",
                format!("password: {value}"),
                "password",
                "password: [REDACTED:assign]",
            ),
            (
                "json-text",
                format!(r#"{{"password": "{value}"}}"#),
                "password",
                r#"{"password": "[REDACTED:assign]"}"#,
            ),
            (
                "nats",
                format!("authorization {{ user: x password: {value} }}"),
                "password",
                "authorization { user: x password: [REDACTED:assign] }",
            ),
            (
                "quoted-env",
                format!("export API_KEY = '{value}'"),
                "API_KEY",
                "export API_KEY = '[REDACTED:assign]'",
            ),
            (
                "mixed-case",
                format!("service.Credential = \"{value}\""),
                "service.Credential",
                "service.Credential = \"[REDACTED:assign]\"",
            ),
        ];
        for (fixture, text, key, expected) in fixtures {
            positive(&text, &value, Rule::Assign, fixture, expected);
            check(
                scan(&text)[0].key.as_deref() == Some(key),
                Rule::Assign,
                fixture,
            );
        }
        for key in SECRET_NAMES {
            let text = format!("MY_{key}_VALUE={value}");
            check(scan(&text).len() == 1, Rule::Assign, key);
        }
    }

    #[test]
    fn negative_fixtures() {
        let fixtures = [
            ("input-tokens", r#""input_tokens": 1234"#),
            ("max-tokens", "max_tokens=4096"),
            ("tokenizer", "tokenizer: cl100k_base"),
            ("token-count", "token_count: 17"),
            ("variable", "password: $DB_PASSWORD"),
            ("braced-variable", "password: ${PW}"),
            ("angle-placeholder", "token=<token>"),
            ("long-angle-placeholder", "token=<placeholder>"),
            ("stars", "password: ********"),
            ("xs", "password: xxxxxxxx"),
            ("upper-xs", "password: XXXXXXXX"),
            ("prose", "rotate the password tomorrow"),
            ("marker", "password: [REDACTED:assign]"),
            ("pem-marker", "[REDACTED:pem]"),
            ("prefix-marker", "[REDACTED:prefix]"),
            ("bearer-marker", "Bearer [REDACTED:bearer]"),
            ("quoted-variable", "password: '${DB_PASSWORD}'"),
            ("digits", "password: 12345678901234567890"),
            ("code-call", "let token = generate_token();"),
            ("code-env", "const apiKey = process.env.API_KEY;"),
            ("code-index", "password = request.form['password']"),
            ("code-attr", "self.secret = settings.secret_value"),
            (
                "actions-secret",
                "GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}",
            ),
            (
                "public-pem",
                "-----BEGIN PUBLIC KEY-----\nignored\n-----END PUBLIC KEY-----",
            ),
        ];
        for (fixture, text) in fixtures {
            assert_eq!(scan(text).len(), 0, "rule assign fixture {fixture}");
            let (output, hits) = redact(text, &RULES);
            check(output == text, Rule::Assign, fixture);
            assert_eq!(hits.len(), 0, "rule assign fixture {fixture}");
        }
    }

    #[test]
    fn deny_list_whole_key() {
        let value = synthetic(24, ALPHABET);
        for key in DENIED_KEYS {
            for key in [key.to_owned(), key.to_ascii_uppercase()] {
                let text = format!("{key}: {value}");
                assert_eq!(scan(&text).len(), 0, "rule assign fixture {key}");
                let mut json = serde_json::json!({key.clone(): value});
                assert_eq!(
                    redact_json(&mut json, &RULES).len(),
                    0,
                    "rule assign fixture {key}"
                );
            }
        }
        // Denials apply to the whole key, not a suffix/subsequence.
        check(
            scan(&format!("my_secret_name={value}")).len() == 1,
            Rule::Assign,
            "whole-key",
        );
    }

    #[test]
    fn prefix_and_bearer_length_floors() {
        for (prefix, len) in PREFIXES.into_iter().take(PREFIXES.len() - 1) {
            let value = token(prefix, len - 1);
            assert_eq!(scan(&value).len(), 0, "rule prefix fixture {prefix}");
        }
        let value = synthetic(19, ALPHABET);
        assert_eq!(
            scan(&format!("Bearer {value}")).len(),
            0,
            "rule bearer fixture short"
        );
        let value = synthetic(20, ALPHABET);
        check(
            scan(&format!("Bearer {value}")).len() == 1,
            Rule::Bearer,
            "floor",
        );
    }

    #[test]
    fn assign_guards_and_metadata() {
        for (len, bucket) in [
            (8, "8-15"),
            (15, "8-15"),
            (16, "16-31"),
            (31, "16-31"),
            (32, "32-63"),
            (63, "32-63"),
            (64, "64+"),
        ] {
            let value = synthetic(len, ALPHABET);
            let hits = scan(&format!("password={value}"));
            check(hits.len() == 1, Rule::Assign, bucket);
            check(hits[0].value_len_bucket == bucket, Rule::Assign, bucket);
            no_value(&hits, &value, bucket);
        }
        let short = synthetic(7, ALPHABET);
        assert_eq!(
            scan(&format!("password={short}")).len(),
            0,
            "rule assign fixture short"
        );
        for (alphabet, class, fixture) in [
            (b"abc".as_slice(), 1, "lower"),
            (b"ABC".as_slice(), 2, "upper"),
            (b"!@#".as_slice(), 8, "symbol"),
        ] {
            let value = synthetic(12, alphabet);
            let hits = scan(&format!("password='{value}'"));
            check(
                hits.len() == 1 && hits[0].value_classes == class,
                Rule::Assign,
                fixture,
            );
        }
        let value = format!(
            "{}{}{}{}",
            synthetic(3, b"abc"),
            synthetic(3, b"ABC"),
            synthetic(3, b"123"),
            synthetic(3, b"!@#")
        );
        check(
            scan(&format!("password='{value}'"))[0].value_classes == 15,
            Rule::Assign,
            "all-classes",
        );
        let unicode: String = synthetic(8, b"a").chars().map(|_| 'é').collect();
        let hits = scan(&format!("password='{unicode}'"));
        check(hits[0].value_len_bucket == "8-15", Rule::Assign, "unicode");
    }

    #[test]
    fn redact_json_nested_pairs_and_leaves() {
        let value = synthetic(24, ALPHABET);
        let prefixed = token("ghp_", 36);
        let bearer = synthetic(20, ALPHABET);
        let mut json = serde_json::json!({
            "data": {"data": {"password": value, "NATS_PASSWORD": value}},
            "items": [format!("result {prefixed}"), {"body": format!("Bearer {bearer}")}, {"token": value}],
            "input_tokens": 1234,
            "enabled": true,
            "empty": null,
            "password": {"note": "rotate the password tomorrow"},
        });
        let hits = redact_json(&mut json, &RULES);
        check(hits.len() == 5, Rule::Assign, "nested-json");
        check(
            hits.iter()
                .filter(|hit| hit.key.as_deref() == Some("password"))
                .count()
                == 1,
            Rule::Assign,
            "bao-json",
        );
        no_value(&hits, &value, "nested-json");
        no_value(&hits, &prefixed, "nested-json-prefix");
        no_value(&hits, &bearer, "nested-json-bearer");
        let expected = serde_json::json!({
            "data": {"data": {"password": "[REDACTED:assign]", "NATS_PASSWORD": "[REDACTED:assign]"}},
            "items": ["result [REDACTED:prefix]", {"body": "Bearer [REDACTED:bearer]"}, {"token": "[REDACTED:assign]"}],
            "input_tokens": 1234,
            "enabled": true,
            "empty": null,
            "password": {"note": "rotate the password tomorrow"},
        });
        check(json == expected, Rule::Assign, "nested-json");
        assert_eq!(
            redact_json(&mut json, &RULES).len(),
            0,
            "rule assign fixture json-idempotence"
        );
        check(json == expected, Rule::Assign, "json-idempotence");
    }

    #[test]
    fn json_assignment_guards() {
        for (fixture, value) in [
            ("variable", "$DB_PASSWORD"),
            ("braced-variable", "${LONG_VARIABLE}"),
            ("angle", "<placeholder>"),
            ("stars", "********"),
            ("xs", "xxxxxxxx"),
            ("marker", "[REDACTED:assign]"),
            ("digits", "12345678901234567890"),
        ] {
            let mut json = serde_json::json!({"password": value});
            let expected = json.clone();
            assert_eq!(
                redact_json(&mut json, &RULES).len(),
                0,
                "rule assign fixture {fixture}"
            );
            check(json == expected, Rule::Assign, fixture);
        }
    }

    #[test]
    fn selected_rules_and_overlaps() {
        let value = token("ghp_", 36);
        let text = format!("password=\"Bearer {value}\"");
        let scanned = scan(&text);
        check(scanned.len() == 3, Rule::Assign, "overlap");
        for (rules, expected, fixture) in [
            (&[][..], text.clone(), "flag-only"),
            (
                &[Rule::Prefix][..],
                "password=\"Bearer [REDACTED:prefix]\"".to_owned(),
                "prefix-only",
            ),
            (
                &[Rule::Bearer][..],
                "password=\"Bearer [REDACTED:bearer]\"".to_owned(),
                "bearer-only",
            ),
            // The assigned value (`Bearer <token>`, 47 chars, four classes) is
            // strong-shaped, so `assign` selection replaces it as `assign-strong`.
            (
                &[Rule::Assign][..],
                "password=\"[REDACTED:assign-strong]\"".to_owned(),
                "assign-only",
            ),
            (
                &RULES[..],
                "password=\"[REDACTED:assign-strong]\"".to_owned(),
                "all-rules",
            ),
        ] {
            let (output, hits) = redact(&text, rules);
            check(output == expected && hits == scanned, Rule::Assign, fixture);
            check(redact(&output, rules).0 == output, Rule::Assign, fixture);
            no_value(&hits, &value, fixture);
        }
        let mut json = serde_json::json!({"password": value});
        let hits = redact_json(&mut json, &[Rule::Prefix]);
        check(hits.len() == 2, Rule::Assign, "json-overlap");
        check(
            json["password"] == "[REDACTED:prefix]",
            Rule::Prefix,
            "json-overlap",
        );
    }

    #[test]
    fn adjacent_matches_and_unicode_context() {
        let password = synthetic(24, ALPHABET);
        let prefixed = token("hvs.", 20);
        let text = format!("한글 password='{password}', token='{password}'; {prefixed} 終");
        positive(
            &text,
            &password,
            Rule::Assign,
            "adjacent-unicode",
            "한글 password='[REDACTED:assign]', token='[REDACTED:assign]'; [REDACTED:prefix] 終",
        );
        check(scan(&text).len() == 3, Rule::Assign, "adjacent-unicode");
    }

    #[test]
    fn quoted_values_and_pem_overlap() {
        let value = format!("{}\\\"{}", synthetic(12, ALPHABET), synthetic(12, ALPHABET));
        positive(
            &format!(r#"{{"password":"{value}"}}"#),
            &value,
            Rule::Assign,
            "escaped-quote",
            r#"{"password":"[REDACTED:assign]"}"#,
        );
        let block = pem("RSA");
        let text = format!("PRIVATE_KEY={block}");
        let (output, hits) = redact(&text, &RULES);
        check(
            output == "PRIVATE_KEY=[REDACTED:pem]",
            Rule::Pem,
            "pem-overlap",
        );
        check(
            hits.iter().any(|hit| hit.rule == Rule::Pem),
            Rule::Pem,
            "pem-overlap",
        );
        check(
            redact(&output, &RULES).0 == output,
            Rule::Pem,
            "pem-overlap",
        );
        let invalid = format!(
            "-----BEGIN RSA PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----",
            synthetic(64, ALPHABET)
        );
        assert_eq!(scan(&invalid).len(), 0, "rule pem fixture mismatched-label");
    }

    #[test]
    #[ignore = "manual 8 MiB synthetic transcript scan; no timing assertion"]
    fn benchmark_scan_8_mib() {
        const SIZE: usize = 8 * 1024 * 1024;
        let line = format!("user: inspect module {}\nassistant: rotate the password tomorrow\ninput_tokens: 1234 max_tokens=4096\n", synthetic(72, ALPHABET));
        let mut text = line.repeat(SIZE.div_ceil(line.len()));
        text.truncate(SIZE);
        // Warm lazy compilation before measuring steady-state transcript scanning.
        assert_eq!(
            scan("ordinary transcript").len(),
            0,
            "rule assign fixture bench-control"
        );
        let start = std::time::Instant::now();
        let hits = scan(std::hint::black_box(&text));
        let elapsed = start.elapsed();
        assert_eq!(hits.len(), 0, "rule assign fixture bench-8-mib");
        println!(
            "8 MiB synthetic transcript scan: {:.3} ms",
            elapsed.as_secs_f64() * 1000.0
        );
        // Positive control: the same instrument must see a synthetic credential.
        text.push_str(&format!("\nNATS_PASSWORD={}\n", synthetic(24, ALPHABET)));
        check(
            scan(&text).len() == 1,
            Rule::Assign,
            "bench-positive-control",
        );
    }

    #[test]
    fn references_are_flagged() {
        let refs = [
            format!("{}{}", "${", "DEPLOY_KEY}"),
            format!("{}{}", "$(", "bao kv get x)"),
            format!("{}{}", "%", "SVC_PASS%"),
            format!("{}{}", "{{", " vault_pass }}"),
            format!("{}{}", "#{", "secret}"),
        ];
        for r in &refs {
            assert!(is_reference(r), "{r} should be a reference");
        }
        for plain in ["aB3-qwertyuiop", "100%", "50% off 20%x", "$HOME"] {
            assert!(!is_reference(plain), "{plain} is not a reference");
        }
    }

    #[test]
    fn placeholders_are_flagged_case_insensitively() {
        let words = [
            "x".repeat(3),
            format!("{}{}", "CHANGE", "ME"),
            format!("my-{}", "Example-1"),
            format!("{}{}", "dum", "my"),
            format!("[{}]", "REDACTED"),
            format!("<{}>", "your token"),
            "*".repeat(3),
        ];
        for w in &words {
            assert!(is_placeholder(w), "{w} should be a placeholder");
        }
        for plain in ["aB3-qwertyuiop", "a<b", "**", "xx-yy"] {
            assert!(!is_placeholder(plain), "{plain} is not a placeholder");
        }
    }

    #[test]
    fn json_values_carry_the_origin() {
        let strong = format!("aB3-{}", "Zq9".repeat(12));
        let line = serde_json::json!({
            "type": "user",
            "content": [
                {"type": "tool_result", "tool_use_id": "call-1",
                 "content": format!("API_{}={strong}", "TOKEN")},
            ],
            "toolUseResult": {"stdout": format!("DB_{}={strong}", "PASSWORD")},
        });
        let hits = json_values(&line);
        assert_eq!(hits.len(), 2);
        for (_, _, origin) in &hits {
            assert_eq!(origin.kind, "tool_result");
            assert_eq!(origin.tool_use_id.as_deref(), Some("call-1"));
        }

        let call = serde_json::json!({"message": {"content": [
            {"type": "text", "text": format!("API_{}={strong}", "TOKEN")},
            {"type": "tool_use", "id": "call-2", "name": "Bash",
             "input": {"command": format!("export API_{}={strong}", "TOKEN")}},
        ]}});
        let kinds: Vec<_> = json_values(&call)
            .into_iter()
            .map(|(_, _, o)| (o.kind, o.tool_use_id))
            .collect();
        assert_eq!(
            kinds,
            vec![("text", None), ("tool_input", Some("call-2".to_owned()))]
        );
    }

    #[test]
    fn a_reference_is_never_assign_strong() {
        let tail = "Z9q8W7e6R5t4Y3u2I1o0P9";
        let reference = format!("API_{}=\"svc-{}{}-{tail}\"", "TOKEN", "${", "DB_PASS}");
        let hits = scan(&reference);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].rule, hits[0].reference), (Rule::Assign, true));
        let (out, _) = redact(&reference, &[Rule::AssignStrong]);
        assert_eq!(out, reference, "assign-strong leaves a reference alone");

        let placeholder = format!("API_{}=\"{}-{tail}-qq\"", "TOKEN", "Example");
        let hits = scan(&placeholder);
        assert_eq!(
            (hits[0].rule, hits[0].placeholder),
            (Rule::AssignStrong, true)
        );
        let (out, _) = redact(&placeholder, &[Rule::AssignStrong]);
        assert!(out.contains("[REDACTED:assign-strong]") && !out.contains(tail));
    }

    #[test]
    fn field_hits_count_the_flags() {
        let tail = "Z9q8W7e6R5t4Y3u2I1o0P9";
        let mut raw = serde_json::json!({
            "a": format!("API_{}=\"svc-{}{}-{tail}\"", "TOKEN", "${", "DB_PASS}"),
            "b": format!("API_{}=\"{}-{tail}-qq\"", "TOKEN", "Example"),
        });
        let hits = apply_to_message(&mut raw, None, None, &[]);
        let count = |rule: Rule| {
            let h = hits.iter().find(|h| h.rule == rule).expect("rule present");
            (h.hits, h.reference_hits, h.placeholder_hits)
        };
        assert_eq!(count(Rule::Assign), (1, 1, 0));
        assert_eq!(count(Rule::AssignStrong), (1, 0, 1));
    }

    /// Gitea #82: AWS's documentation example key is printed on every AWS docs
    /// page; a random key id of the same shape still fires.
    #[test]
    fn aws_documentation_example_key_is_not_a_finding() {
        let example = ["AKIA", "IOSFODNN7", "EXAMPLE"].concat();
        let text = format!("aws_access_key_id = {example}\nother {}", token("AKIA", 16));
        let hits: Vec<Rule> = scan(&text).into_iter().map(|f| f.rule).collect();
        assert_eq!(hits, vec![Rule::Prefix], "only the random key fires");
        let (out, _) = redact(&format!("key {example} here"), &[Rule::Prefix]);
        assert!(out.contains(&example), "the example is left in place");
    }

    /// Gitea #82: a PEM header with no body and no END line is not a key.
    #[test]
    fn bare_pem_header_fires_nothing() {
        for label in ["RSA", "OPENSSH", "EC"] {
            let text = format!("Your key file starts with -----BEGIN {label} PRIVATE KEY----- and");
            assert!(scan(&text).is_empty(), "{label}");
        }
    }

    /// Gitea #82: each hit records the content block it sat in.
    #[test]
    fn redact_json_records_where_each_hit_sat() {
        let secret = || token("ghp_", 36);
        let mut line = serde_json::json!({
            "content": [
                { "type": "text", "text": format!("typed {}", secret()) },
                { "type": "tool_result", "tool_use_id": "t1", "content": format!("fetched {}", secret()) },
                { "type": "tool_use", "id": "t2", "input": { "command": format!("run {}", secret()) } },
            ],
            "toolUseResult": { "stdout": format!("out {}", secret()) },
            "summary": format!("loose {}", secret()),
        });
        let mut origins: Vec<&str> = redact_json(&mut line, &[])
            .into_iter()
            .map(|f| f.origin.expect("JSON hits carry an origin"))
            .collect();
        origins.sort_unstable();
        assert_eq!(
            origins,
            ["other", "text", "tool_input", "tool_result", "tool_result"]
        );
        let (_, plain) = redact(&format!("plain {}", secret()), &[]);
        assert_eq!(plain[0].origin, None);
    }
}
