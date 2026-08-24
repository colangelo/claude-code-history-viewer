# 2026-08-23 — the hourly `:05`/`:10` excursion on `/v1/healthz/journal`

Infra flagged a recurring latency excursion on the endpoint Gatus pages on, and handed
the cause to this side of the line (thread `88e1fea6`, after the `cchv-v0.21.1` swap).
This note records **what has been ruled out and how**, so the next reader starts from
the surviving candidates rather than re-walking the whole surface.

**The thread is closed.** The mechanism is an hourly WAL-checkpoint storm on pg1 that
flushes up to 38 % of the instance-wide buffer cache in ~84 s (*The cause*, below), and the
producer is named: the `direction` database's client rewrites its whole document corpus
once an hour (*Attribution*, below — infra's live capture, thread `b1d5226a`). Both
falsifiers were answered, one on each side of the fence. Everything above those sections is
the elimination work that got there, and it is kept because the eliminations are what make
the answer trustworthy, not because the question is still open. **What binds now is in
*What cchv should and should not do about it*, and it is mostly "do not act".**

**Read the title as historical.** `:05`/`:10` is where *Gatus's poll phase* happened to
catch it. The event is a **`:04`–`:12` window** whose internal peak moves hour to hour;
two hours measured here peaked at `:05` and at `:09`. Anything keyed to `:05`
specifically will read as intermittent — see *Prospective test*.

## The reading — infra's, taken on the instrument that pages

Gatus, `mon` → m4m, 300 s cadence. **Nobody on this side reproduced these numbers**;
they are relayed, not inherited-as-fact, and the distinction matters because the whole
question is about a phase in the hour.

| window | n | baseline | excursion |
|---|---|---|---|
| pre-swap 12:30–15:25Z | 33 | 3.31–4.02 s | `:05`/`:10` pairs — 8.15/7.07, 8.43/6.94, 9.97/7.41 |
| post-swap 15:30–16:15Z | 10 | 2.06–2.52 s | **5.36 s** at 16:05:23Z, 2.87 s at 16:10:21Z |

`cchv-v0.21.1` (the `work_mem` fix, #36) took ~4.5 s off the peak and left the periodic
component intact. **Infra's post-swap excursion is n=1** — they say so themselves, and
every argument below was written inheriting that weakness. It is no longer the only
post-swap hour: the 17:05Z reading in *Result* is a second one, taken here.

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
This argument was written resting on infra's single post-swap excursion, asking for a
second post-swap hour to confirm or dissolve it. **That reading was then taken, below:**
16:05Z (infra, Gatus path) and 17:05Z (this session, mbm5 path) are two independent
post-swap hours at the same `:05` phase, so the restart did not re-phase it and the
class-wide exclusion stands. The two hours are from *different vantage points*, which is
what the phase argument needs — the same phase seen twice — and not what a magnitude
comparison would need.

**Exactly one hourly cycle exists in this repo's code**, found by sweeping the crates for
schedule constants: `sync-daemon/src/config.rs:36`, `default_scan_interval() = 3600` — the
safety-net rescan. Its phase is fixed and anchored to daemon start:
`lib.rs:177` re-arms `rescan_deadline` the moment the timer arm fires, before the pass
runs, so pass duration never drifts it. The only other hourly thing we own is the
distiller's launchd `StartInterval`, which infra already excluded by timing — the
excursion *precedes* the ~:13Z tick rather than coinciding with it.

## Surviving candidates, each with the reading that would kill it

1. **m4m's sync-daemon hourly rescan.** Ours. **Retired — see *What this closes on our
   side* below; it was settled from the code without needing the ssh reading this
   paragraph asks for.** Its phase is wherever m4m's daemon last
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

Probe window 16:22Z–17:16Z, script `/tmp/cchv-journal-phase-probe.sh`.

**The excursion reproduced from this second, independent vantage point, and the control
did not move.** Taken from ac-mbm5 by this session, 2026-08-23:

| sampled at | `/v1/healthz/journal` (fold) |
|---|---|
| 16:22:28Z | 2.406 s |
| 16:25:32Z | 2.437 s |
| 16:35:41Z | 2.509 s |
| 16:45:50Z | 2.445 s |
| 16:55:58Z | 2.404 s |
| 16:59:03Z | 2.394 s |
| 17:02:07Z | 2.350 s |
| **17:05:11Z** | **4.148 s** |
| 17:08:17Z | 3.479 s |
| 17:11:23Z | 2.567 s |
| 17:14:27Z | 2.579 s |

Control `/v1/healthz` across the same hour: **n=53, min 0.302 s, max 0.557 s, mean
0.389 s.** The control sample taken in the *same second* as the 4.148 s peak was
**0.302 s — the minimum of all 53**.

Three things follow.

1. **It is pg1-side work on the fold.** The control shares the network path, the TLS
   termination, the m4m process and the connection pool, and differs only in doing
   `SELECT 1` instead of the fold. It is flat through the excursion. m4m CPU, the tailnet
   and pool acquisition are all excluded.

2. **It is one event that decays, not a pair.** 4.148 → 3.479 → 2.567 → 2.579 across
   `:05`→`:14`. Infra's "recurring pair at `:05` and `:10`" is a single perturbation
   peaking around `:05` and relaxing to baseline over ~6 minutes; their `:10` poll catches
   the tail rather than a second event. Peak-then-decay is the signature of a cache working
   set being evicted and progressively re-warmed, not of a job running concurrently for ten
   minutes.

3. **This number is a floor, not a match.** Sampling the fold every 150 s keeps pg1's
   buffers warmer than Gatus's 300 s does, which shrinks the excursion. 4.15 s here against
   infra's 5.36 s at the same phase is consistent with that and is not a discrepancy to
   chase.

## What this closes on our side

- **The sync-daemon rescan is not it — on code, not on timing.** Both arms of the loop in
  `sync-daemon/src/lib.rs` break to the *same* `sync::run_once` call with the same
  arguments: a safety-net rescan does identical work to a watcher-triggered pass, and
  ac-mbm5 runs ~70 of those an hour (4,587 passes in 2.6 days). There is no "hourly burst"
  for the rescan to be. This retires candidate 1 above, which the ssh block had left open.
- **m4m ingests continuously right through the window.** `/v1/healthz/ingest` sampled every
  300 s for an hour (n=10) puts m4m's arrival lag at 0–66 s, including 10 s at 17:02Z and
  47 s at 17:07Z. No hourly gap, no hourly burst.

So cchv owns no hourly cycle that can produce this. The fold is the **victim** — the one
query expensive enough to be sensitive to the perturbation — and the perturbation is on
pg1.

## Handed back, with falsifiers

- An hourly job on pg1 firing around `:03`–`:05` (backup, snapshot, dump, ANALYZE, a
  container- or host-level timer). *Falsifier:* no timer in that phase means this is wrong.
  **Answered — it holds, and it is not a timer. See *The cause* below.**
- If the mechanism is eviction: `pg_stat_database` `blks_hit`/`blks_read` for the archive
  DB, sampled at `:04` and `:20`, should show the hit ratio dip at `:05` and recover. Flat
  means eviction is wrong and the VM-decay story (rule 3) is the next one to test.
  **Answered — the dip is measured, 99.7 % → 50.3 %. See *Falsifier 2 — CONFIRMED* below.**

Direct confirmation from this side would need pg1 credentials: `kv/infra/cchv/pg1` is 403
for ac's OIDC token, and broadening it is not this session's call. It was not needed:
infra took the readings on their own side of the fence, which is where they belong.

## The cause — an hourly WAL-checkpoint storm on pg1 (infra, thread `745a573f`)

Infra answered falsifier 1 from a week of `log_checkpoints` output already on disk in
`/var/log/postgresql` — no new instrumentation. **Relayed, not taken here**, per the same
rule that labels the Gatus numbers above; infra's write-up is
`docs/2026-08-23-pg1-hourly-wal-checkpoint-storm.md` (infra `0670ced` for the storm,
`76cf054` for the attribution and our falsifier 2).

Every hour, three to four **WAL-triggered** checkpoints (`checkpoint starting: wal` — the
`max_wal_size` threshold, not `checkpoint_timeout`) fire in a five-minute window. First one
of the hour, 22 consecutive hours, lands at `:04:33`–`:04:48`. A minute-of-hour histogram
over a full day is `:04`=16 `:05`=11 `:06`=6 `:07`=14 `:08`=14 `:09`=3 `:10`=1 `:12`=1,
and **zero outside `:04`–`:12`**.

`shared_buffers` on pg1 is **1,966,088 kB = 1.875 GB** (245,761 buffers). In the 17:00Z hour
one checkpoint wrote **92,825 buffers = 37.8 % of the entire instance buffer cache in 84 s**.
pg1 turns over 7–9 GB of WAL per hour — ~180 GB/day — inside that five-minute window, ~150×
the baseline rate; the timed checkpoints between bursts cover 10–47 MB.

> **Correction, 2026-08-24 (infra, thread `745a573f`).** This document published
> `shared_buffers` as `196608 × 8 kB = 1.5 GB`. The value read on pg1 is `1966088 kB`. Infra
> calls the smaller number our premise, and nothing on this side ever measured it — which is
> this repo's *say who took a reading* rule biting on a number small enough that nobody
> thought to ask. The stale divisor is **0.8×** the real one, so **every share-of-cache
> percentage first published here is 1.25× too large**; multiply an old figure by 0.8 for the
> corrected one (47.2 % → 37.8 %, 27.0 % → 21.6 %, 16.3 % → 13.0 %). Corrected in place
> below. Raw buffer counts, WAL volumes and every latency reading are unaffected, and so is
> the conclusion — one checkpoint still flushes better than a third of the instance cache.

So the eviction candidate has a named, measured cause. The architectural fact underneath it
is the one to carry forward: **the archive DB does not have a buffer cache of its own.**
`shared_buffers` is instance-wide, cchv shares that 1.875 GB with every other database on
pg1, and the fold's working set is large enough to be the thing that notices when a third of
it is flushed. **That is not fixable by sizing on this hardware** — see *The lever* below.

### Why the checkpoints are `wal` and not `time` — the WAL is saturated

The `checkpoint starting: wal` lines are not the 900 s timer firing early; they are
`max_wal_size` being hit. Infra's counters, 2026-08-24 (thread `745a573f`):

```
pg_stat_checkpointer   num_timed 22,353   num_requested 6,761   -> 23 % size-forced
max_wal_size           4096 MB            min_wal_size 80 MB
pg_wal on disk         4.1 GB             -> at the ceiling
checkpoint_timeout     900 s              checkpoint_completion_target 0.9
```

`num_requested` is the counter for checkpoints triggered by hitting `max_wal_size`, and it is
**23 % of all checkpoints on the instance**. So the burst is not a coincidence of the
15-minute timer at all: on roughly a quarter of cycles `direction`'s rewrite fills 4 GB of
WAL faster than `checkpoint_timeout`, each fill forces a checkpoint *immediately*, and a
forced checkpoint gets **none of the `completion_target 0.9` spreading** that softens a timed
one. Three to four of them in a `:04`–`:12` window is what a saturated 4 GB WAL looks like
from the outside.

This also retires a loose end in *The trap infra hit on our behalf* below. That section
concluded "the producer is a database *client* writing, not a scheduler firing on pg1", which
was right but left the 15-minute `checkpoint_timeout` as an unexplained near-coincidence.
There is no coincidence: the timer is not what fires.

### The lever — pg1-side, disk not RAM, and it is ac's call

Infra's own read, recorded because it is the reason the *do not tune the fold* rule below now
carries a date rather than being open-ended:

- **Doubling `max_wal_size` 4 GB → 8 GB is the cheap lever.** It converts size-forced
  checkpoints back into timed ones, which *do* get spread by `completion_target 0.9`.
  `/var/lib/postgresql` is 47 GB with 27 GB free (40 % used), so the 4 GB of headroom exists.
  `ALTER SYSTEM` + reload, no restart, reversible.
- **Raising `shared_buffers` is the option infra would not take.** 1.875 GB is already ~24 %
  of the box's 7.7 GiB total (4 vCPU), `free` shows 169 MiB genuinely free with 6.9 GiB in
  buff/cache, and the same box runs gitea, woodpecker, `direction` and gatus. There is no
  comfortable RAM to give the archive a bigger cache.
- **Not applied.** Prod tuning on infra's box is ac's decision, not a side effect of
  answering our question. Infra has put the numbers to them.

### The same-hour interlock — two instruments, neither aware of the other

Infra's checkpoint log and this session's probe both cover **17:00Z on 2026-08-23**. Lining
them up was retrospective, but the two datasets were collected independently and for
different reasons, so the alignment is evidence rather than fitting:

| time | pg1 checkpoint log (infra) | `/v1/healthz/journal` (here) |
|---|---|---|
| 17:02:07Z | — | 2.350 s |
| **17:04:43Z** | **start: wal** | |
| 17:05:11Z | *(writing)* | **4.148 s** |
| 17:06:22Z | complete — 53,135 buf, 21.6 % | |
| 17:06:36Z | start: wal | |
| 17:08:00Z | complete — 92,825 buf, **37.8 %** | |
| 17:08:10Z | start: wal | |
| 17:08:17Z | *(writing)* | 3.479 s |
| **17:09:47Z** | **complete — 32,047 buf, 13.0 % → burst ends** | |
| 17:11:23Z | — | 2.567 s |
| 17:14:27Z | — | 2.579 s |
| 17:23:10Z | start: **time** — 600 buf, 0.2 % | |

Every fold sample taken **inside** the burst window (17:04:43–17:09:47) is elevated; every
sample outside it is baseline. No exceptions in either direction. That also sharpens
*Result* point 2 above: the decay's end is not a free-running re-warm curve that happens to
flatten around `:11` — it is pinned to the moment the last burst checkpoint completes.

**What this interlock is worth, stated honestly: one hour, two elevated samples and three
baseline ones, aligned after the fact.** It is why run 2 of the probe was launched *before*
the 18:04Z burst as a prospective test — see below.

### Prospective test — the 18:00Z hour. **The prediction held.**

Stated *before* the burst, at 17:40Z: an elevated fold inside the `:04`–`:12` window,
relaxing after it, control flat throughout. A flat hour, or a peak outside the window,
falsifies the coupling. Script now landed as `scripts/journal-phase-probe.sh`.

| sampled at | `/v1/healthz/journal` | | |
|---|---|---|---|
| 17:50:27Z | 2.585 s | | |
| 17:53:31Z | 2.443 s | | baseline, five consecutive samples |
| 17:56:36Z | 2.606 s | | |
| 17:59:40Z | 2.599 s | | |
| 18:02:45Z | 2.675 s | | |
| **18:05:49Z** | **3.416 s** | ← | inside the burst window |
| **18:08:54Z** | **3.731 s** | ← | inside the burst window |
| 18:12:00Z | 2.818 s | | relaxing |
| 18:15:05Z | 2.581 s | | baseline |

Control `/v1/healthz` over the same span: **n=31, min 0.304 s, max 0.597 s, mean 0.376 s**,
flat through both elevated samples. So the second hour reproduces the first: elevation
confined to the burst window, control unmoved. This is no longer a retrospective alignment
— the window was named first and the readings landed in it.

Two things this hour adds that the 17:00Z one could not:

- **The within-burst shape varies; the window does not.** 17:00Z peaked at `:05` and 18:00Z
  at `:09`. That is expected — infra's histogram has three to four checkpoints spread over
  `:04`–`:12` and the largest flush is not always the first — but it means *"the excursion
  is at `:05`"* was always an artifact of Gatus's poll phase. **The claim to carry is
  `:04`–`:12`, not `:05`/`:10`.** Anything that keys on `:05` specifically will read as
  intermittent.
- **Peak magnitude varies hour to hour** (4.15 s, then 3.73 s) with the size of that hour's
  flush. Both under infra's 5.36 s, consistent with the 150 s-vs-300 s cadence note.

**One unexplained reading, recorded rather than smoothed away.** The first fold sample of
run 2 — 17:40:16Z, off-phase, 24 minutes after run 1 last touched the fold — cost **5.05 s**,
above both hours' in-burst peaks. About 0.7 s of that is fresh-process connection setup
(the first control of the run read 1.11 s against a 0.30–0.60 s steady state), leaving
~4.3 s unaccounted for. The obvious story — that magnitude tracks time-since-last-fold, so
Gatus at 300 s always reads a colder cache than this probe at 150 s — is **weakened by this
same run**: the 17:50Z sample, ten minutes after that one, was an ordinary 2.585 s. Infra's
live capture (`pg_waldump` over the burst's WAL segments is a large sequential read on pg1)
may have been running. It is one sample, it is not explained, and it is not evidence for
anything yet.

**Asked and not settled (2026-08-24).** Infra cannot say whether their capture perturbed it:
the WAL read ran in another session on their box and they do not have its exact window. So
this stays **unresolved** — specifically *not* "our instrument confirmed it was self-inflicted",
which is the comfortable reading and the one nobody measured.

### The trap infra hit on our behalf, and it is worth knowing

`/etc/cron.d/sysstat` on pg1 carries `5-55/10 * * * * root … debian-sa1` — a job at exactly
the phase we were hunting. **pg1 has no cron daemon at all**: `systemctl is-enabled cron` →
`not-found`, no `/usr/sbin/cron`, no process. An enumeration that reads `/etc/cron.d` and
stops there finds a perfect suspect that has never run, and would have closed this thread
on a false cause. Same shape as this repo's rule *"name the reading that would make it
FAIL"* — "there is a crontab entry in the right phase" has no failing reading attached to
it; "the daemon that would run it is installed and running" does.

Everything genuinely scheduled was enumerated and is in the wrong phase or the wrong period
(`pg-backup.timer` 02:30 daily, `pg-gin-clean` every 10 min, `checkpoint_timeout` 900 s and
logging `time` not `wal`, vzdump 02:30–03:10, one k8s CronJob daily at 04:00). **So the
producer is a database *client* writing, not a scheduler firing on pg1.**

### Attribution — CONFIRMED: `direction` rewrites its whole corpus, hourly

The lead below was a phase coincidence; it is now a live capture. Infra sampled
`pg_stat_database` and `pg_stat_activity` at 15 s across the 18:04Z burst (thread
`b1d5226a`, 17:32–18:18Z). **Relayed, not taken here** — pg1 credentials were never needed
on this side and none were requested. Deltas over 18:02:46Z → 18:09:03Z, whole instance:

| database | xact | ins | upd | del |
|---|---|---|---|---|
| **direction** | **+47,824** | **+561,161** | +47,780 | **+561,201** |
| cchv_archive | +301 | +6,094 | +625 | +12 |
| gitea | +1,648 | +48 | +89 | +0 |
| gatus | +234 | +486 | +222 | +627 |
| woodpecker / atuin / vikunja / postgres | ≤3 digits | | | |

Inserts ≈ deletes over six minutes, so it is not growth — the document corpus is **torn
down and rewritten once an hour**. `pg_stat_activity` names it directly: backends from the
`direction` app's egress node, user `direction`, running `INSERT INTO documents …` and
`DELETE FROM document_links WHERE source_id = $1` in a `BEGIN`/`COMMIT` loop, 47,824
transactions, continuously from 18:04:02Z. ~180 GB of WAL a day for a 2.2 GB database,
~80× amplification, paid by every tenant of the instance.

That also settles the earlier fingerprint argument in both directions: the loop is armed by
the `direction-api` process, which is why the phase jumped on both of 2026-08-22's pod
restarts (`:51:4x` → `:58:2x` → `:04:3x`) and did **not** jump on our 15:2xZ binary swap.
The producer was never on m4m.

The fix — incremental reconcile (upsert + delete-what-vanished) instead of a corpus
rewrite — is upstream of pg1 and goes to `direction` as a separate thread. Infra owns the
measurement; the pg1-side knobs are strictly worse. **Nothing here is cchv's to do.**

### Falsifier 2 — CONFIRMED, and the cost lands on our query

The prediction was: if the mechanism is eviction, the archive DB's hit ratio dips at the
burst and recovers. `cchv_archive` `blks_hit`/`blks_read`, per 15 s slice (infra, same
capture):

| slice | blks_read | blks_hit | hit ratio |
|---|---|---|---|
| 18:00:31Z | +474,655 | +1,450,252 | **75.3 %** ← `:00` fold poll, quiet instance |
| 18:01:01Z | +137 | +44,416 | 99.7 % |
| 18:02:46Z | +0 | +1,095 | 100 % |
| 18:05:32Z | +979,363 | +989,994 | **50.3 %** ← `:05` fold poll, mid-burst |
| 18:06:02Z | +385,134 | +914,703 | 70.4 % |
| 18:09:03Z | +383,456 | +915,771 | 70.5 % |

Baseline slices are 99.7–100 %; through the burst the archive DB runs 50–70 %. Sharper than
the dip: **the same fold read 474,655 blocks from outside `shared_buffers` at `:00` and
979,363 at `:05`** — same hour, ~5 min apart, a little over twice as many. With 1.875 GB of
`shared_buffers` for the whole instance and `direction` churning half a million rows through
it hourly (this hour's checkpoints wrote 23.8 %, 27.8 % and 41.5 % of it), our working set
is simply not resident when the `:05` poll arrives. **So eviction is measured, not
inferred.**

**Bookkeeping, in case it resurfaces:** infra's 2026-08-24 message on thread `745a573f`
still lists falsifier 2 as *"ours and still open"*. It is not — it was answered by infra's
own capture the previous evening on thread `b1d5226a` and written up in their repo at
`76cf054`. Two of their sessions, one unaware of the other; the answer is the table above,
and nobody needs to take the reading again.

Two honesty notes, neither of which moves the conclusion. It does **not** exclude the
VM-decay alternative (rule 3) — a checkpoint storm and an insert-triggered VACUUM on
`messages` can both be downstream of one write burst; the excursion simply no longer needs
it. And two independent pollers were live in that hour (Gatus at 300 s, our probe 2 at
150 s), so a 15 s slice can contain more than one fold: the hit-ratio collapse is robust to
that, the exact 2× block-read factor less so.

## What cchv should and should not do about it

- **Not our bug, and not our fix.** The fold is the victim; the producer is another
  database's client on a shared instance. Nothing in this repo can move the phase, the
  volume, or the eviction.
- **The 15 s Gatus timeout stays load-bearing.** The `work_mem` fix (#36, `cchv-v0.21.1`)
  bought headroom against a driver we do not control — pre-swap the excursion reached
  9.97 s, 0.03 s under the old 10 s default. Do not retune that timeout down on the strength
  of the post-swap numbers.
- **Do not tune the fold against this driver, and as of 2026-08-24 do not *spec* it either.**
  Insulating the fold from a cold cache (the index declined in #36, a materialised day fold,
  a smaller working set) is still a legitimate *engineering* question, but it is no longer
  motivated by this excursion: the excursion will move or vanish when `direction` is fixed.
  Infra's instruction is explicit and it has a gate — **do not spec a materialised day fold
  until ac has ruled on the `max_wal_size` change**, or we may build insulation against a
  driver that is about to get quieter. That ruling is the thing to check for before this
  bullet expires; it is not on a timer.
- **The pg1-side knobs are not symmetric, and the earlier "both worse than the upstream fix"
  is superseded.** `max_wal_size` 4 GB → 8 GB is now infra's *cheap lever* and is with ac;
  `shared_buffers` is the one they would not take. Detail and numbers: *The lever*, above.
- **If the peak changes magnitude over the coming days, that is `direction` changing, not a
  cchv regression.** Read it that way before opening anything.
- **Leaving the `:05` poll in place is the informative choice.** Dropping it would make the
  excursion disappear from our series honestly, but right now the fold is the only
  instrument on that box that notices the storm at all.
- **Falsifier 2 is answered — see *Falsifier 2 — CONFIRMED* above.** Both falsifiers this
  document handed back came home: one from a log already on disk, one from a live capture,
  neither needing a credential this side did not have. That is the pattern worth keeping —
  hand back a falsifier with the reading that would kill it, and let the side that owns the
  fence take it.
