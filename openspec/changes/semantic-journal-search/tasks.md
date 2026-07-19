# Tasks: semantic-journal-search

## 1. Embedder spike (de-risk D3 first)

- [ ] 1.1 Candle spike: load bge-small-en-v1.5 (safetensors + tokenizer.json) from a dir, mean-pool + normalize, embed a sentence on CPU — measure compile-time cost, binary-size delta, and per-embed latency; record numbers in the PR/commit. If unacceptable (build breaks MSRV 1.77.2, binary balloons, or >200ms/embed), STOP and re-decide via the D3 trait firewall before proceeding
- [ ] 1.2 `crates/hub/src/embed.rs`: `Embedder` trait (`fn embed(&self, text: &str) -> Result<Vec<f32>>` + model id + dim) with `CandleEmbedder` (lazy init from `embed_model_dir`/`HUB_EMBED_MODEL_DIR`, init failure → disabled state, never fatal) and a deterministic `StubEmbedder` for tests
- [ ] 1.3 Config plumbing: `embed_model_dir` in `HubConfig` (toml + env, same precedence as every other setting); weights-fetch recipe documented (huggingface bge-small-en-v1.5 → dir layout) in deployment.md §2

## 2. Storage + sweep

- [ ] 2.1 `migrations/0004_journal_embeddings.sql`: `journal_embeddings` (id, journal_entry_id FK → journal_entries ON DELETE CASCADE, model TEXT, dim SMALLINT, embedding REAL[], content_hash TEXT, created_at; UNIQUE (journal_entry_id, model)); plain types only — applies on vanilla Postgres
- [ ] 2.2 Content-hash function over (headline, summary, topics, open_questions) — stable serialization, unit-tested; `skip`-status rows excluded
- [ ] 2.3 Sweep task in hub: find `entry`-status rows whose active-model embedding is missing or hash-stale → embed → upsert; run at startup, on interval, and nudged after `POST /v1/journal/entries`; runtime `sqlx::query*` (SQLX_OFFLINE-safe per journal.rs convention); per-entry failure isolation + tracing
- [ ] 2.4 Integration tests (stub embedder): bootstrap embeds existing entries; regenerated entry re-embeds (hash change); skip rows excluded; model change re-embeds; deleting all embedding rows self-heals

## 3. Query modes

- [ ] 3.1 `mode` param on `SearchParams` (`keyword` default | `semantic` | `hybrid`; unknown → 400); journal leg only
- [ ] 3.2 Semantic ranking: embed query, load active-model vectors (+ entry ids), cosine in-process, top-N best-first mapped into `JournalHit` with `rank` = similarity
- [ ] 3.3 Hybrid: RRF (k=60) over keyword + semantic rank lists; `rank` = RRF score
- [ ] 3.4 Degradation: embedder disabled / zero embeddings / model mismatch → keyword results + additive `journal_degraded: true` on the response (absent otherwise); never an error
- [ ] 3.5 Integration tests: mode=keyword byte-compat (replay pre-change request shape), semantic surfaces a hash-known entry for a paraphrase the FTS misses (stub embedder with contrived vectors), hybrid fuses, degraded path returns 200 + flag, unknown mode 400

## 4. Clients

- [ ] 4.1 Webapp: `hubApi.journalSearch` sends `mode=hybrid` (+ optional `journal_degraded` surfaced as a subtle hint in the search section); vitest for the param + degraded flag parse
- [ ] 4.2 cchv-find skill (`~/_sync/dev/CONTEXT/SKILLS/cchv-find/SKILL.md`): document `mode=` with guidance (hybrid default for recall questions; keyword for exact-term lookups) — post-deploy, after live verification

## 5. Quality gate + acceptance + release prep

- [ ] 5.1 Full gate: `pnpm tsc --build .`, `pnpm vitest run`, `pnpm lint`, `pnpm run i18n:validate` (if UI strings added), `SQLX_OFFLINE=true cargo test --workspace --exclude claude-code-history-viewer -- --test-threads=1` (with TEST_DATABASE_URL), clippy `-D warnings`, fmt
- [ ] 5.2 Acceptance rerun of the measured gap: with real weights locally, the 6 recorded paraphrase queries (recall spot-check 2026-07-19) each surface their target entry in top-5 via `mode=hybrid` against a seeded hub — the concrete bar for D3's model-quality risk
- [ ] 5.3 deployment.md: §2 gains the model-dir staging step + `HUB_EMBED_MODEL_DIR`; "future phases" note updated (journal semantic = DONE, message-scale pgvector remains)
- [ ] 5.4 Rebase onto current `main`, merge, bump 0.11.0 + `just sync-version`, release per process (coordinate with the release train — check `git tag` for the latest `cchv-v0.10.x` first); deploy relay: §2b swap + one-time model-dir staging
