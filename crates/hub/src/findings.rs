//! Credential findings, read side (Gitea #34): `GET /v1/findings/summary` for
//! alerting, and the operator dry run over messages already stored, which is
//! also infra's sweep for `ac/infra#104`.
//!
//! Hard rule, as in `redact`: nothing here renders, logs or hashes a detected
//! value. The summary returns counts; the dry run reports a hit's location,
//! rule, secret-NAMED key and value SHAPE (length bucket, character classes).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

use crate::auth::Authenticated;
use crate::error::HubError;
use crate::redact::{self, Finding, Origin, Rule};
use crate::state::AppState;

/// `24h`, `7d`, `90m`, `30s` or bare seconds.
pub fn parse_since(s: &str) -> Option<i64> {
    let s = s.trim();
    let (num, unit) = match s.char_indices().last()? {
        (i, c) if c.is_ascii_alphabetic() => (&s[..i], c),
        _ => (s, 's'),
    };
    let n: i64 = num.parse().ok().filter(|n| *n > 0)?;
    let mult = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86_400,
        _ => return None,
    };
    n.checked_mul(mult)
}

#[derive(Debug, Deserialize)]
pub struct SummaryParams {
    /// Window, e.g. `24h` (the default).
    pub since: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FindingsSummary {
    pub since_secs: i64,
    /// Finding rows (message × field × rule) recorded in the window.
    pub total: i64,
    pub by_rule: BTreeMap<String, i64>,
    /// Distinct messages with a finding, per rule, not counting
    /// `api-request-blob` copies: the per-day count monitoring alerts on.
    #[serde(default)]
    pub messages_by_rule: BTreeMap<String, i64>,
    /// How many of those were stored redacted.
    pub redacted: i64,
    pub latest_detected_at: Option<DateTime<Utc>>,
    pub detector_version: u32,
}

fn since_secs(params: &SummaryParams) -> Result<i64, HubError> {
    match params.since.as_deref() {
        None => Ok(86_400),
        Some(s) => parse_since(s)
            .ok_or_else(|| HubError::BadRequest(format!("bad since {s:?}: use e.g. 24h, 7d"))),
    }
}

/// Finding counts by rule over the last `since_secs`. Shared by the summary and
/// the health check so the two can never disagree about a window.
async fn count_since(pool: &PgPool, since_secs: i64) -> Result<FindingsSummary, HubError> {
    let rows = sqlx::query(
        r"
        SELECT rule, count(*) AS n, count(*) FILTER (WHERE redacted) AS r, max(detected_at) AS latest
        FROM credential_findings
        WHERE detected_at > now() - make_interval(secs => $1)
        GROUP BY rule
        ",
    )
    .bind(since_secs as f64)
    .fetch_all(pool)
    .await?;

    let mut out = FindingsSummary {
        since_secs,
        total: 0,
        by_rule: BTreeMap::new(),
        messages_by_rule: BTreeMap::new(),
        redacted: 0,
        latest_detected_at: None,
        detector_version: redact::DETECTOR_VERSION,
    };
    for row in rows {
        let n: i64 = row.get("n");
        out.total += n;
        out.redacted += row.get::<i64, _>("r");
        out.by_rule.insert(row.get("rule"), n);
        let latest: Option<DateTime<Utc>> = row.get("latest");
        out.latest_detected_at = out.latest_detected_at.max(latest);
    }
    let messages = sqlx::query(
        r"
        SELECT f.rule, count(DISTINCT f.message_ref) AS n
        FROM credential_findings f
        JOIN messages m ON m.id = f.message_ref
        WHERE f.detected_at > now() - make_interval(secs => $1)
          AND m.type IS DISTINCT FROM 'api-request-blob'
        GROUP BY f.rule
        ",
    )
    .bind(since_secs as f64)
    .fetch_all(pool)
    .await?;
    for row in messages {
        out.messages_by_rule.insert(row.get("rule"), row.get("n"));
    }
    Ok(out)
}

/// `GET /v1/findings/summary?since=24h` — counts only, never message text.
pub async fn summary(
    _auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<SummaryParams>,
) -> Result<Json<FindingsSummary>, HubError> {
    Ok(Json(count_since(&state.pool, since_secs(&params)?).await?))
}

/// `GET /v1/healthz/findings?since=24h` — unauthenticated, like every
/// `/v1/healthz/*`, so Gatus can page on it (ac's Q-b, 2026-10-10). `200` while
/// no finding was recorded in the window, `503` once one was: a credential
/// reached a transcript and its owner should rotate it, whether or not the
/// archive stored it redacted (the plaintext still sits in the Mac-side file).
/// Same body as the summary: counts by rule, never a value, session or key name.
pub async fn healthz(
    State(state): State<AppState>,
    Query(params): Query<SummaryParams>,
) -> Result<(StatusCode, Json<FindingsHealth>), HubError> {
    let summary = count_since(&state.pool, since_secs(&params)?).await?;
    let (code, status) = if summary.total == 0 {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "findings")
    };
    Ok((code, Json(FindingsHealth { status, summary })))
}

