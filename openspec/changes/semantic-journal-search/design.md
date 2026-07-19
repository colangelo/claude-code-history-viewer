# Design: semantic-journal-search

## Context

Measured recall gap (2026-07-19, live hub, recorded in
`docs/archive/deployment.md` "future phases" note and memory
`archive-retrieval-roadmap`): keyword recall 6/6, paraphrase recall
~0/6, no stemming (`'simple'` FTS config, kept for CJK safety — not
changing it). The retrieval corpus that matters is the distilled journal
(~92 entries today, grows a few/day): every failed paraphrase query was
really a journal-entry lookup. Environment facts established by probing,
not assumption:

- **aiproxy has NO embeddings**: `POST /v1/embeddings` → hard 404 for
  both OpenAI- and Gemini-style model names; `/v1/models` lists chat +
  image models only. Any HTTP-embeddings design would block on an infra
  ask of uncertain feasibility (CLIProxyAPI may not support embeddings
  passthrough at all).
- **Local dev Postgres 18.4 (Homebrew) has no pgvector**; prod pg1 has
  pgvector 0.8.5 installed but the extension is not necessarily created
  in `cchv_archive`, and creating it needs privilege coordination.
- Hub deploys as a **single ad-hoc-signed binary** swapped per §2b —
  runtime dylib dependencies would complicate that story.
- The hub is the only place query-time embedding can live (a semantic
  query must be embedded in the request path by *someone*; clients like
  cchv-find are curl-level).

## Goals / Non-Goals

**Goals:**

- Fix the measured paraphrase-recall gap for journal retrieval, cheaply,
  with zero new external service/key dependencies.
- Keep `mode` absent/`keyword` byte-compatible; degrade — never fail —
  when the embedder is unavailable.
- Keep the whole stack testable on vanilla Postgres (dev/CI) and
  deployable as the same single-binary swap.

**Non-Goals:**

- Message-level embeddings (100k+ rows) — explicitly the later phase;
  that is where pgvector/`halfvec` + the pg1 disk envelope apply.
- Changing FTS behavior (stemming/config) — CJK safety keeps `'simple'`.
- Embedding via aiproxy/OpenAI — nothing to call today (404); a
  configurable HTTP backend can be added behind the same trait later.
- Semantic ranking of the message-results leg of `/v1/search`.

## Decisions

### D1. Journal-first scope

