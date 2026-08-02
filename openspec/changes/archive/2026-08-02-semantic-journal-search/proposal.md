# Proposal: semantic-journal-search

## Why

Archive recall is keyword-strong but concept-blind — measured 2026-07-19
against the live hub: 6/6 known journal entries found via exact terms,
**~0/6 via paraphrase** ("keeping config files identical on both laptops"
→ zero results for the chezmoi-sync entry). Root cause is structural:
`websearch_to_tsquery('simple', …)` AND-s terms and does no stemming
(`deploying`→1 hit vs `deployed`→10, confirmed), so the most common real
recall need — "I vaguely remember we discussed X" — returns nothing or
common-word noise. The ~92 distilled journal entries are a tiny,
high-signal, human-phrased corpus: embedding *them* is nearly free and
directly fixes the measured misses.

## What Changes

- The hub gains a **local embedder** (small sentence-embedding model run
  in-process on CPU; no network, no keys — measured: aiproxy has **no**
  embeddings endpoint, hard 404) used for both entry embeddings and
  query-time embeddings.
- A **`journal_embeddings` side table** (additive migration `0004`,
  `real[]` vectors — **deliberately NOT pgvector** at this scale; see
  design D2) stores one embedding per (entry, model) with content-hash
  dirty detection; a background sweep (startup + interval + after journal
  writes) keeps it current, including the one-time bootstrap of existing
  entries.
- `GET /v1/search` journal leg gains a **`mode` param**:
  `keyword` (default — byte-compatible), `semantic` (cosine KNN),
  `hybrid` (reciprocal-rank fusion of both). Embedder unavailable →
  graceful degradation to keyword.
- The webapp's `journalSearch` requests `mode=hybrid` (older hubs ignore
  the unknown param → unchanged behavior); the `cchv-find` skill is
  updated to use and document the modes.
- **Message-level embeddings stay a later phase** — that's where pgvector
  (`halfvec`, pg1 envelope already sized by infra) actually earns its keep.

## Capabilities

### New Capabilities

- `semantic-search`: the hub-local embedder, journal-embedding lifecycle
  (storage, content-hash refresh, bootstrap/sweep), semantic + hybrid
  query modes with keyword degradation, and the explicit journal-scale /
  message-scale storage boundary.

### Modified Capabilities

- `archive-search-api`: the `/v1/search` journal leg accepts
  `mode=keyword|semantic|hybrid`; default remains byte-compatible.

## Impact

- **Crates**: `hub` (embedder module + model assets loading, migration
  `0004`, sweep task, mode param + RRF; new dev-time model-weight
  fixture for tests). No `sync-daemon`, `protocol`, or `history-core`
  changes — daemons and wire format are untouched.
- **Webapp**: one-line `mode=hybrid` in `hubApi.journalSearch`.
- **Skill**: `~/_sync/dev/CONTEXT/SKILLS/cchv-find` documents the modes
  (post-deploy).
- **DB**: additive migration `0004` (`journal_embeddings`, plain types —
  works on any Postgres; no extension required). Rollback = previous
  binary; table sits inert.
- **Deploy**: hub-only swap per `deployment.md` §2b + a one-time model
  weights directory staged next to the binary (`HUB_EMBED_MODEL_DIR`).
- **Version**: ships as `cchv-v0.12.0` (minor: new feature + migration;
  v0.11.x was consumed by the release train while this change was drafted).
