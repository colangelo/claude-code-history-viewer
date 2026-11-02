//! The client-scope sweep (openspec `hub-client-read-scope`, R2-B, R2-C, task 2.4):
//! classify every session not scanned under the current generation, from its
//! full stored form, and stamp it only when no ingest touched it meanwhile.
//!
//! Writes only raise a class; the generation stamp is conditional on the
//! `client_rev` the sweep read, so a crash, a resume or two sweeps at once
//! cannot lower a flag or stamp a session past content it did not see.

use std::collections::{BTreeMap, HashSet};

use serde_json::Value;
use sqlx::{PgPool, Row};

use crate::client_ingest::{base, parent_from_path, FAMILY};
use crate::client_policy::Policy;
use crate::client_scope::{
    classify, normalize, Class, MessageFields, ProjectFields, SessionFields,
};

pub struct SweepOptions {
    pub dry_run: bool,
    pub batch: i64,
    /// Restrict to these session ids (tests, and a targeted re-scan).
    pub only_sessions: Option<Vec<i64>>,
    /// Also read sessions already stamped lab at the current generation: the
    /// extractor changed (E1) while the routing files did not. Raise-only, like
    /// every sweep; a client session is never read again.
    pub rescan: bool,
}

/// Counts only: never a path, a name or a text.
#[derive(Debug, Default)]
pub struct SweepReport {
    pub scanned: u64,
    pub by_class: BTreeMap<String, u64>,
    /// Sessions the hub leaves lab that the distiller's content terms would
    /// protect (Q1: counted, never used for the flag).
    pub term_only: u64,
    /// Sessions turned client only because a protected journal entry names them.
    pub family_protected_entry: u64,
    /// Sessions turned client only through another family member.
    pub family_other: u64,
    /// Sessions classified but not stamped: an ingest touched them meanwhile.
    pub not_stamped: u64,
}

const PAGE: i64 = 500;

