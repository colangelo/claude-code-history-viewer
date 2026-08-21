---
type: reference
title: "2026-08-21 — the journal wrong-day bug, and what became rules"
description: "One reported symptom, four defects of the same shape; three releases; a 401-group backfill; and the checks that could not fail."
tags: [journal, hub, distiller, release, deploy, postmortem]
timestamp: 2026-08-21
---
# 2026-08-21 — the journal wrong-day bug, and what became rules

ac reported the archive journal showing 2026-08-20 work under 2026-08-19. That single
symptom had **four** causes, three of them the same mistake in different places, and
finding the last two took a production deploy each. Tracker: #35 (closed), #37 (closed),
#38 (closed as dup of #11), #36/#39/#40/#41 (open).

## What shipped

| Thing | Verified how |
|---|---|
| `cchv-v0.19.0` — grouping folds on **message** date, not session start | 2026-08-20 entries exist for the first time (20 of them); spanning sessions appear under both days |
| Distiller reinstall (infra, 08:15Z) — windowed transcripts | installed file `cmp`-identical to `HEAD`; `build_transcript` takes `entry_date` |
| `cchv-v0.20.0` — single-pass health + per-day arrival | health 6.9 s → 3.4 s; probe `POST /v1/journal/ticks` 405 → 401 with two controls |
| `cchv-v0.20.1` — dirty detection at day granularity (#37) | 503/3 groups → 200/0; migration 0008 in **13 ms**; held while 6,669 records ingested for the live repo |
| rkyv reviewed-ignore + a CI guard that can expire it | `Security Audit` success on `2a318594`; guard fails if an absence-based ignore enters the build graph |
| Historical backfill — 401 groups | 0 never-distilled / 0 wrong-session-set / **562 correct**; 199 entries + 202 skips, 0 failed |
| `journal-day-bucketing` archived | 6 requirements folded into `journal-entries`, `journal-health`, `archive-search-api` |

**The before/after that closes #35:** the Vikunja board moved out of the 19th's entry and
into the 20th's, where it happened, and the 19th greps clean for every 20th-only subject.

## Learnings → rules

1. **When a change alters the granularity of a key, sweep every predicate that tests that
   key — at once, not as they occur to you.** Grouping moved session → day; the transcript
   window, the arrival timestamp and the dirty check all still tested it at session
   granularity. Two of the three were found in production, after shipping, by someone
   noticing an odd number. *Evidence:* #37; `design.md` had named the fourth defect in its
   risk list and dismissed it with "that is correct — those days really are dirty", a
   claim about live behaviour made by intuition with the archive one query away.

2. **An installed script is a *copy*: a green `main` says nothing about what is running.**
   `~/.local/bin/cchv-distill` was a Jul-24 copy, so the windowing fix had never executed
   while every checkbox and CI badge said it had. And the rule is about the **install
   boundary**, so when a deploy gap is found on one task, re-check the whole section on
   the far side of it — not the task that surfaced it. *Evidence:* #40.

3. **A probe is a property of the RELEASE, not of the service.** `POST /v1/journal/ticks`
   405 → 401 proved v0.20.0 and is *vacuous* for v0.20.1, where the route already exists
   and answers 401 on both sides — that release had to be proven by its migration. Two
   prior forms were also wrong: `cchv-deploy`'s documented "404 → non-404" passes on the
   old binary (the `static_dir` fallback answers 405 to a POST on any path), and its
   replacement `405 → 2xx` is unreachable because the route is authenticated.
   *Evidence:* CONTEXT `cchv-deploy` SKILL.md § the swap-proof probe.

4. **"CI green" needs the system *and* the sha named.** Wrong twice in one day: once
   scoped to GitHub Actions while infra checked Gitea (which has no CI at all —
   `total_count=0` means *not wired*, not *unknown*), once scoped to `server-release.yml`
   while `Security Audit` was red on the **tag's** sha. *Evidence:* AGENTS.md § Phase 4,
   now `gh run list --commit <tag-sha>` across all workflows as part of **cutting**.

5. **A non-zero exit carries "the answer is no" and "I could not answer" on the same
   wire.** `git merge-base --is-ancestor` exits 1 for "not an ancestor" and **128** for
   "that object does not exist"; `||`, `&&`, `set -e` and `2>/dev/null` collapse them. So
   a **fabricated** sha verifies as a confident "not published" — which is exactly what
   happened, on *both* sides of the relay. *Evidence:* AGENTS.md publication proof, now
   reading `rc` by `case`.

6. **Never write a sha you have not printed in the command that produced it.** `b7d1a1a4`
   was published to a closed issue and to infra's `m4m.md`; it resolves to nothing. The
   commit was `2a318594`. This is a different failure from misapplying a real sha (which
   also happened, `c31d31bd` vs `b073deba`) and worse: a wrong reference fails loudly when
   followed, a fabricated one resolves to nothing. Pairs with rule 5 — one prevents the
   input, the other its laundering, and we had one each while it still crossed.

7. **A known trap is a hypothesis, not an explanation.** A grep returned 0 and the
   documented BSD-grep `\|` trap was reached for; the control showed `\|` → 0 *and*
   `-E` → 0 — the string was simply line-wrapped. A catalogue of failure modes makes it
   easier to pattern-match a symptom onto the wrong one, and then supplies false
   *confidence* rather than false ignorance. Same family: "the timing proves day-slicing
   works" — it didn't; the installed script had no window at all.

8. **Before relaying a deploy, predict what the health endpoints will SAY, not just that
   they will answer.** v0.20.0 swapped in seconds with no downtime to report, and then
   `/v1/healthz/journal` went 200 → 503 because the release changed the predicate the
   check evaluates. The relay omitted it, and infra had to decide mid-deploy whether a
   503 was their swap or our code. *Evidence:* CONTEXT `cchv-deploy` § expected non-200.

9. **Row counts in this archive are *records*, not conversation — 91.1 % of
   `claude`-provider rows have NULL content.** They are sidecar state records
   (`permission-mode`, `agent-color`, `worktree-state`, …). Every "N messages" figure,
   including ones in this repo's own docs, overstates conversation by ~11×. It is also why
   ~50 % of the backfill legitimately skipped. *Evidence:* #41.

10. **The test harness is not exempt from the bug class the tests exist to catch.** Two
    tests written today lied rather than failed: `seed()` keyed messages by array index so
    a second seed collided and upserted to a no-op (the test observed "no new data" while
    believing it had ingested some), and `pending_for()` read absence off a possibly
    truncated 200-row page. Both are "a check that cannot fail" — the same shape as
    rules 3 and 4.

## Open at handover

| Issue | What | Blocked on |
|---|---|---|
| **#36** | fold queries seq-scan where the bitmap path is 2–3× faster (health 3.4 s → 1.6 s, pending 3.25 s → 1.08 s, both measured) | nothing — `status/ready` |
| **#39** | no `version` on `/v1/healthz`, so a hub swap can only be proven by route archaeology | nothing; infra explicitly handed it to us |
| **#40** | the installed distiller is a copy with no version signal | nothing; recommendation recorded (log version at tick start, **not** a symlink — this worktree is Syncthing-shared) |
| **#41** | 91 % of rows are state records | `needs/decision` — filter at query time vs at ingest is ac's call |
| **#11** | remaining `cargo audit` advisories | nothing; one entry lighter after today |

Infra's side, for reference: `ac/infra#98` (relay-supervisor 401 — **seven** messages died
today, including a correction and the correction to it; treat `relay-send` success as
enqueue, not delivery) and `ac/infra#116`.