#[derive(Debug, Serialize)]
pub struct FindingsHealth {
    /// `ok` · `findings` (at least one credential finding in the window).
    pub status: &'static str,
    #[serde(flatten)]
    pub summary: FindingsSummary,
}

// ---------------------------------------------------------------------------
// Dry run
// ---------------------------------------------------------------------------

/// A location a dry run must find a hit at before its result counts: a known
/// leak, given as the hub row id or as `<session-id>:<message-uuid>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expect {
    Row(i64),
    Uuid { session: String, uuid: String },
}

impl Expect {
    pub fn parse(s: &str) -> Option<Expect> {
        match s.split_once(':') {
            Some((session, uuid)) if !session.is_empty() && !uuid.is_empty() => {
                Some(Expect::Uuid {
                    session: session.to_string(),
                    uuid: uuid.to_string(),
                })
            }
            Some(_) => None,
            None => s.parse().ok().map(Expect::Row),
        }
    }

    fn label(&self) -> String {
        match self {
            Expect::Row(id) => format!("row {id}"),
            Expect::Uuid { session, uuid } => format!("{session}:{uuid}"),
        }
    }

    fn matches(&self, hit: &Hit) -> bool {
        match self {
            Expect::Row(id) => hit.row_id == *id,
            Expect::Uuid { session, uuid } => {
                hit.session_id == *session && hit.uuid.as_deref() == Some(uuid.as_str())
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DryRunOptions {
    /// Only messages whose timestamp is at or after this.
    pub since: Option<DateTime<Utc>>,
    /// Rules to report; empty = all.
    pub rules: Vec<Rule>,
    pub expect: Vec<Expect>,
    /// Messages per query.
    pub batch: i64,
    /// When set, each hit also carries a keyed fingerprint of its value: the
    /// first 12 hex of HMAC-SHA256(key, value). Never the value or a plain hash,
    /// which a guessed passphrase could be tested against offline.
    pub fingerprint_key: Option<Vec<u8>>,
}

/// HMAC-SHA256 (RFC 2104) over `sha2`, so the fingerprint needs no new crate.
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut block = [0_u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let ipad: Vec<u8> = block.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = block.iter().map(|b| b ^ 0x5c).collect();
    let inner = Sha256::new()
        .chain_update(&ipad)
        .chain_update(msg)
        .finalize();
    Sha256::new()
        .chain_update(&opad)
        .chain_update(inner)
        .finalize()
        .into()
}

/// The first 12 hex of HMAC-SHA256(key, value).
pub fn fingerprint(key: &[u8], value: &str) -> String {
    let mut hex = String::with_capacity(12);
    for b in &hmac_sha256(key, value.as_bytes())[..6] {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

/// What the fingerprint path learns about a value before dropping it.
#[derive(Debug, Clone)]
struct Keyed {
    fingerprint: String,
    reference: bool,
    placeholder: bool,
    origin: Origin,
}

/// One field's detections, each with its [`Keyed`] facts when asked for.
type FieldHits = (&'static str, Vec<(Finding, Option<Keyed>)>);

/// One detection, with everything a rotation needs and no value.
#[derive(Debug, Clone)]
pub struct Hit {
    pub row_id: i64,
    pub session_id: String,
    pub uuid: Option<String>,
    pub machine: String,
    pub timestamp: Option<DateTime<Utc>>,
    pub field: &'static str,
    pub rule: Rule,
    pub key: Option<String>,
    pub value_len_bucket: &'static str,
    pub value_classes: u8,
    /// Keyed fingerprint, only with `DryRunOptions::fingerprint_key`; so are
    /// the fields below it.
    pub fingerprint: Option<String>,
    /// The value is an unexpanded reference (`${VAR}`, `$(cmd)`, …).
    pub reference: bool,
    /// The value contains a placeholder word or shape (`xxx`, `<token>`, …).
    pub placeholder: bool,
    /// `tool_result`, `tool_input`, `text`, `other` or `search_text`.
    pub source: &'static str,
    /// For a tool result: the basename of the file the producing call read
    /// (a `Read`, or `cat`/`grep`/`sed` in a `Bash` command). Never a path.
    pub file: Option<String>,
    /// `sessions.id` and the tool call id, to resolve `file` after the scan.
    session_ref: i64,
    tool_use_id: Option<String>,
}

#[derive(Debug, Default)]
pub struct DryRunReport {
    pub rules: Vec<Rule>,
    pub scanned: u64,
    pub id_range: Option<(i64, i64)>,
    pub time_range: (Option<DateTime<Utc>>, Option<DateTime<Utc>>),
    pub since: Option<DateTime<Utc>>,
    pub machines: BTreeSet<String>,
    pub hits: Vec<Hit>,
    /// Each `--expect`, and whether a hit landed on it.
    pub expected: Vec<(Expect, bool)>,
}

/// What no dry run can see, stated on every report (infra's condition 3).
pub const NOT_REACHED: &[&str] = &[
    "Mac-side transcript files that were never ingested (outside the archive)",
    "messages lost to ingest gaps (a daemon that was down, excluded, or failing a session)",
    "rows outside --since",
    "shapes no rule covers: a secret with no secret-named key, no known prefix, no PEM block",
];

impl DryRunReport {
    pub fn verdict(&self) -> &'static str {
        if self.expected.is_empty() {
            "UNVERIFIED: no --expect given, so the hit count is a floor of unknown sensitivity"
        } else if self.expected.iter().any(|(_, found)| !found) {
            "COULD NOT LOOK: a known leak was not found, so this result says nothing about the archive"
        } else {
            "FLOOR: every known leak was found; the hits below are a lower bound, not an all-clear"
        }
    }

    /// Plain-text report. Contains no detected value by construction: it is
    /// built only from `Hit` fields, none of which holds one.
    pub fn render(&self) -> String {
        let mut o = String::new();
        let rules: Vec<&str> = self.rules.iter().map(Rule::id).collect();
        let _ = writeln!(
            o,
            "credential dry run — detector v{}",
            redact::DETECTOR_VERSION
        );
        let _ = writeln!(o, "VERDICT: {}", self.verdict());
        let _ = writeln!(o, "\nREACH (what was scanned)");
        let _ = writeln!(o, "  messages scanned: {}", self.scanned);
        if let Some((a, b)) = self.id_range {
            let _ = writeln!(o, "  row ids: {a}..={b}");
        }
        let fmt_ts = |t: Option<DateTime<Utc>>| t.map_or("-".to_string(), |t| t.to_rfc3339());
        let _ = writeln!(
            o,
            "  message time range: {} .. {}",
            fmt_ts(self.time_range.0),
            fmt_ts(self.time_range.1)
        );
        let _ = writeln!(o, "  --since: {}", fmt_ts(self.since));
        let machines: Vec<&str> = self.machines.iter().map(String::as_str).collect();
        let _ = writeln!(o, "  machines: {}", machines.join(", "));
        let _ = writeln!(o, "  fields: raw, content, search_text");
        let _ = writeln!(o, "  rules: {}", rules.join(", "));
        let _ = writeln!(o, "NOT REACHED");
        for line in NOT_REACHED {
            let _ = writeln!(o, "  - {line}");
        }
        let _ = writeln!(o, "\nPOSITIVE CONTROLS (--expect)");
        if self.expected.is_empty() {
            let _ = writeln!(o, "  none given");
        }
        for (e, found) in &self.expected {
            let _ = writeln!(
                o,
                "  {}: {}",
                e.label(),
                if *found { "FOUND" } else { "MISSED" }
            );
        }
        let mut by_rule: BTreeMap<&str, usize> = BTreeMap::new();
        let mut by_key: BTreeMap<&str, usize> = BTreeMap::new();
        for h in &self.hits {
            *by_rule.entry(h.rule.id()).or_default() += 1;
            if let Some(k) = &h.key {
                *by_key.entry(k.as_str()).or_default() += 1;
            }
        }
        let _ = writeln!(o, "\nTOTALS ({} hits)", self.hits.len());
        for (r, n) in &by_rule {
            let _ = writeln!(o, "  rule {r}: {n}");
        }
        for (k, n) in &by_key {
            let _ = writeln!(o, "  key {k}: {n}");
        }
        if self.hits.iter().any(|h| h.fingerprint.is_some()) {
            let mut flags: BTreeMap<&str, [usize; 3]> = BTreeMap::new();
            for h in &self.hits {
                let row = flags.entry(h.rule.id()).or_default();
                row[0] += usize::from(h.reference);
                row[1] += usize::from(h.placeholder);
                row[2] += usize::from(!h.reference && !h.placeholder);
            }
            for (r, [refs, phs, real]) in &flags {
                let _ = writeln!(
                    o,
                    "  rule {r}: reference {refs}, placeholder {phs}, neither {real}"
                );
            }
        }
        let _ = writeln!(
            o,
            "\nHITS (location, rule, key, value shape — never the value)"
        );
        for h in &self.hits {
            let _ = write!(
                o,
                "  row={} session={} message={} machine={} at={} field={} rule={} key={} shape={}/{}",
                h.row_id,
                h.session_id,
                h.uuid.as_deref().unwrap_or("-"),
                h.machine,
                fmt_ts(h.timestamp),
                h.field,
                h.rule.id(),
                h.key.as_deref().unwrap_or("-"),
                h.value_len_bucket,
                classes(h.value_classes),
            );
            match &h.fingerprint {
                Some(fp) => {
                    let _ = writeln!(
                        o,
                        " fp={fp} ref={} ph={} src={} file={}",
                        yn(h.reference),
                        yn(h.placeholder),
                        h.source,
                        h.file.as_deref().unwrap_or("-"),
                    );
                }
                None => {
                    let _ = writeln!(o);
                }
            }
        }
        o
    }
}

fn yn(b: bool) -> &'static str {
    if b {
        "y"
    } else {
        "n"
    }
}

fn classes(bits: u8) -> String {
    let names = [(1, "lower"), (2, "upper"), (4, "digit"), (8, "symbol")];
    let v: Vec<&str> = names
        .iter()
        .filter(|(b, _)| bits & b != 0)
        .map(|(_, n)| *n)
        .collect();
    v.join("+")
}

/// Scan stored messages in id order with the ingest detector, changing nothing.
/// Callers should hand it a pool whose sessions are read-only.
pub async fn dry_run(pool: &PgPool, opts: &DryRunOptions) -> anyhow::Result<DryRunReport> {
    let rules: Vec<Rule> = if opts.rules.is_empty() {
        Rule::ALL.to_vec()
    } else {
        opts.rules.clone()
    };
    let mut report = DryRunReport {
        rules: rules.clone(),
        since: opts.since,
        ..DryRunReport::default()
    };
    let batch = opts.batch.max(1);
    let mut last_id = 0_i64;
    loop {
        let rows = sqlx::query(
            r#"
            SELECT m.id, m.session_id AS session_ref, s.session_id, m.uuid, mc.hostname, m."timestamp",
                   m.raw, m.content, m.search_text
            FROM messages m
            JOIN sessions s ON s.id = m.session_id
            JOIN machines mc ON mc.machine_id = m.machine_id
            WHERE m.id > $1 AND ($2::timestamptz IS NULL OR m."timestamp" >= $2)
            ORDER BY m.id
            LIMIT $3
            "#,
        )
        .bind(last_id)
        .bind(opts.since)
        .bind(batch)
        .fetch_all(pool)
        .await?;
        if rows.is_empty() {
            break;
        }
        for row in &rows {
            let id: i64 = row.get("id");
            last_id = id;
            report.scanned += 1;
            report.id_range = Some(report.id_range.map_or((id, id), |(a, _)| (a, id)));
            let ts: Option<DateTime<Utc>> = row.get("timestamp");
            if let Some(t) = ts {
                let (lo, hi) = report.time_range;
                report.time_range = (
                    Some(lo.map_or(t, |l| l.min(t))),
                    Some(hi.map_or(t, |h| h.max(t))),
                );
            }
            let machine: String = row.get("hostname");
            report.machines.insert(machine.clone());

            let mut raw: serde_json::Value = row.get("raw");
            let content: Option<serde_json::Value> = row.get("content");
            let search_text: Option<String> = row.get("search_text");
            let key = opts.fingerprint_key.as_deref();
            let keyed = |hits: Vec<(Finding, String, Origin)>| -> Vec<(Finding, Option<Keyed>)> {
                hits.into_iter()
                    .map(|(f, value, origin)| {
                        let facts = key.map(|k| Keyed {
                            fingerprint: fingerprint(k, &value),
                            reference: redact::is_reference(&value),
                            placeholder: redact::is_placeholder(&value),
                            origin,
                        });
                        (f, facts)
                    })
                    .collect()
            };
            let plain = |hits: Vec<Finding>| -> Vec<(Finding, Option<Keyed>)> {
                hits.into_iter().map(|f| (f, None)).collect()
            };
            // `&[]`: detect only, never replace.
            let mut per_field: Vec<FieldHits> = Vec::new();
            if key.is_some() {
                per_field.push(("raw", keyed(redact::json_values(&raw))));
                if let Some(c) = &content {
                    per_field.push(("content", keyed(redact::json_values(c))));
                }
                if let Some(t) = &search_text {
                    let at = Origin {
                        kind: "search_text",
                        tool_use_id: None,
                    };
                    let hits = redact::scan_values(t)
                        .into_iter()
                        .map(|(f, v)| (f, v.to_owned(), at.clone()))
                        .collect();
                    per_field.push(("search_text", keyed(hits)));
                }
            } else {
                per_field.push(("raw", plain(redact::redact_json(&mut raw, &[]))));
                if let Some(mut c) = content {
                    per_field.push(("content", plain(redact::redact_json(&mut c, &[]))));
                }
                if let Some(t) = &search_text {
                    per_field.push(("search_text", plain(redact::scan(t))));
                }
            }
            for (field, findings) in per_field {
                for (f, facts) in findings
                    .into_iter()
                    .filter(|(f, _)| f.rule.selected(&rules))
                {
                    let facts = facts.as_ref();
                    report.hits.push(Hit {
                        row_id: id,
                        session_id: row.get("session_id"),
                        uuid: row.get("uuid"),
                        machine: machine.clone(),
                        timestamp: ts,
                        field,
                        rule: f.rule,
                        key: f.key,
                        value_len_bucket: f.value_len_bucket,
                        value_classes: f.value_classes,
                        fingerprint: facts.map(|k| k.fingerprint.clone()),
                        reference: facts.is_some_and(|k| k.reference),
                        placeholder: facts.is_some_and(|k| k.placeholder),
                        source: facts.map_or("-", |k| k.origin.kind),
                        file: None,
                        session_ref: row.get("session_ref"),
                        tool_use_id: facts.and_then(|k| k.origin.tool_use_id.clone()),
                    });
                }
            }
        }
        tracing::info!(
            scanned = report.scanned,
            last_id,
            hits = report.hits.len(),
            "dry run progress"
        );
    }
    if opts.fingerprint_key.is_some() {
        resolve_files(pool, &mut report.hits).await?;
    }
    report.expected = opts
        .expect
        .iter()
        .map(|e| (e.clone(), report.hits.iter().any(|h| e.matches(h))))
        .collect();
    Ok(report)
}

/// Fill `Hit::file` for tool results: find each producing `tool_use` in the
/// same session (one query per session) and keep only a file basename.
async fn resolve_files(pool: &PgPool, hits: &mut [Hit]) -> anyhow::Result<()> {
    let mut wanted: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
    for h in hits.iter().filter(|h| h.source == "tool_result") {
        if let Some(id) = &h.tool_use_id {
            wanted.entry(h.session_ref).or_default().insert(id.clone());
        }
    }
    let mut files: BTreeMap<(i64, String), String> = BTreeMap::new();
    for (session_ref, ids) in wanted {
        let ids: Vec<String> = ids.into_iter().collect();
        let rows = sqlx::query(
            r"
            SELECT b->>'id' AS id, b->>'name' AS name, b->'input' AS input
            FROM messages m,
                 jsonb_array_elements(CASE WHEN jsonb_typeof(m.raw->'message'->'content') = 'array'
                                           THEN m.raw->'message'->'content' ELSE '[]'::jsonb END) b
            WHERE m.session_id = $1 AND b->>'type' = 'tool_use' AND b->>'id' = ANY($2)
            ",
        )
        .bind(session_ref)
        .bind(&ids)
        .fetch_all(pool)
        .await?;
        for row in rows {
            let id: Option<String> = row.get("id");
            let name: Option<String> = row.get("name");
            let input: Option<serde_json::Value> = row.get("input");
            if let (Some(id), Some(name), Some(input)) = (id, name, input) {
                if let Some(file) = read_basename(&name, &input) {
                    files.insert((session_ref, id), file);
                }
            }
        }
    }
    for h in hits.iter_mut() {
        if let Some(id) = &h.tool_use_id {
            h.file = files.get(&(h.session_ref, id.clone())).cloned();
        }
    }
    Ok(())
}

/// The basename of the file a tool call read: a `Read`'s `file_path`, or the
/// last operand of the first `cat`/`grep`/`sed` in a `Bash` command.
pub fn read_basename(tool: &str, input: &serde_json::Value) -> Option<String> {
    let path = match tool {
        "Read" => input.get("file_path")?.as_str()?.to_owned(),
        "Bash" => command_file(input.get("command")?.as_str()?)?,
        _ => return None,
    };
    let base = path.rsplit(['/', '\\']).next()?.trim();
    (!base.is_empty()).then(|| base.to_owned())
}

fn command_file(command: &str) -> Option<String> {
    for segment in command.split(['|', ';', '&', '\n']) {
        let mut words = segment.split_whitespace();
        let Some(program) = words.next() else {
            continue;
        };
        if !matches!(program.rsplit('/').next(), Some("cat" | "grep" | "sed")) {
            continue;
        }
        let operands: Vec<&str> = words
            .take_while(|w| !w.starts_with('>') && !w.starts_with('<') && !w.starts_with("2>"))
            .filter(|w| !w.starts_with('-'))
            .collect();
        // grep and sed take a pattern or script before the files.
        let min = if program.ends_with("cat") { 1 } else { 2 };
        if operands.len() >= min {
            let last = operands.last()?;
            return Some(last.trim_matches(|c| c == '"' || c == '\'').to_owned());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Real run (task 3.3)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct RedactOptions {
    /// The dry run this real run executes. Its rules must be non-empty: a real
    /// run names what it replaces.
    pub scan: DryRunOptions,
    /// Refuse when the dry run lists more rows than this. The operator states
    /// the count they read from their own dry run, so a run that has grown
    /// since (a new rule, a wider `--since`) stops before writing.
    pub max_rows: i64,
}

#[derive(Debug, Default)]
pub struct RedactReport {
    /// The dry run the rewrite executed, as it would print on its own.
    pub dry_run: DryRunReport,
    /// Row ids rewritten, ascending.
    pub rewritten: Vec<i64>,
    /// Rewritten rows whose tool rows changed and were re-derived.
    pub tool_rows_rederived: Vec<i64>,
    /// Chosen-rule hits a rescan of the rewritten rows still finds (expect 0).
    pub after_hits: usize,
}

impl RedactReport {
    /// Contains no detected value: the dry run's report plus row ids and counts.
    pub fn render(&self) -> String {
        let mut o = self.dry_run.render();
        let _ = writeln!(o, "\nREAL RUN");
        let _ = writeln!(o, "  rows rewritten: {}", self.rewritten.len());
        let ids: Vec<String> = self.rewritten.iter().map(i64::to_string).collect();
        let _ = writeln!(o, "  row ids: {}", ids.join(" "));
        let _ = writeln!(
            o,
            "  tool rows re-derived: {}",
            self.tool_rows_rederived.len()
        );
        let _ = writeln!(
            o,
            "  journal days re-dirtied: each rewritten row's session-day (ingest_xid restamped)"
        );
        let _ = writeln!(
            o,
            "  rescan of rewritten rows, chosen rules: {} hits",
            self.after_hits
        );
        o
    }
}

/// `(row, field, rule) → hits` for the chosen rules, from detector output.
type HitCounts = BTreeMap<(i64, &'static str, &'static str), u32>;

/// Rewrite the rows a dry run lists, replacing the chosen rules' values with
/// the ingest marker, and nothing else.
///
/// Parity with the dry run is by construction and then checked: the rows come
/// from [`dry_run`] itself, each row is re-read under `FOR UPDATE` and its
/// chosen-rule hits must equal the dry run's for that row field by field, or
/// the row's transaction rolls back and the run stops. Every guard that can
/// refuse (no rule, a missed `--expect`, more rows than `max_rows`) refuses
/// before the first write.
///
/// Per rewritten row, in one transaction: `raw`/`content`/`search_text`
/// rewritten (`text_search` is generated from them); `message_id` re-derived;
/// tool rows re-derived only when the redacted payload extracts differently;
/// findings upserted as `redacted`; `ingest_xid` restamped so the row's journal
/// day reads dirty and is re-distilled. The stats mirror holds no message text
/// and keys tool rows above its watermark, so it needs no rebuild.
pub async fn redact_existing(pool: &PgPool, opts: &RedactOptions) -> anyhow::Result<RedactReport> {
    let rules = opts.scan.rules.clone();
    anyhow::ensure!(!rules.is_empty(), "a real run needs at least one --rule");
    let report = dry_run(pool, &opts.scan).await?;
    anyhow::ensure!(
        !report.expected.iter().any(|(_, found)| !found),
        "{} — nothing written",
        report.verdict()
    );
    let mut expected: HitCounts = BTreeMap::new();
    for h in &report.hits {
        *expected
            .entry((h.row_id, h.field, h.rule.id()))
            .or_default() += 1;
    }
    let targets: BTreeSet<i64> = report.hits.iter().map(|h| h.row_id).collect();
    let n_rows = i64::try_from(targets.len()).unwrap_or(i64::MAX);
    anyhow::ensure!(
        n_rows <= opts.max_rows,
        "the dry run lists {n_rows} rows, above --max-rows {} — nothing written",
        opts.max_rows
    );

    let mut out = RedactReport {
        dry_run: report,
        ..RedactReport::default()
    };
    for &id in &targets {
        if rewrite_row(pool, id, &rules, &expected).await? {
            out.tool_rows_rederived.push(id);
        }
        out.rewritten.push(id);
        tracing::info!(
            row = id,
            done = out.rewritten.len(),
            of = targets.len(),
            "redacted"
        );
    }

    // Rescan the rewritten rows only: a full rescan would repeat the whole scan,
    // and the operator's own dry run afterwards is the archive-wide check.
    for &id in &targets {
        let row = sqlx::query("SELECT raw, content, search_text FROM messages WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await?;
        let mut raw: serde_json::Value = row.get("raw");
        let mut content: Option<serde_json::Value> = row.get("content");
        let mut search_text: Option<String> = row.get("search_text");
        let hits = redact::apply_to_message(&mut raw, content.as_mut(), search_text.as_mut(), &[]);
        out.after_hits += hits
            .iter()
            .filter(|h| h.rule.selected(&rules))
            .map(|h| h.hits as usize)
            .sum::<usize>();
    }
    Ok(out)
}

/// Rewrite one row in its own transaction. Returns whether its tool rows were
/// re-derived. Errors (and rolls back) when the row's chosen-rule hits no
/// longer match what the dry run counted.
async fn rewrite_row(
    pool: &PgPool,
    id: i64,
    rules: &[Rule],
    expected: &HitCounts,
) -> anyhow::Result<bool> {
    use crate::extract;

    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "SELECT session_id, type, raw, content, search_text FROM messages WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    let session_pk: i64 = row.get("session_id");
    let message_type: Option<String> = row.get("type");
    let mut raw: serde_json::Value = row.get("raw");
    let mut content: Option<serde_json::Value> = row.get("content");
    let mut search_text: Option<String> = row.get("search_text");
    let old_uses = extract::tool_uses(message_type.as_deref(), content.as_ref(), &raw);
    let old_results = extract::tool_results(content.as_ref());

    let hits = redact::apply_to_message(&mut raw, content.as_mut(), search_text.as_mut(), rules);
    let found: HitCounts = hits
        .iter()
        .filter(|h| h.rule.selected(rules))
        .map(|h| ((id, h.field, h.rule.id()), h.hits))
        .collect();
    let listed: HitCounts = expected
        .range((id, "", "")..)
        .take_while(|((row, ..), _)| *row == id)
        .map(|(k, v)| (*k, *v))
        .collect();
    anyhow::ensure!(
        found == listed,
        "row {id}: hits differ from the dry run's (row changed since the scan?) — rolled back"
    );

    sqlx::query(
        r"
        UPDATE messages
        SET raw = $2, content = $3, search_text = $4, message_id = $5,
            ingest_xid = pg_current_xact_id()
        WHERE id = $1
        ",
    )
    .bind(id)
    .bind(&raw)
    .bind(&content)
    .bind(&search_text)
    .bind(extract::message_id(&raw))
    .execute(&mut *tx)
    .await?;

    for h in hits.iter().filter(|h| h.rule.selected(rules)) {
        sqlx::query(
            r"
            INSERT INTO credential_findings
                (message_ref, field, rule, hits, key_names, redacted, detector_version)
            VALUES ($1, $2, $3, $4, $5, true, $6)
            ON CONFLICT (message_ref, field, rule, detector_version)
                DO UPDATE SET redacted = true
            ",
        )
        .bind(id)
        .bind(h.field)
        .bind(h.rule.id())
        .bind(i32::try_from(h.hits).unwrap_or(i32::MAX))
        .bind(&h.key_names)
        .bind(i32::try_from(redact::DETECTOR_VERSION).unwrap_or(i32::MAX))
        .execute(&mut *tx)
        .await?;
    }

    let new_uses = extract::tool_uses(message_type.as_deref(), content.as_ref(), &raw);
    let new_results = extract::tool_results(content.as_ref());
    let rederive = new_uses != old_uses || new_results != old_results;
    if rederive {
        sqlx::query("DELETE FROM message_tool_uses WHERE message_ref = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM message_tool_results WHERE message_ref = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        for u in &new_uses {
            sqlx::query(
                r"
                INSERT INTO message_tool_uses
                    (message_ref, session_id, seq, tool_name, tool_use_id, skill_name,
                     subagent_type, is_error)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                ",
            )
            .bind(id)
            .bind(session_pk)
            .bind(u.seq)
            .bind(&u.tool_name)
            .bind(&u.tool_use_id)
            .bind(&u.skill_name)
            .bind(&u.subagent_type)
            .bind(u.is_error)
            .execute(&mut *tx)
            .await?;
        }
        for r in &new_results {
            sqlx::query(
                r"
                INSERT INTO message_tool_results
                    (message_ref, session_id, seq, tool_use_id, is_error)
                VALUES ($1, $2, $3, $4, $5)
                ",
            )
            .bind(id)
            .bind(session_pk)
            .bind(r.seq)
            .bind(&r.tool_use_id)
            .bind(r.is_error)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(rederive)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_parses_units_and_rejects_nonsense() {
        assert_eq!(parse_since("24h"), Some(86_400));
        assert_eq!(parse_since("7d"), Some(604_800));
        assert_eq!(parse_since("90m"), Some(5_400));
        assert_eq!(parse_since("3600"), Some(3_600));
        assert_eq!(parse_since("0h"), None);
        assert_eq!(parse_since("2w"), None);
        assert_eq!(parse_since("h"), None);
    }

    #[test]
    fn expect_parses_row_ids_and_session_uuid_pairs() {
        assert_eq!(Expect::parse("123"), Some(Expect::Row(123)));
        assert_eq!(
            Expect::parse("sess-1:u-9"),
            Some(Expect::Uuid {
                session: "sess-1".into(),
                uuid: "u-9".into()
            })
        );
        assert_eq!(Expect::parse("sess-1:"), None);
        assert_eq!(Expect::parse("abc"), None);
    }

    #[test]
    fn read_basename_keeps_only_the_file_name() {
        let read = serde_json::json!({"file_path": "/srv/app/config/app.env"});
        assert_eq!(read_basename("Read", &read).as_deref(), Some("app.env"));
        let cases = [
            ("cat /etc/svc/secrets.yaml | head", Some("secrets.yaml")),
            ("cd /x && grep -n PASSWORD conf/db.ini", Some("db.ini")),
            ("sed -n '1,20p' ./deploy/.env", Some(".env")),
            ("cat -A notes.txt > /tmp/out", Some("notes.txt")),
            ("grep -rn TOKEN", None),
            ("ls -la /etc", None),
        ];
        for (command, want) in cases {
            let input = serde_json::json!({ "command": command });
            assert_eq!(read_basename("Bash", &input).as_deref(), want, "{command}");
        }
        assert_eq!(read_basename("Write", &read), None);
    }

    #[test]
    fn verdict_requires_every_positive_control() {
        let mut r = DryRunReport::default();
        assert!(r.verdict().starts_with("UNVERIFIED"));
        r.expected = vec![(Expect::Row(1), true), (Expect::Row(2), false)];
        assert!(r.verdict().starts_with("COULD NOT LOOK"));
        r.expected[1].1 = true;
        assert!(r.verdict().starts_with("FLOOR"));
    }
}
