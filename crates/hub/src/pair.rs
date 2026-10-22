//! The `pair` rule (ac/security F7, 2026-10-22): a secret printed as one field of
//! a name/value structure, where no single field matches another rule. A value
//! field fires when it sits in the same object as a name field whose content is
//! secret-named, or as a sensitivity flag set to true.
//!
//! Precision comes from the pairing, so the value floor is low on purpose. As in
//! `redact`, nothing here keeps or logs a value: callers get byte ranges and the
//! secret-NAMED name.

use regex::Regex;
use serde_json::{Map, Value};
use std::sync::LazyLock;

use crate::redact::placeholder;

/// Why a pair fired: (a) a secret-named name, (b) a sensitivity flag, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    Name,
    Flag,
    Both,
}

impl Basis {
    pub fn id(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Flag => "flag",
            Self::Both => "name+flag",
        }
    }

    fn of(named: bool, flagged: bool) -> Option<Basis> {
        match (named, flagged) {
            (true, true) => Some(Self::Both),
            (true, false) => Some(Self::Name),
            (false, true) => Some(Self::Flag),
            (false, false) => None,
        }
    }
}

/// One detected value in a text: its byte range, the name it was paired with
/// and why it fired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPair {
    pub start: usize,
    pub end: usize,
    pub name: String,
    pub basis: Basis,
}

/// A structured JSON object that fires: the name, the basis and the keys of the
/// value fields that pass the floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectPair {
    pub name: String,
    pub basis: Basis,
    pub value_keys: Vec<String>,
}

const NAME_FIELDS: [&str; 4] = ["name", "key", "variable", "id"];
const VALUE_FIELDS: [&str; 4] = ["value", "secret", "data", "val"];
const FLAG_FIELDS: [&str; 6] = [
    "sensitive",
    "issensitive",
    "secure",
    "masked",
    "protected",
    "encrypted",
];

fn is_field(set: &[&str], key: &str) -> bool {
    set.iter().any(|f| key.eq_ignore_ascii_case(f))
}

/// The `assign` lexicon, matched as a substring like `assign` does.
const LONG_NAMES: [&str; 8] = [
    "PASSWORD",
    "PASSWD",
    "SECRET",
    "TOKEN",
    "API_KEY",
    "APIKEY",
    "PRIVATE_KEY",
    "CREDENTIAL",
];
/// Short words matched only as a whole token, so `PATH` is not `PAT` and
/// `AUTHOR` is not `AUTH`.
const TOKENS: [&str; 8] = [
    "PASS", "PASSWD", "PWD", "PAT", "AUTH", "APIKEY", "DSN", "SAS",
];
/// Two-token names: `ACCESS_KEY`, `accessKey`, `ConnectionString`, …
const TOKEN_PAIRS: [(&str, &str); 5] = [
    ("ACCESS", "KEY"),
    ("CLIENT", "SECRET"),
    ("CONN", "STRING"),
    ("CONNECTION", "STRING"),
    ("SIGNING", "KEY"),
];

