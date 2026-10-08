//! Credential-shape detector for Gitea #34, independent of ingest and storage.
//!
//! Hard rule: nothing here may render, log or hash a value. Findings retain only
//! rule ids, secret-named keys and coarse shape metadata; spans stay internal.

use regex::{Regex, RegexSet};
use serde_json::Value;
use std::sync::LazyLock;

pub const DETECTOR_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    Pem,
    Prefix,
    Bearer,
    Assign,
}

impl Rule {
    pub fn id(&self) -> &'static str {
        match self {
            Self::Pem => "pem",
            Self::Prefix => "prefix",
            Self::Bearer => "bearer",
            Self::Assign => "assign",
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
}

const RULES: [Rule; 4] = [Rule::Pem, Rule::Prefix, Rule::Bearer, Rule::Assign];
const PATTERNS: [&str; 4] = [
    r"-----BEGIN (?P<label>(?:[A-Z0-9]+ )*PRIVATE KEY)-----[\s\S]*?-----END (?P<end_label>(?:[A-Z0-9]+ )*PRIVATE KEY)-----",
    r"\b(?:gh[pousr]_[A-Za-z0-9_\-]{36,}|github_pat_[A-Za-z0-9_\-]{22,}|sk-ant-[A-Za-z0-9_\-]{20,}|sk-(?:proj-)?[A-Za-z0-9_\-]{20,}|xox[abposr]-[A-Za-z0-9_\-]{10,}|AKIA[0-9A-Z]{16}\b|hv[sbr]\.[A-Za-z0-9_\-]{20,})",
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

fn placeholder(value: &str) -> bool {
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
    Finding {
        rule,
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
    .then(|| finding(Rule::Assign, Some(key), value))
}

// Byte offsets never escape the detector. No secret is copied into a span.
struct Span {
    start: usize,
    end: usize,
    finding: Finding,
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
                    }) {
                        continue;
                    }
                    (value, finding(rule, None, value.as_str()))
                }
                Rule::Bearer => {
                    let value = captures.name("value").expect("bearer value capture");
                    (value, finding(rule, None, value.as_str()))
                }
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
            });
        }
    }
    // Enclosing spans first; ties retain pem/prefix/bearer/assign precedence.
    spans.sort_by_key(|span| (span.start, std::cmp::Reverse(span.end)));
    spans
}

/// Scan every rule; return metadata only, including overlapping detections.
pub fn scan(text: &str) -> Vec<Finding> {
    spans(text).into_iter().map(|span| span.finding).collect()
}

/// Replace values for the selected rules and return findings for ALL rules.
/// Overlapping replacements share the enclosing/leftmost span's marker.
pub fn redact(text: &str, rules: &[Rule]) -> (String, Vec<Finding>) {
    let spans = spans(text);
    let mut replacements: Vec<(usize, usize, Rule)> = Vec::new();
    for span in &spans {
        if !rules.contains(&span.finding.rule) {
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
    (output, spans.into_iter().map(|span| span.finding).collect())
}

/// Walk string leaves and secret-named object keys paired with string values.
/// Keys and non-string leaves are preserved. Findings include flag-only rules.
pub fn redact_json(v: &mut Value, rules: &[Rule]) -> Vec<Finding> {
    fn walk(v: &mut Value, key: Option<&str>, rules: &[Rule], hits: &mut Vec<Finding>) {
        match v {
            Value::String(text) => {
                let assignment = key.and_then(|key| assignment_finding(key, text, false));
                let (redacted, leaf_hits) = redact(text, rules);
                *text = if assignment.is_some() && rules.contains(&Rule::Assign) {
                    "[REDACTED:assign]".to_owned()
                } else {
                    redacted
                };
                hits.extend(assignment);
                hits.extend(leaf_hits);
            }
            Value::Array(values) => {
                for value in values {
                    walk(value, None, rules, hits);
                }
            }
            Value::Object(values) => {
                for (key, value) in values {
                    walk(value, Some(key), rules, hits);
                }
            }
            _ => {}
        }
    }
    let mut hits = Vec::new();
    walk(v, None, rules, &mut hits);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    const PREFIXES: [(&str, usize); 20] = [
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
        for (prefix, len) in PREFIXES.into_iter().take(19) {
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
            (
                &[Rule::Assign][..],
                "password=\"[REDACTED:assign]\"".to_owned(),
                "assign-only",
            ),
            (
                &RULES[..],
                "password=\"[REDACTED:assign]\"".to_owned(),
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
}