/// Classify one session from its row and every stored message, page by page.
/// Returns the class and whether the content terms hit anywhere.
async fn classify_session(
    pool: &PgPool,
    policy: &Policy,
    id: i64,
) -> anyhow::Result<(Class, bool)> {
    let row = sqlx::query(
        "SELECT p.project_path, s.summary, s.file_path, s.entrypoint,
                p.name, p.git_remote_url, p.git_main_path
         FROM sessions s LEFT JOIN projects p ON p.id = s.project_id WHERE s.id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    let project: Option<String> = row.try_get(0)?;
    let summary: Option<String> = row.try_get(1)?;
    let file_path: Option<String> = row.try_get(2)?;
    let entrypoint: Option<String> = row.try_get(3)?;
    let project_name: Option<String> = row.try_get(4)?;
    let git_remote_url: Option<String> = row.try_get(5)?;
    let git_main_path: Option<String> = row.try_get(6)?;
    let fields = SessionFields {
        project_path: project.as_deref().unwrap_or(""),
        summary: summary.as_deref(),
        file_path: file_path.as_deref(),
        entrypoint: entrypoint.as_deref(),
        project: ProjectFields {
            name: project_name.as_deref(),
            git_remote_url: git_remote_url.as_deref(),
            git_main_path: git_main_path.as_deref(),
        },
    };
    let mut best = classify(&policy.markers, fields, std::iter::empty());
    let mut term = false;
    let mut after = 0_i64;
    let null = Value::Null;
    loop {
        let rows: Vec<(i64, Option<Value>, Value, Option<String>)> = sqlx::query_as(
            "SELECT id, content, raw, search_text FROM messages
             WHERE session_id = $1 AND id > $2 ORDER BY id LIMIT $3",
        )
        .bind(id)
        .bind(after)
        .bind(PAGE)
        .fetch_all(pool)
        .await?;
        let Some(last) = rows.last() else { break };
        after = last.0;
        let page = classify(
            &policy.markers,
            SessionFields::default(),
            rows.iter().map(|(_, c, r, st)| MessageFields {
                content: c.as_ref().unwrap_or(&null),
                raw: r,
                search_text: st.as_deref(),
            }),
        );
        if page != Class::None && (best == Class::None || page.rank() < best.rank()) {
            best = page;
        }
        if !term && !policy.terms.is_empty() {
            term = rows.iter().any(|(_, c, _, _)| {
                let t = normalize(&c.as_ref().unwrap_or(&null).to_string());
                policy.terms.iter().any(|w| t.contains(w.as_str()))
            });
        }
    }
    Ok((best, term))
}

/// Raise the class, and stamp the generation only if `client_rev` is still
/// `seen_rev`. Returns whether it stamped.
pub async fn stamp(
    pool: &PgPool,
    id: i64,
    class: &str,
    generation: &str,
    seen_rev: i64,
) -> anyhow::Result<bool> {
    let stamped: bool = sqlx::query_scalar(
        "UPDATE sessions SET
            client_class = CASE
                WHEN $2 <> 'none' AND (client_class IS NULL OR client_class = 'none') THEN $2
                WHEN client_class IS NULL THEN $2
                ELSE client_class END,
            client_gen = CASE WHEN client_rev = $4 THEN $3 ELSE client_gen END
         WHERE id = $1
         RETURNING client_gen IS NOT DISTINCT FROM $3",
    )
    .bind(id)
    .bind(class)
    .bind(generation)
    .bind(seen_rev)
    .fetch_one(pool)
    .await?;
    Ok(stamped)
}

pub async fn sweep(
    pool: &PgPool,
    policy: &Policy,
    opts: &SweepOptions,
) -> anyhow::Result<SweepReport> {
    let mut report = SweepReport::default();
    let mut client: HashSet<i64> = HashSet::new();
    let mut after = 0_i64;
    loop {
        let rows: Vec<(i64, i64, String, String, Option<String>)> = sqlx::query_as(
            "SELECT id, client_rev, provider, session_id, file_path FROM sessions
             WHERE id > $1
               AND (client_gen IS DISTINCT FROM $2 OR ($5 AND client_class = 'none'))
               AND ($3::bigint[] IS NULL OR id = ANY($3))
             ORDER BY id LIMIT $4",
        )
        .bind(after)
        .bind(&policy.generation)
        .bind(opts.only_sessions.as_deref())
        .bind(opts.batch.max(1))
        .bind(opts.rescan)
        .fetch_all(pool)
        .await?;
        let Some(last) = rows.last() else { break };
        after = last.0;
        for (id, rev, provider, sid, file_path) in rows {
            let (class, term) = classify_session(pool, policy, id).await?;
            report.scanned += 1;
            *report
                .by_class
                .entry(class.as_str().to_string())
                .or_default() += 1;
            if class == Class::None && term {
                report.term_only += 1;
            }
            if class != Class::None {
                client.insert(id);
            }
            if opts.dry_run {
                continue;
            }
            if !stamp(pool, id, class.as_str(), &policy.generation, rev).await? {
                report.not_stamped += 1;
            }
            if class != Class::None {
                let parent = file_path
                    .as_deref()
                    .and_then(parent_from_path)
                    .unwrap_or("");
                let n = sqlx::query(&format!(
                    "UPDATE sessions SET client_class = 'family:' || $5, client_rev = client_rev + 1
                     WHERE {FAMILY} AND (client_class IS NULL OR client_class = 'none')"
                ))
                .bind(&provider)
                .bind(&sid)
                .bind(parent)
                .bind(id)
                .bind(base(class.as_str()))
                .execute(pool)
                .await?
                .rows_affected();
                report.family_other += n;
            }
        }
    }

    // The sessions of a protected journal entry are one family (R2-B).
    let named: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT s.id FROM journal_entries j JOIN sessions s ON s.id = ANY(j.session_ids)
         WHERE j.route = 'protected'
           AND (s.client_class IS NULL OR s.client_class = 'none')
           AND ($1::bigint[] IS NULL OR s.id = ANY($1))",
    )
    .bind(opts.only_sessions.as_deref())
    .fetch_all(pool)
    .await?;
    let named: Vec<i64> = named
        .into_iter()
        .filter(|id| !client.contains(id))
        .collect();
    report.family_protected_entry = named.len() as u64;
    if !opts.dry_run && !named.is_empty() {
        sqlx::query(
            "UPDATE sessions SET client_class = 'family:protected-entry', client_rev = client_rev + 1
             WHERE id = ANY($1) AND (client_class IS NULL OR client_class = 'none')",
        )
        .bind(&named)
        .execute(pool)
        .await?;
    }
    Ok(report)
}