/// Split a name into upper-case tokens at non-alphanumerics and camelCase
/// boundaries: `DbConnectionString` -> `DB`, `CONNECTION`, `STRING`.
fn tokens(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if !c.is_ascii_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if c.is_ascii_uppercase() && prev_lower && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        cur.push(c.to_ascii_uppercase());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Whether a name field's content is secret-named (shape (a)).
pub fn secret_named(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if LONG_NAMES.iter().any(|n| upper.contains(n)) {
        return true;
    }
    let toks = tokens(name);
    toks.iter().any(|t| TOKENS.contains(&t.as_str()))
        || toks
            .windows(2)
            .any(|w| TOKEN_PAIRS.contains(&(w[0].as_str(), w[1].as_str())))
        || toks.len() >= 2 && toks.last().is_some_and(|t| t == "KEY")
}

/// A name field's content as a plain identifier (`DB_PASSWORD`, `smtp.pwd`,
/// `"ApiKey"`), or `None` for code and prose (`StoreKey::from(key),`), which
/// a `key: …` line in source code would otherwise pair.
fn plain_name(raw: &str) -> Option<String> {
    let name = raw
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`');
    (!name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/')))
    .then(|| name.to_owned())
}

fn truthy(v: &str) -> bool {
    v.trim().eq_ignore_ascii_case("true")
}

/// The value floor (security policy § 3). Low on purpose: the pairing is the
/// precision.
pub fn passes_floor(value: &str) -> bool {
    static URL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^[A-Za-z][A-Za-z0-9+.\-]*://([^/?#\s]*)").expect("url pattern")
    });
    static REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"^(?:\$\{[^}]*\}|\$\([^)]*\)|\{\{[^}]*\}\}|#\{[^}]*\}|%[A-Za-z_][A-Za-z0-9_]*%)$",
        )
        .expect("whole reference pattern")
    });
    let v = value.trim();
    if v.chars().count() < 8 || placeholder(v) || REFERENCE.is_match(v) {
        return false;
    }
    if ["true", "false", "yes", "no"]
        .iter()
        .any(|b| v.eq_ignore_ascii_case(b))
    {
        return false;
    }
    if v.len() < 12 && v.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    if let Some(c) = URL.captures(v) {
        if !c[1].contains('@') {
            return false;
        }
    }
    // An existing mask: one repeated character (`********`, `XXXXXXXX`).
    let mut chars = v.chars();
    let first = chars.next();
    !chars.all(|c| Some(c) == first)
}

/// A structured JSON object that fires, if any.
pub fn object_pair(map: &Map<String, Value>) -> Option<ObjectPair> {
    let name = map
        .iter()
        .find(|(k, v)| is_field(&NAME_FIELDS, k) && v.is_string())
        .and_then(|(_, v)| v.as_str())
        .and_then(plain_name)?;
    let flagged = map.iter().any(|(k, v)| {
        is_field(&FLAG_FIELDS, k)
            && match v {
                Value::Bool(b) => *b,
                Value::String(s) => truthy(s),
                _ => false,
            }
    });
    let basis = Basis::of(secret_named(&name), flagged)?;
    let value_keys: Vec<String> = map
        .iter()
        .filter(|(k, v)| is_field(&VALUE_FIELDS, k) && v.as_str().is_some_and(passes_floor))
        .map(|(k, _)| k.clone())
        .collect();
    (!value_keys.is_empty()).then_some(ObjectPair {
        name,
        basis,
        value_keys,
    })
}

/// A field read out of a text shape: lower-case key, value range in the text.
struct Field {
    key: String,
    start: usize,
    end: usize,
}

/// Judge one object's fields; push a hit per value field that fires.
fn judge(text: &str, fields: &[Field], out: &mut Vec<TextPair>) {
    let Some(name) = fields.iter().find(|f| is_field(&NAME_FIELDS, &f.key)) else {
        return;
    };
    let Some(name) = plain_name(&text[name.start..name.end]) else {
        return;
    };
    let flagged = fields
        .iter()
        .any(|f| is_field(&FLAG_FIELDS, &f.key) && truthy(&text[f.start..f.end]));
    let Some(basis) = Basis::of(secret_named(&name), flagged) else {
        return;
    };
    for f in fields.iter().filter(|f| is_field(&VALUE_FIELDS, &f.key)) {
        let (start, end) = trim_range(text, f.start, f.end);
        let value = &text[start..end];
        // A trailing `,` or `;` is a line of code (`value: compute(x),`).
        if !value.ends_with([',', ';']) && passes_floor(value) {
            out.push(TextPair {
                start,
                end,
                name: name.clone(),
                basis,
            });
        }
    }
}

/// Narrow a range to its trimmed, unquoted content.
fn trim_range(text: &str, mut start: usize, mut end: usize) -> (usize, usize) {
    let s = &text[start..end];
    let lead = s.len() - s.trim_start().len();
    let trail = s.len() - s.trim_end().len();
    start += lead;
    end -= trail;
    let s = &text[start..end];
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        start += 1;
        end -= 1;
    }
    (start, end)
}

static JSON_OBJECT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{[^{}]*\}").expect("flat object pattern"));
/// `"key": "string"` or `"key": true`, plain or escaped once (`\"key\":\"v\"`).
static JSON_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"\\?"(?P<k>[A-Za-z_][A-Za-z0-9_\-]*)\\?"\s*:\s*(?:"(?P<s>(?:[^"\\]|\\.)*)"|\\"(?P<e>(?:[^"\\]|\\[^"])*)\\"|(?P<b>(?i:true|false)))"#,
    )
    .expect("json field pattern")
});
static XML_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<[A-Za-z][\w:.\-]*\s[^<>]*>").expect("xml tag pattern"));
static XML_ATTR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?P<k>[A-Za-z_][\w:.\-]*)\s*=\s*\\?(?:"(?P<d>[^"\\]*)\\?"|'(?P<q>[^']*)')"#)
        .expect("xml attribute pattern")
});
static XML_CHILD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<(?P<k>[A-Za-z][\w:.\-]*)>(?P<v>[^<]*)</[A-Za-z][\w:.\-]*>")
        .expect("xml child pattern")
});
static YAML_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*(?:-[ \t]+)?(?P<k>[A-Za-z_][A-Za-z0-9_]*)[ \t]*:[ \t]+(?P<v>[^\r\n#]*?)[ \t]*\r?$")
        .expect("yaml field pattern")
});