Embed the ~92-and-growing distilled entries, not messages. The entries
are human-phrased, high-signal, and tiny — embedding cost is a rounding
error, and the measured misses were all journal lookups. Message-scale
semantic search stays deferred until journal-level proves insufficient
(explicitly spec'd boundary).

### D2. Plain `real[]` storage + in-process cosine; pgvector deferred

Alternative considered: `halfvec` + pgvector now (it's installed on
pg1). Rejected for this phase: local dev/CI Postgres lacks the
extension (Homebrew PG18 + pgvector is a source-build detour), prod
`CREATE EXTENSION` needs privilege coordination with infra, and at
journal scale exact in-process cosine over all active-model vectors is
byte-identical in results and effectively free (thousands of entries ×
384 dims is microseconds). Migration `0004` therefore uses plain types
and runs anywhere. The message-scale phase graduates to pgvector with a
new additive migration; nothing in this phase's consumers assumes the
storage representation.

### D3. Embedder = small local model, in-process, pure-Rust runtime

Alternatives considered:
- **HTTP OpenAI-compatible client** — dead on arrival today (aiproxy
  404, no house OpenAI API key story); remains a natural later addition
  behind the same `Embedder` trait.
- **fastembed-rs / ort (ONNX Runtime)** — mature, but links/loads the
  onnxruntime native library: threatens the single-binary §2b codesign
  swap and adds a build-time native dependency.
- **Distiller-side (Python) embedding** — could embed entries, but NOT
  queries; the hub still needs an embedder in the request path, so this
  would duplicate rather than remove the requirement.

Chosen: **candle** (pure-Rust, CPU) running **bge-small-en-v1.5**
(384-dim, the standard small retrieval baseline; journal entries are
English by construction — distiller output). Weights + tokenizer
(~65 MB) live in a directory staged at deploy (`HUB_EMBED_MODEL_DIR`),
NOT in git and NOT compiled into the binary; lazy-loaded on first use.
The embedder sits behind a small `Embedder` trait so tests use a
deterministic stub and a future HTTP backend slots in. If candle's
build/runtime cost surprises during implementation, the trait is the
firewall: swapping the backend does not touch schema, sweep, or API.

### D4. Hub-side embedding sweep (no distiller changes, no new write endpoints)

Alternative considered: distiller embeds entries and POSTs vectors
(mirrors the distillation pattern). Rejected: the hub must own an
embedder anyway for queries (D3), so entry embedding in the hub
eliminates a second embed client, new machine-token endpoints, and a
distiller deploy — the daemon/distiller are untouched entirely.
Mechanics: sweep runs at startup, on an interval, and is nudged after
`POST /v1/journal/entries`; dirty detection is a content hash (headline
+ summary + topics + open_questions) stored on the embedding row —
regenerated entries re-embed, no-ops stay no-ops, interrupted sweeps
self-heal. Bootstrap of pre-existing entries is just the first sweep.

### D5. Query modes and fusion

`mode=keyword|semantic|hybrid` on the journal leg only. `semantic` =
cosine between embedded query and active-model entry vectors.
`hybrid` = reciprocal-rank fusion (RRF, k=60) of the keyword and
semantic rank lists — rank-based fusion avoids score-normalization
between ts_rank and cosine. Hit shape unchanged; `rank` carries the
active mode's score (FTS rank / cosine similarity / RRF score).
Degradation (no embedder, no embeddings yet, dimension/model mismatch)
returns keyword results plus a `journal_degraded: true` field in the
response (additive, absent otherwise). Webapp + cchv-find default to
`hybrid`; old hubs ignore the param (verified pattern — serde default).

### D6. Version and rollout

`cchv-v0.11.0`. Hub-only swap per §2b + stage the model dir once
(`~/.config/cchv/embed-model/` on m4m; relay includes the fetch
recipe). Migration `0004` auto-runs (additive, no extension). Rollback:
previous binary ignores the table; embeddings are derived data
(deleting the table's rows is always safe).

## Risks / Trade-offs

- [candle + tokenizers build weight: compile time, binary size, MSRV]
  → trait firewall (D3); measure in the first implementation task and
  fall back to the HTTP-trait-impl + infra-ask path if unacceptable.
- [bge-small quality on short paraphrase queries] → it's the standard
  small retrieval baseline and the eval bar is concrete: the 6 measured
  paraphrase misses become the acceptance test; if it can't clear them,
  a larger model in the same dir layout is a config change.
- [English-only model vs CJK content] → journal entries are English by
  construction (distiller prompt); messages aren't in scope. Revisit at
  message-scale.
- [Embedding at query time adds latency] → one 384-dim CPU forward pass
  (~ms-tens-of-ms); bounded, local, no network.
- [Model weights are deploy-time state outside the binary] → §2b relay
  gains one `mkdir + fetch` step; absence degrades gracefully (spec'd),
  so a missed step means keyword-only, not an outage.
- [`real[]` reads the whole active-model set per semantic query] →
  thousands of rows max at journal scale; revisit only at message scale
  (where pgvector is the plan anyway).

## Migration Plan

1. Migration `0004_journal_embeddings.sql` (auto-run): the table +
   index on (model), no extension.
2. Deploy hub binary per §2b; stage `HUB_EMBED_MODEL_DIR` weights
   (one-time); launcher env addition in `cchv-launch`'s hub template.
3. First sweep bootstraps all existing entries (~92 × ms — seconds).
4. Webapp `mode=hybrid` rides the same release bundle; cchv-find skill
   doc updated post-verification.
5. Rollback: previous binary; table inert; no config to revert (env var
   simply unused).

## Open Questions

- None blocking. Deferred by design: HTTP embedder backend (needs an
  actual endpoint to exist), message-scale pgvector phase, non-English
  journal content.