/// The effective-class census (security O3): every session, read-only. Each
/// session is counted once, under the first that applies: its own content
/// hits (local), another member of its family does (family-only), a protected
/// journal entry names it or its family (protected-entry-only), else lab, of
/// which term-only counts the lab sessions the distiller's terms would protect.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Census {
    pub sessions: u64,
    pub local: u64,
    pub family_only: u64,
    pub protected_entry_only: u64,
    pub lab: u64,
    pub term_only: u64,
}

fn find(parent: &mut Vec<usize>, i: usize) -> usize {
    let mut r = i;
    while parent[r] != r {
        r = parent[r];
    }
    let mut j = i;
    while parent[j] != r {
        let next = parent[j];
        parent[j] = r;
        j = next;
    }
    r
}

fn union(parent: &mut Vec<usize>, a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        parent[ra] = rb;
    }
}

/// Read-only: classifies every session's full stored form and closes the
/// families in memory. `only_sessions` limits it (tests).
pub async fn census(
    pool: &PgPool,
    policy: &Policy,
    only_sessions: Option<&[i64]>,
) -> anyhow::Result<Census> {
    let rows: Vec<(i64, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, provider, session_id, file_path FROM sessions
         WHERE ($1::bigint[] IS NULL OR id = ANY($1)) ORDER BY id",
    )
    .bind(only_sessions)
    .fetch_all(pool)
    .await?;
    let index: std::collections::HashMap<i64, usize> =
        rows.iter().enumerate().map(|(i, r)| (r.0, i)).collect();
    let mut parent: Vec<usize> = (0..rows.len()).collect();
    let mut by_provider_id: std::collections::HashMap<(&str, &str), usize> =
        std::collections::HashMap::new();
    for (i, (_, provider, sid, _)) in rows.iter().enumerate() {
        if let Some(&j) = by_provider_id.get(&(provider.as_str(), sid.as_str())) {
            union(&mut parent, i, j);
        } else {
            by_provider_id.insert((provider.as_str(), sid.as_str()), i);
        }
    }
    for (i, (_, provider, _, file_path)) in rows.iter().enumerate() {
        if let Some(p) = file_path.as_deref().and_then(parent_from_path) {
            if let Some(&j) = by_provider_id.get(&(provider.as_str(), p)) {
                union(&mut parent, i, j);
            }
        }
    }
    let protected: Vec<Vec<i64>> = sqlx::query_scalar(
        "SELECT session_ids FROM journal_entries WHERE route = 'protected' AND route_at = generated_at",
    )
    .fetch_all(pool)
    .await?;
    let mut named = vec![false; rows.len()];
    for ids in &protected {
        let members: Vec<usize> = ids.iter().filter_map(|id| index.get(id).copied()).collect();
        for w in members.windows(2) {
            union(&mut parent, w[0], w[1]);
        }
        for m in members {
            named[m] = true;
        }
    }
    let mut local = vec![false; rows.len()];
    let mut term = vec![false; rows.len()];
    for (i, (id, ..)) in rows.iter().enumerate() {
        let (class, t) = classify_session(pool, policy, *id).await?;
        local[i] = class != Class::None;
        term[i] = t;
    }
    let mut comp_local: std::collections::HashMap<usize, bool> = std::collections::HashMap::new();
    let mut comp_named: std::collections::HashMap<usize, bool> = std::collections::HashMap::new();
    for i in 0..rows.len() {
        let r = find(&mut parent, i);
        *comp_local.entry(r).or_default() |= local[i];
        *comp_named.entry(r).or_default() |= named[i];
    }
    let mut c = Census {
        sessions: rows.len() as u64,
        ..Census::default()
    };
    for i in 0..rows.len() {
        let r = find(&mut parent, i);
        if local[i] {
            c.local += 1;
        } else if comp_local[&r] {
            c.family_only += 1;
        } else if comp_named[&r] {
            c.protected_entry_only += 1;
        } else {
            c.lab += 1;
            if term[i] {
                c.term_only += 1;
            }
        }
    }
    Ok(c)
}
