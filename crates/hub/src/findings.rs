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
use crate::redact::{self, Finding, Rule};
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
}

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
        let _ = writeln!(
            o,
            "\nHITS (location, rule, key, value shape — never the value)"
        );
        for h in &self.hits {
            let _ = writeln!(
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
        }
        o
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
            SELECT m.id, s.session_id, m.uuid, mc.hostname, m."timestamp",
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
            // `&[]`: detect only, never replace.
            let mut per_field: Vec<(&'static str, Vec<Finding>)> =
                vec![("raw", redact::redact_json(&mut raw, &[]))];
            if let Some(mut c) = content {
                per_field.push(("content", redact::redact_json(&mut c, &[])));
            }
            if let Some(t) = &search_text {
                per_field.push(("search_text", redact::scan(t)));
            }
            for (field, findings) in per_field {
                for f in findings.into_iter().filter(|f| rules.contains(&f.rule)) {
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
    report.expected = opts
        .expect
        .iter()
        .map(|e| (e.clone(), report.hits.iter().any(|h| e.matches(h))))
        .collect();
    Ok(report)
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
    fn verdict_requires_every_positive_control() {
        let mut r = DryRunReport::default();
        assert!(r.verdict().starts_with("UNVERIFIED"));
        r.expected = vec![(Expect::Row(1), true), (Expect::Row(2), false)];
        assert!(r.verdict().starts_with("COULD NOT LOOK"));
        r.expected[1].1 = true;
        assert!(r.verdict().starts_with("FLOOR"));
    }
}
