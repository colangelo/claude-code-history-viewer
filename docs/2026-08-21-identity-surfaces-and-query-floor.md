---
type: reference
title: "2026-08-21 (afternoon) — build identity surfaces shipped, and the query floor measured"
description: "cchv-v0.21.0 makes a running build nameable; then four measurements of the journal fold, three checks that could not fail, and four corrections traded with infra."
tags: [release, hub, distiller, postgres, measurement, deploy, postmortem]
timestamp: 2026-08-21
---
# 2026-08-21 (afternoon) — identity surfaces, and the query floor

Successor session to `docs/2026-08-21-journal-day-bucketing.md` (same day, morning). That
doc is the wrong-day journal bug; this one is what the successor did after signing off on
it: shipped **`cchv-v0.21.0`** (#39, #40) and measured **#36/#41/#30** into a specced
change. The per-task detail is already written where it belongs —
`docs/archive/deployment.md` §2b/§3c for the deploy, `openspec/changes/journal-query-floor/`
for the measurement reasoning, and the issues for the numbers. **This doc is only the
cross-cutting layer**: what binds afterwards, and what is open.

## What shipped

| Thing | Verified how |
|---|---|
| `cchv-v0.21.0` — `version` on `/v1/healthz` (#39) | `null` → `"0.21.0"`; all six workflows green on the tag sha `9b0633cb` (first fully-green tag since the rkyv advisory) |
| Distiller announces `DISTILL_VERSION` + its own git blob id (#40) | live log line `cchv-distill 0.21.0 blob=a44b4d32bb99 mode=forward`; blob = `git rev-parse cchv-v0.21.0:scripts/cchv-distill.py` |
| Both halves readable in one GET | `/v1/healthz/journal` → `hub_version` = `last_tick_distiller_version` = `0.21.0` after the 14:57:30Z tick |
| `sync-version`'s fifth target | marker validated **before** any write, so a missing marker is a refusal, not a half-synced bump |
| Deployed to m4m | three steps by infra; confirmed independently from Gatus on `mon` |
| `journal-query-floor` specced (#36, #41) | 18 tasks; index build deliberately **not** run — a production write |
| #30 re-investigated | 0.48 % duplication, not 23×; it collapses into #41 |
| 3 openspec changes archived | `distiller-tick-observability`, `build-identity-surfaces`, `hub-mirror-refresh-timeout` |

## Learnings → rules

1. **A check that cannot fail is worse than no check, because it reads as the control.**
   Three separate instances in one afternoon, each of which produced a confident wrong
   answer before being caught:
   - `count(*)` vs `count(DISTINCT uuid)` to measure ingest duplication returned a clean
     **1.0** — and could never have returned anything else, because `convert.rs` (header,
     lines 4–5) fills a missing uuid with a **random v4** on every re-parse. Also found:
     `content_hash` is populated on **0** rows, so it cannot serve as an identity either.
   - Timing an endpoint with `curl --max-time 10` abandoned the HTTP request while
     **leaving the query running server-side**; six accumulated and starved each other on
     IO, so the "measurement" was of my own backlog.
   - A wait loop written as `until [ "$(…)" != "null" ]` treated a *failed read* (empty
     string) as success and printed **"IDENTITY TICK LANDED"** for a tick that had not
     happened. Same defect as `--is-ancestor` collapsing 1 and 128 (morning doc, rule 5):
     "no" and "I cannot answer" on one wire.
   *Rule:* before trusting a check, ask what reading would make it **fail**. If you cannot
   name one, it is not a check. *Evidence:* #30 comment 7493; #36 comment 7398.

2. **Publish the number from the machine that will run it.** Migration `0009` was measured
   at **0.93 ms** against a local table of prod's exact shape and took **7.15 ms** on prod
   — **7.7×**. The conclusion was unaffected, but the figure would have been extrapolated
   later to a bigger table. "Measured, not estimated" was true and still misleading:
   measuring the wrong machine is its own kind of estimate. *Evidence:* infra on the
   v0.21.0 deploy; `deployment.md` §2b.

3. **A measurement taken inside a cycle, without its position in that cycle, is a
   measurement of the day.** `messages` is append-only, so VACUUM there is driven by the
   *insert* trigger (~every 1.5 M inserts) and the visibility map over the hot range decays
   between runs. The fraction of a 7-day window needing `Heap Fetches` therefore swings
   **0 % → ~60 %** across a cycle: the same index measures excellent or mediocre depending
   on the hour. At the time of writing the position was **75 %**, i.e. the pessimistic end
   — the direction that gets a *correct* change rejected. *Rule:* record
   `n_ins_since_vacuum` and `relallvisible`/`relpages` with every timing, and state which
   way the position biases the result. *Evidence:* #36 comment 7403; `journal-query-floor`
   task 3.0.

4. **A threshold is not a schedule.** `last_autovacuum` two days old was read as "autovacuum
   is behind". It was not: the dead-tuple trigger (1,515,936) will *never* fire on an
   append-mostly archive holding 929 dead tuples, and that is correct behaviour. The real
   exposure was one level down (rule 3). Naming a *state* from a *timestamp* without
   checking the mechanism that sets it is the general form. *Evidence:* #36 comment 7403.

5. **The catalogue of traps makes a plausible story cheaper to reach for, not harder.**
   Between this session and infra, four corrections were traded in one afternoon, two each
   way — a "floor vs peak" story for an unexplained 1.55× gap, the autovacuum misreading, a
   `pg_stat_reset()` suggestion that would have erased the baseline it was measuring, and a
   claim that the new `version` field retires the `strings`-marker family (it cannot answer
   for a staged-but-unstarted binary — two proofs, two moments). Every one was caught by
   *the other party holding a reading*. Infra's own instances were reaching for the
   BSD-grep rule on a line-wrapped marker, and "a sibling session restructured it" for a
   file they had truncated themselves. *Rule:* a catalogue tells you what a symptom **could**
   be; only a reading tells you what it **is**.

## Open at handover

| Issue | What | Blocked on |
|---|---|---|
| **#36** | journal query floor — covering index + `work_mem`. Specced as `journal-query-floor`, 18 tasks, 0 done | nothing — ac said start |
| **#41** | row counts ≠ conversation turns | the docs half rides #36's tasks 4.1/4.2; the analytics count is a separate change |
| **#30** | `seq`-in-`message_key` duplication, now measured at **≤0.48 %** | `needs/decision` — recommendation is *accept and document*, because the proposed fix risks collapsing genuine repeats |
| **#34** | credentials stored verbatim, no purge path | `needs/design`; shares the "what does the archive keep" question with #41, deliberately **not** batched |
| #11, #23 | cargo audit; drop the Tauri shell | untouched |

**Sequencing decision (ac, 2026-08-21):** these do **not** go out as one batch. #36 is a
four-arm measurement campaign and the others perturb the table it measures — #30 changes
what is *written* to it, #41's analytics half adds *readers*. They also land on three
different deploy surfaces (a pg1 DDL, a hub release, and `sync-daemon` on **every**
machine), so batching saves no window. Order: `journal-query-floor` (#36 + #41 docs) →
#41 analytics, its own change → #30 only if the decision is to fix it.