/// Group fields into objects where the shape has no explicit object boundary
/// (XML children, YAML): each name field starts a new object, which runs to the
/// next name field. A value written before its name is not paired (v1).
fn judge_runs(text: &str, fields: Vec<Field>, out: &mut Vec<TextPair>) {
    let mut run: Vec<Field> = Vec::new();
    for f in fields {
        if is_field(&NAME_FIELDS, &f.key) {
            judge(text, &run, out);
            run.clear();
        }
        run.push(f);
    }
    judge(text, &run, out);
}

/// Every value in `text` that fires, from the four text shapes.
pub fn text_pairs(text: &str) -> Vec<TextPair> {
    let lower_has = |needles: &[&str]| {
        needles.iter().any(|n| {
            text.as_bytes()
                .windows(n.len())
                .any(|w| w.eq_ignore_ascii_case(n.as_bytes()))
        })
    };
    // Cheap prefilter: no value-like field name, nothing to pair.
    if !lower_has(&VALUE_FIELDS) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for obj in JSON_OBJECT.find_iter(text) {
        let fields: Vec<Field> = JSON_FIELD
            .captures_iter(obj.as_str())
            .filter_map(|c| {
                let v = c
                    .name("s")
                    .or_else(|| c.name("e"))
                    .or_else(|| c.name("b"))?;
                Some(Field {
                    key: c["k"].to_ascii_lowercase(),
                    start: obj.start() + v.start(),
                    end: obj.start() + v.end(),
                })
            })
            .collect();
        judge(text, &fields, &mut out);
    }
    for tag in XML_TAG.find_iter(text) {
        let fields: Vec<Field> = XML_ATTR
            .captures_iter(tag.as_str())
            .filter_map(|c| {
                let v = c.name("d").or_else(|| c.name("q"))?;
                Some(Field {
                    key: c["k"].to_ascii_lowercase(),
                    start: tag.start() + v.start(),
                    end: tag.start() + v.end(),
                })
            })
            .collect();
        judge(text, &fields, &mut out);
    }
    let children: Vec<Field> = XML_CHILD
        .captures_iter(text)
        .map(|c| {
            let v = c.name("v").expect("child value");
            Field {
                key: c["k"].to_ascii_lowercase(),
                start: v.start(),
                end: v.end(),
            }
        })
        .collect();
    judge_runs(text, children, &mut out);
    let yaml: Vec<Field> = YAML_FIELD
        .captures_iter(text)
        .map(|c| {
            let v = c.name("v").expect("yaml value");
            Field {
                key: c["k"].to_ascii_lowercase(),
                start: v.start(),
                end: v.end(),
            }
        })
        .collect();
    judge_runs(text, yaml, &mut out);
    out.sort_by_key(|p| (p.start, p.end));
    out.dedup_by_key(|p| (p.start, p.end));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redact::{redact, redact_json, scan, Rule};

    const MARK: &str = "[REDACTED:pair]";

    /// A fake value, built from parts so no fixture reads as a credential.
    fn fake(n: u8) -> String {
        format!(
            "{}{}-{}",
            "Hx7q",
            "2Lm9",
            char::from(b'a' + n).to_string().repeat(4)
        )
    }

    fn pairs(text: &str) -> Vec<(String, Option<&'static str>)> {
        scan(text)
            .into_iter()
            .filter(|f| f.rule == Rule::Pair)
            .map(|f| (f.key.unwrap_or_default(), f.pair_basis))
            .collect()
    }

    fn redacted(text: &str) -> String {
        redact(text, &[Rule::Pair]).0
    }

    #[test]
    fn planted_fakes_are_redacted() {
        let cases = [
            // 1. a JSON variable list with a secret-named Name
            (
                format!(
                    r#"[{{"Name": "DB_{}", "Value": "{}"}}, {{"Name": "Region", "Value": "westeurope-1"}}]"#,
                    "PASSWORD",
                    fake(1)
                ),
                fake(1),
                "name",
            ),
            // 2. XML Name= / Value= attributes on one element
            (
                format!(r#"<Variable Name="Smtp{}" Value="{}" />"#, "Pwd", fake(2)),
                fake(2),
                "name",
            ),
            // 3. XML child elements
            (
                format!(
                    "<Variable>\n  <Name>Deploy{}</Name>\n  <Value>{}</Value>\n</Variable>",
                    "Token",
                    fake(3)
                ),
                fake(3),
                "name",
            ),
            // 4. a sensitivity flag under a harmless name
            (
                format!(
                    r#"{{"Name": "BuildTarget", "Value": "{}", "Sensitive": true}}"#,
                    fake(4)
                ),
                fake(4),
                "flag",
            ),
            // YAML (Kubernetes env style)
            (
                format!("env:\n  - name: API_{}\n    value: {}\n", "AUTH", fake(6)),
                fake(6),
                "name",
            ),
            // escaped JSON inside JSON text
            (
                format!(
                    r#"{{\"key\":\"Conn{}\",\"value\":\"{}\"}}"#,
                    "String",
                    fake(7)
                ),
                fake(7),
                "name",
            ),
        ];
        for (text, value, basis) in cases {
            let out = redacted(&text);
            assert!(!out.contains(&value), "not redacted: {text}");
            assert!(out.contains(MARK), "{text}");
            let hits = pairs(&text);
            assert_eq!(hits.len(), 1, "{text}");
            assert_eq!(hits[0].1, Some(basis), "{text}");
        }
    }

    #[test]
    fn a_value_printed_twice_is_replaced_twice_and_counted_once() {
        let v = fake(5);
        let text = format!(
            r#"{{"Name": "SIGNING_{}", "Value": "{v}"}} and again later: {v}"#,
            "KEY"
        );
        let out = redacted(&text);
        assert!(!out.contains(&v));
        assert_eq!(out.matches(MARK).count(), 2);
        assert_eq!(pairs(&text).len(), 1);
    }

    #[test]
    fn negative_controls_are_untouched() {
        let controls = [
            format!(r#"<Setting Name="{}MinLength" Value="12" />"#, "Password"),
            format!(
                r#"<Setting Name="{}_RESET_URL" Value="https://portal.local/reset/start" />"#,
                "PASSWORD"
            ),
            r#"{"Name": "Description", "Value": "Deploys the nightly build to the staging pool"}"#
                .to_owned(),
            r#"{"Name": "Region", "Value": "westeurope-1"}"#.to_owned(),
            format!(r#"{{"Name": "DB_{}", "Value": "********"}}"#, "PASSWORD"),
            // token matching: PATH is not PAT, AUTHOR is not AUTH
            r#"{"Name": "PATH", "Value": "/usr/local/bin:/usr/bin"}"#.to_owned(),
            r#"{"Name": "AUTHOR", "Value": "Someone Example-Free"}"#.to_owned(),
        ];
        for text in controls {
            assert_eq!(redacted(&text), text, "changed: {text}");
            assert!(pairs(&text).is_empty(), "flagged: {text}");
        }
    }

    #[test]
    fn source_code_is_not_a_pair() {
        let rust = "        Task {\n            key: StoreKey::from(&task.key()),\n            value: serde_json::to_value(&task)?,\n        }";
        let go = format!("\tname: \"API_{}\",\n\tvalue: os.Getenv(\"X\"),", "TOKEN");
        for code in [rust.to_owned(), go] {
            assert!(pairs(&code).is_empty(), "flagged code: {code}");
        }
    }

    #[test]
    fn a_url_with_userinfo_still_fires() {
        let text = format!(
            r#"{{"Name": "DB_{}", "Value": "postgres://svc:{}@db.local/app"}}"#,
            "DSN",
            fake(8)
        );
        assert_eq!(pairs(&text).len(), 1);
    }

    #[test]
    fn structured_json_objects_are_judged() {
        let mut v = serde_json::json!({"variables": [
            {"name": format!("ACCESS_{}", "KEY"), "value": fake(9)},
            {"name": "Region", "value": "westeurope-1"},
            {"name": "Feature", "value": fake(10), "encrypted": "True"},
        ]});
        let hits = redact_json(&mut v, &[Rule::Pair]);
        let pair: Vec<_> = hits.iter().filter(|f| f.rule == Rule::Pair).collect();
        assert_eq!(pair.len(), 2);
        let text = v.to_string();
        assert!(!text.contains(&fake(9)) && !text.contains(&fake(10)));
        assert!(text.contains("westeurope-1"));
        assert_eq!(text.matches(MARK).count(), 2);
    }

    #[test]
    fn flag_only_mode_changes_nothing() {
        let text = format!(r#"<Variable Name="Smtp{}" Value="{}" />"#, "Pwd", fake(2));
        assert_eq!(redact(&text, &[]).0, text);
        assert_eq!(pairs(&text).len(), 1);
    }

    #[test]
    fn names_tokenise() {
        assert_eq!(tokens("DbConnectionString"), ["DB", "CONNECTION", "STRING"]);
        for named in [
            "SMTP_PWD",
            "gitlabPat",
            "ApiKey",
            "client_secret",
            "sas",
            "SigningKey",
            "LicenseKey",
        ] {
            assert!(secret_named(named), "{named}");
        }
        for plain in ["PATH", "Author", "Passage", "Region", "KEY", "Keyboard"] {
            assert!(!secret_named(plain), "{plain}");
        }
    }
}
