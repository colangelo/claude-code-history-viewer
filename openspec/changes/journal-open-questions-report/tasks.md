# Tasks

## 1. Measure before building

- [ ] 1.1 Embed one busy project's last-30-day questions locally (the same bge model,
      from the hub's model dir), offline, without writing to pg1.
- [ ] 1.2 Hand-label about 100 question pairs from that project as same-thread or
      different. Measure the precision and recall of cosine thresholds 0.80–0.92 and pick
      the threshold that keeps precision ≥ 0.95 (a wrong merge is worse than a duplicate).
      Record the numbers here. If no threshold reaches 0.95 precision, stop and revisit D1.

## 2. Storage and sweep

- [ ] 2.1 Migration: `journal_question_embeddings(entry_id, ordinal, model, embedding
      REAL[], PRIMARY KEY (entry_id, ordinal, model))`, cascading with the entry row. Plain
      `REAL[]` like `journal_embeddings` (0004: deliberately not pgvector at journal scale).
- [ ] 2.2 `embed_sweep.rs`: a new source that embeds questions of entries lacking rows.
      A re-distilled entry replaces its question rows.
- [ ] 2.3 PG integration test: an entry with 3 questions gets 3 rows; a re-distill with
      2 questions leaves 2.

## 3. Report

- [ ] 3.1 Grouping (D1) and states (D2) as pure functions, with unit tests on fixed
      vectors: merge, no cross-project merge, quiet by active days, calendar gap.
- [ ] 3.2 `GET /v1/journal/open-questions` (D3) with read auth. Integration test over the
      throwaway PG.
- [ ] 3.3 Clippy on CI's Rust version, fmt, `--test-threads=1`.

## 4. Ship (needs ac)

- [ ] 4.1 Release, then infra deploys. The sweep backfills about 2.5 k questions; check
      its duration in the hub log.
- [ ] 4.2 `cchv-find` skill: document the endpoint (relay to the CONTEXT owner).
