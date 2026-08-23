# 2026-08-23 — the hourly `:05`/`:10` excursion on `/v1/healthz/journal`

Infra flagged a recurring latency excursion on the endpoint Gatus pages on, and handed
the cause to this side of the line (thread `88e1fea6`, after the `cchv-v0.21.1` swap).
This note records **what has been ruled out and how**, so the next reader starts from
the surviving candidates rather than re-walking the whole surface. It does not identify
the cause.

## The reading — infra's, taken on the instrument that pages

Gatus, `mon` → m4m, 300 s cadence. **Nobody on this side reproduced these numbers**;
they are relayed, not inherited-as-fact, and the distinction matters because the whole
question is about a phase in the hour.

| window | n | baseline | excursion |
|---|---|---|---|
| pre-swap 12:30–15:25Z | 33 | 3.31–4.02 s | `:05`/`:10` pairs — 8.15/7.07, 8.43/6.94, 9.97/7.41 |
| post-swap 15:30–16:15Z | 10 | 2.06–2.52 s | **5.36 s** at 16:05:23Z, 2.87 s at 16:10:21Z |

`cchv-v0.21.1` (the `work_mem` fix, #36) took ~4.5 s off the peak and left the periodic
component intact. **The post-swap excursion is n=1** — infra says so themselves, and
every argument below inherits that weakness.

Worth keeping from infra's account independently of the cause: 15:05:28Z measured
**9.97 s**, 0.03 s under the 10 s Gatus default retired in `2f23d7f`. The 15 s timeout
is still load-bearing.

## Ruled out on our side, with the evidence

**There is no cache in the journal read path.** `health.rs` contains no cache, no
`OnceCell`, no memo of any kind; `healthz_journal` opens a transaction
(`journal::begin_fold_tx`) and recomputes `SESSION_DAYS_CTE` live on every call. Infra's
first guess — a cache expiry — has nothing in this path to attach to.

**Both hub-internal background loops are 300 s, and a third is 30 s.** The embed sweeper
(`lib.rs:272`) and the DuckDB stats-mirror refresher (`config.rs:99`,
`default_refresh_secs() = 300`) both tick every 5 minutes; `db_watchdog::DEFAULT_INTERVAL`
is 30 s. A 300 s cycle cannot produce a signature that appears in 2 of 12 polls an hour —
it would land on every poll or on none.

**Stronger, and it rules out the whole class rather than these three: the excursion kept
its phase across the hub restart.** The binary swapped between the two series (~15:25–15:30Z),
so every hub-internal timer re-armed at process start and re-phased with it. The excursion
did not move — still `:05`/`:10` afterwards. Anything anchored to hub process start is
therefore excluded *whatever its period*, including a hypothetical hourly cache TTL.
This argument rests on the single post-swap excursion; a second hour of Gatus data either
confirms it or dissolves it.

**Exactly one hourly cycle exists in this repo's code**, found by sweeping the crates for
schedule constants: `sync-daemon/src/config.rs:36`, `default_scan_interval() = 3600` — the
safety-net rescan. Its phase is fixed and anchored to daemon start:
`lib.rs:177` re-arms `rescan_deadline` the moment the timer arm fires, before the pass
runs, so pass duration never drifts it. The only other hourly thing we own is the
distiller's launchd `StartInterval`, which infra already excluded by timing — the
excursion *precedes* the ~:13Z tick rather than coinciding with it.

## Surviving candidates, each with the reading that would kill it

1. **m4m's sync-daemon hourly rescan.** Ours. Its phase is wherever m4m's daemon last
   started, which nobody here can see — peer-Mac ssh routes through the 1Password agent
   and prompts, so an unattended session cannot take this reading.
   *Falsifier:* `grep 'sync pass complete' /tmp/cchv-daemon.log` on m4m. If the passes are
   near-continuous rather than clustered at a fixed minute, our daemon is not it. On
   ac-mbm5 they **are** near-continuous — 4,587 passes in 2.6 days, one every ~40 s,
   because the file watcher keeps firing on an actively used Mac — and this machine's
   rescan phase is `:32` (daemon start 2026-08-21T00:32:01Z), not `:05`. So mbm5 is
   excluded, and m4m is only a candidate if its watcher is quiet enough for the rescan
   to stand out.

2. **A pg1-side periodic — infra's half of the line, despite where the flag was thrown.**
   The endpoint's cost is dominated by pg1 work (the fold over ~2.4 M rows), and this
   repo has already measured the mechanism that would convert *any* hourly insert burst or
   buffer eviction into exactly this shape: `messages` is append-only, so VACUUM there is
   insert-triggered and visibility-map coverage over the hot range decays between runs —
   the share of a 7-day window needing `Heap Fetches` swings **0 % → ~60 %**
   (`docs/2026-08-21-identity-surfaces-and-query-floor.md` rule 3).
   *Falsifier:* sample `n_ins_since_vacuum` and `relallvisible`/`relpages` on `messages` at
   `:04` and again at `:20`. Flat across the pair means the VM-decay story is wrong.

That second candidate is exactly the trap rule 5 of that same document warns about — the
catalogue makes a plausible story *cheaper to reach for*, not likelier to be true. It is
listed as a candidate carrying its own falsifier, not as a diagnosis.

## The differential probe, and what it can settle

Run from ac-mbm5 across the 17:00Z window, sampling two unauthenticated endpoints:

- **control** `/v1/healthz` — `SELECT 1` on pg1 (`health.rs:34`). Same network path, same
  pool, none of the fold.
- **treatment** `/v1/healthz/journal` — the fold.

Both rising together points at m4m or the network/connection; only the treatment rising
points at pg1-side work on the fold. Two design notes, both from this repo's own rules:
`--max-time` is set to 30 s, far above any observed value, because a ceiling *below* the
query time abandons the request client-side while the server keeps running it — the
self-inflicted-backlog mistake in AGENTS.md. And the treatment is sampled every 150 s, not
every 60 s, because sampling the fold each minute would keep pg1's buffer cache warm over
the hot range and could mask the excursion being measured.

Caveat on the vantage point: this is mbm5 → m4m, not the Gatus path, so it cannot confirm
or deny infra's numbers. It is a **differential** — control against treatment in the same
minute from the same host — which is what makes it readable from the wrong vantage point
at all. The baseline agrees (2.41–2.48 s here against infra's 2.06–2.52 s), which is
reassurance, not proof.

### Result

Probe window 16:22Z–17:22Z, script `/tmp/cchv-journal-phase-probe.sh`.

<!-- RESULT -->
