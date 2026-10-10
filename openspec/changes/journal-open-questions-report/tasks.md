# Tasks

> **Gate:** ac decided Q1 on 2026-10-10 (yes: the distiller links and resolves). Groups 2–5
> follow the revised design (D1–D3). Ship (group 6) needs ac.

## 1. Measure before building

- [x] 1.1 Embed one busy project's last-30-day questions locally (the same bge model,
      from the hub's model dir), offline, without writing to pg1.
- [x] 1.2 Hand-label about 100 question pairs from that project as same-thread or
      different. Measure the precision and recall of cosine thresholds 0.80–0.92 and pick
      the threshold that keeps precision ≥ 0.95 (a wrong merge is worse than a duplicate).
      **Done 2026-10-09 — result: STOP, D1 revised (see design.md D1).** Busiest project,
      last 30 days: 107 questions over 23 active days, 5,471 cross-day pairs. Top 100 pairs
      hand-labelled (ambiguous counted as different): 25 same-thread. Precision by threshold:
      ≥ 0.85 → 12/12; ≥ 0.84 → 0.92; ≥ 0.80 → 0.82; ≥ 0.77 → 0.71. Recall at 0.85 is about
      half, and single-link grouping there turns 107 questions into 97 threads.

## 2. Storage

- [x] 2.1 Migration `0012`: `journal_thread_id_seq`; `open_question_threads BIGINT[]` and
      `resolved_threads BIGINT[]` on `journal_entries`; seed one id per existing question.

## 3. Hub

- [x] 3.1 POST `/v1/journal/entries`: accept, validate (D2) and upsert the two arrays.
- [x] 3.2 Thread fold and states (D3) as a pure function with unit tests: restated,
      resolved, reopened, quiet by active days, calendar gap.
- [x] 3.3 `GET /v1/journal/open-questions` (D3) with read auth.
- [x] 3.4 PG integration tests: new + continued ids, foreign/unknown id 400, continued and
      resolved 400, length mismatch 400, re-distill replaces links, migration seeding,
      report states, unauthenticated 401. Migration seeding checked by hand on a scratch DB
      (2026-10-10): oldest entry's questions got ids 1, 2, the next 3, 4; the sequence went
      on at 5; a misaligned row is refused by the CHECK. `journal_threads_test.rs` (7 tests)
      and 5 fold unit tests; a mutation that skips the unknown-id check fails the suite.

## 4. Distiller

- [x] 4.1 Fetch the open threads before generating; prompt lists them; parse
      `{"q", "continues"}` objects and plain strings; `resolved` list.
- [x] 4.2 Sanitise: drop unoffered ids and ids both continued and resolved; post the two
      arrays. pytest for prompt, parsing and sanitising.

## 5. Gate

- [ ] 5.1 fmt, clippy on CI's Rust version, all crate tests with `--test-threads=1`,
      distiller pytest.

## 6. Ship (needs ac)

- [ ] 6.1 Release (hub + migration); infra reinstalls the distiller in the same batch.
      Live check: one entry written after the swap has `open_question_threads` set.
- [ ] 6.2 `cchv-find` skill: document the endpoint (relay to the CONTEXT owner).
