//! Background embedding sweep: keeps `journal_embeddings` current with
//! `journal_entries` content for the active embedder model.
//!
//! The sweep is the ONLY writer of embedding rows. It runs at startup, on an
//! interval, and is nudged by journal writes (`AppState::embed_nudge`); dirty
//! detection is a content hash of the embedded text stored on each row, so
//! regenerated entries re-embed, no-ops stay no-ops, and interrupted or
//! deleted state self-heals on the next pass (bootstrap of pre-existing
//! entries is just the first sweep). Embedding rows are derived data — no
//! journal, session, or message row is ever modified here.
//!
//! Runtime `sqlx::query*` on purpose: the CI gate builds with `SQLX_OFFLINE`
//! (see the note at the top of `journal.rs`).

use sqlx::{PgPool, Row};
use std::time::Duration;

use crate::embed::Embedder;
use crate::state::AppState;

/// The text a journal entry is embedded from: its human-phrased content
/// fields, newline-joined. Identifying fields (date, path, session ids) are
/// deliberately excluded — they carry no semantic signal and FTS already
/// covers exact lookups on them.
pub fn embed_text(
    headline: Option<&str>,
    summary: Option<&str>,
    topics: &[String],
    open_questions: &[String],
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(h) = headline {
        if !h.trim().is_empty() {
            parts.push(h.trim().to_string());
        }
    }
    if let Some(s) = summary {
        if !s.trim().is_empty() {
            parts.push(s.trim().to_string());
        }
    }
    if !topics.is_empty() {
        parts.push(topics.join(", "));
    }
    if !open_questions.is_empty() {
        parts.push(open_questions.join(" "));
    }
    parts.join("\n")
}

/// Dirty marker for an entry's stored embedding: hex sha-256 of the exact
/// text that would be embedded now. Hashing the embed input itself is the
/// stable serialization — identical input ⇒ identical vector ⇒ clean.
pub fn content_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepStats {
    /// Rows (re)embedded this pass.
    pub embedded: usize,
    /// Rows whose embed or upsert failed (retried next pass).
    pub failed: usize,
}

/// One pass: (re)embed every `entry`-status journal row whose active-model
/// embedding is missing or hash-stale. Per-entry failures are isolated — one
/// bad row never blocks the rest — and left for the next pass.
pub async fn sweep(pool: &PgPool, embedder: &dyn Embedder) -> Result<SweepStats, sqlx::Error> {
    let rows = sqlx::query(
        r"
        SELECT je.id, je.headline, je.summary, je.topics, je.open_questions,
               e.content_hash AS stored_hash
        FROM journal_entries je
        LEFT JOIN journal_embeddings e
            ON e.journal_entry_id = je.id AND e.model = $1
        WHERE je.status = 'entry'
        ORDER BY je.id
        ",
    )
    .bind(embedder.model_id())
    .fetch_all(pool)
    .await?;

    let mut stats = SweepStats::default();
    for row in rows {
        let id: i64 = row.get("id");
        let headline: Option<String> = row.get("headline");
        let summary: Option<String> = row.get("summary");
        let topics: Vec<String> = row.get("topics");
        let open_questions: Vec<String> = row.get("open_questions");
        let stored_hash: Option<String> = row.get("stored_hash");

        let text = embed_text(
            headline.as_deref(),
            summary.as_deref(),
            &topics,
            &open_questions,
        );
        let hash = content_hash(&text);
        if stored_hash.as_deref() == Some(hash.as_str()) {
            continue;
        }

        // ~tens of ms on CPU per entry; sweeps run in a background task, so
        // briefly occupying the worker is fine at journal scale.
        let vector = match embedder.embed(&text) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(entry_id = id, error = %format!("{e:#}"),
                    "embed failed; entry left for next sweep");
                stats.failed += 1;
                continue;
            }
        };

        let dim = i16::try_from(vector.len()).unwrap_or(i16::MAX);
        let upsert = sqlx::query(
            r"
            INSERT INTO journal_embeddings
                (journal_entry_id, model, dim, embedding, content_hash)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (journal_entry_id, model)
            DO UPDATE SET dim          = excluded.dim,
                          embedding    = excluded.embedding,
                          content_hash = excluded.content_hash,
                          created_at   = now()
            ",
        )
        .bind(id)
        .bind(embedder.model_id())
        .bind(dim)
        .bind(&vector)
        .bind(&hash)
        .execute(pool)
        .await;
        match upsert {
            Ok(_) => stats.embedded += 1,
            Err(e) => {
                tracing::warn!(entry_id = id, error = %e,
                    "embedding upsert failed; entry left for next sweep");
                stats.failed += 1;
            }
        }
    }
    if stats.embedded > 0 || stats.failed > 0 {
        tracing::info!(embedded = stats.embedded, failed = stats.failed,
            model = embedder.model_id(), "embedding sweep pass");
    }
    Ok(stats)
}

/// Long-running sweeper: a pass at startup, then one per `interval` or
/// whenever a journal write nudges `embed_nudge` — whichever comes first.
pub async fn run_sweeper(state: AppState, interval: Duration) {
    let Some(embedder) = state.embedder.clone() else {
        return;
    };
    loop {
        if let Err(e) = sweep(&state.pool, embedder.as_ref()).await {
            tracing::warn!(error = %e, "embedding sweep pass failed");
        }
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = state.embed_nudge.notified() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_text_joins_content_fields_only() {
        let t = embed_text(
            Some("Headline"),
            Some("A summary."),
            &["one".into(), "two".into()],
            &["why?".into()],
        );
        assert_eq!(t, "Headline\nA summary.\none, two\nwhy?");
    }

    #[test]
    fn embed_text_skips_empty_fields() {
        assert_eq!(embed_text(None, Some("  "), &[], &[]), "");
        assert_eq!(embed_text(Some("H"), None, &[], &[]), "H");
    }

    #[test]
    fn content_hash_is_stable_and_content_sensitive() {
        let a = content_hash("same text");
        let b = content_hash("same text");
        let c = content_hash("different text");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64);
    }
}
