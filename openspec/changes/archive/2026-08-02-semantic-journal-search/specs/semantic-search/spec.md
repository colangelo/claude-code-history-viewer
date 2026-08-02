# semantic-search Specification (new capability)

## ADDED Requirements

### Requirement: Hub-local embedder

The hub SHALL embed text with a small sentence-embedding model executed
in-process on CPU, loaded lazily from a filesystem directory configured
via `embed_model_dir` (hub.toml) / `HUB_EMBED_MODEL_DIR` (env). The same
embedder MUST be used for entry embeddings and query embeddings (a
cosine space is only meaningful within one model), and every stored
embedding SHALL record the model identifier it was produced with. No
network call is made for embedding: the archive currently has no
reachable embeddings API (aiproxy `/v1/embeddings` is a 404), and query
latency must not depend on an external service. When the model directory
is absent or fails to load, the hub MUST keep serving every existing
endpoint unchanged — semantic features degrade per the degradation
requirement, never a startup failure.

#### Scenario: Missing model directory is not fatal

- **WHEN** the hub starts with `embed_model_dir` unset or pointing at a missing/corrupt directory
- **THEN** it serves all pre-existing endpoints normally and `mode=semantic|hybrid` degrade to keyword behavior

#### Scenario: One space per model

- **WHEN** entries were embedded with model A and the hub is later configured with model B
- **THEN** semantic queries only ever compare vectors sharing the active model's identifier, and stale model-A rows are re-embedded by the sweep

### Requirement: Journal embedding storage and lifecycle

Journal-entry embeddings SHALL live in a dedicated additive table
(`journal_embeddings`) keyed by (journal entry, model), storing the
vector as a plain float array with its dimension and a content hash of
the embedded text (headline + summary + topics + open questions). A
background sweep — at startup, on an interval, and after journal-entry
writes — SHALL (re)embed entries whose current content hash differs from
the stored one, which also performs the one-time bootstrap of
pre-existing entries and self-heals interrupted runs. Embedding rows are
derived data: deleting them all MUST be safe (the sweep regenerates) and
no journal, session, or message row is ever modified by this capability.

#### Scenario: Bootstrap of existing entries

- **WHEN** the feature first deploys against an archive with existing journal entries and an empty `journal_embeddings` table
- **THEN** the sweep embeds every `entry`-status row without external input, and semantic queries work once it completes

#### Scenario: Regenerated entry is re-embedded

- **WHEN** the distiller regenerates an entry for a (date, project) group with new content
- **THEN** the stored content hash no longer matches and the next sweep replaces that entry's embedding

#### Scenario: Skip rows are never embedded

- **WHEN** the sweep encounters `skip`-status journal rows
- **THEN** they are excluded (they carry no retrievable content)

### Requirement: Journal-scale storage without pgvector

At journal scale the store SHALL use plain Postgres types (`real[]`)
with similarity computed in the hub process over the full (small) set of
active-model embeddings — exact results, no extension dependency, so
dev/test/CI and pg1 all work unchanged. The pgvector/`halfvec` path
(extension, ANN index, the pg1 disk envelope infra sized) is explicitly
RESERVED for the future message-scale phase; adopting it there MUST be
possible via a new additive migration without touching this table's
consumers.

#### Scenario: Works on a vanilla Postgres

- **WHEN** migration `0004` runs on a Postgres with no pgvector extension installed
- **THEN** it applies cleanly and all semantic features function

### Requirement: Semantic and hybrid query modes with keyword degradation

Journal search SHALL support three modes: `keyword` (existing FTS —
the default, byte-compatible), `semantic` (cosine similarity between the
embedded query and active-model entry embeddings, best-first), and
`hybrid` (reciprocal-rank fusion of the keyword and semantic rankings).
The existing hit shape is reused with the `rank` field carrying the
active mode's score. When the embedder or embeddings are unavailable,
`semantic` and `hybrid` MUST degrade to `keyword` results (never an
error), and the response SHALL indicate the degradation so callers can
tell recall quality dropped.

#### Scenario: Paraphrase recall (the measured gap)

- **WHEN** a query paraphrases an entry with no distinctive shared vocabulary (e.g. "keeping config files identical on both laptops" vs a chezmoi-sync headline)
- **THEN** `mode=semantic` and `mode=hybrid` surface that entry among the top journal hits, where `keyword` returns nothing

#### Scenario: Keyword mode is byte-compatible

- **WHEN** a request omits `mode` or sends `mode=keyword`
- **THEN** the journal block is byte-identical to pre-change behavior

#### Scenario: Degraded gracefully

- **WHEN** `mode=hybrid` is requested but the embedder is unavailable
- **THEN** keyword results return with a degradation indicator and HTTP 200
