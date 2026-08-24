# 2026-08-23 — the hourly `:05`/`:10` excursion on `/v1/healthz/journal`

Infra flagged a recurring latency excursion on the endpoint Gatus pages on, and handed
the cause to this side of the line (thread `88e1fea6`, after the `cchv-v0.21.1` swap).
This note records **what has been ruled out and how**, so the next reader starts from
the surviving candidates rather than re-walking the whole surface.

**The thread is closed.** The mechanism is an hourly WAL-checkpoint storm on pg1 that
flushes up to 47 % of the instance-wide buffer cache in ~84 s (*The cause*, below), and the
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

**And as of 2026-08-24T11:20Z the window is historical too.** `max_wal_size` 4 GB → 8 GB
doubled the WAL distance each forced checkpoint has to accumulate, so it takes twice as long
to arm: the first post-change hour's forced checkpoint started at **`:13:15`** and completed
**`:17:04`**, outside `:04`–`:12` entirely. **Anything still keyed to `:04`–`:12` now reads
as "the storm stopped" and means nothing by it** — including a probe whose dense window
brackets the old phase. See *Post-change reading*, below.

**And as of 2026-08-24 ~12:00Z the storm itself is over — there is no window at all.**
`direction-prod` shipped its incremental-reconcile fix (0.99.3), the producer stopped
writing, and pg1 measured **zero** forced checkpoints across four consecutive hours against
a timed positive control still at 4/hour. **Both windows — `:04`–`:12` and `:13`–`:17` —
now match nothing, correctly.** Re-running any window-keyed grep or probe against the
checkpoint log from here on measures *absence*, not phase. See *The storm is over*, below.

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

`shared_buffers` on pg1 is `196608 × 8 kB` = **1.5 GB** (1536 MB, 196,608 buffers). In the
17:00Z hour one checkpoint wrote **92,825 buffers = 47.2 % of the entire instance buffer
cache in 84 s**. pg1 turns over 7–9 GB of WAL per hour — ~180 GB/day — inside that
five-minute window, ~150× the baseline rate; the timed checkpoints between bursts cover
10–47 MB.

> **Correction withdrawn, 2026-08-24 (infra, thread `745a573f`) — the figure above is the
> original one and it is right.** For about two hours this document carried
> `shared_buffers = 1,966,088 kB = 1.875 GB` and every share-of-cache percentage scaled by
> 0.8×, on infra's correction. **Infra withdrew it the same day: the value is 1536 MB.**
>
> Their instrument was `SELECT setting || coalesce(unit,'')`, which for a setting whose unit
> is `8kB` **glues the number to the unit** and yields a well-formed kB figure that answers a
> different question: `setting=196608`, `unit=8kB` concatenates to `1966088kB`, read as
> 1.875 GB. The real value is `196608 × 8 kB = 1536 MB`. **Every 8kB-unit setting relayed on
> that thread is wrong the same way** — `effective_cache_size` is **4096 MB** (not
> `5242888kB`) and `wal_buffers` is **16 MB** (not `20488kB`).
>
> **The blast radius has a sharp edge, and it is the reason this one landed** (infra,
> 2026-08-24, thread `8d5eb1ba`): the probe corrupted **every 8kB-unit row and no others**,
> and those three rows — `shared_buffers`, `effective_cache_size`, `wal_buffers` — are exactly
> the ones whose corrupted form still reads as an ordinary kB figure. So it was **wrong for
> every row it touched and visibly wrong for none**. A per-row plausibility check cannot find
> this class; only re-deriving from `setting` and `unit` can. (Infra also reconstructed a
> *cause* for the 1.875 GB before finding the real one — "25 % of a 7.5 GiB VM, read as a
> measurement" — which fits to within rounding. A coincidence that good is what makes a wrong
> number survive review.)
>
> The `MB`/`kB`/`s` rows were unaffected — the concatenation happened to reproduce their own
> true strings — so `max_wal_size 4096MB` (the pre-change value; raised to 8 GB later the
> same day — *The lever*), `min_wal_size 80MB`, `work_mem 4096kB`,
> `maintenance_work_mem 65536kB`, `checkpoint_timeout 900s` and
> `checkpoint_completion_target 0.9` all stand, as do the checkpoint counters below (a
> different query).
>
> **Read a pg setting as `pg_size_pretty(setting::bigint * 8192)` when its `unit` is `8kB`**,
> and re-derive rather than transcribe any relayed number that is load-bearing for a change.
>
> The episode is worth more than the number. This is the `--is-ancestor` family — an
> instrument returning a *plausible* value instead of an error — with one new edge: it did not
> merely produce a wrong answer, it **overturned a correct one**. The withdrawn correction
> was accepted here in ~20 minutes on the strength of this repo's own *say who took a reading*
> rule, which cut the wrong way: "we never measured it, they did" is not the same as "their
> reading is sound", and a peer's number is a reading that needs a source too. Where a
> correction contradicts a figure we *derived* (196608 × 8 kB is arithmetic, not a guess),
> ask which query produced theirs before editing ours.

So the eviction candidate has a named, measured cause. The architectural fact underneath it
is the one to carry forward: **the archive DB does not have a buffer cache of its own.**
`shared_buffers` is instance-wide, cchv shares that 1.5 GB with every other database on
pg1, and the fold's working set is large enough to be the thing that notices when half of
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

### The lever — pg1-side, disk not RAM. **`max_wal_size` 4 GB → 8 GB is APPLIED**

- **Doubling `max_wal_size` 4 GB → 8 GB was the cheap lever, and ac approved it.** It converts
  size-forced checkpoints back into timed ones, which *do* get spread by
  `completion_target 0.9`. `/var/lib/postgresql` is 47 GB with 27 GB free (40 % used), so the
  4 GB of headroom exists. **Applied by infra 2026-08-24T11:20:07Z** — `ALTER SYSTEM` +
  `pg_reload_conf()`, **no restart** (`context=sighup`, `pending_restart=false`, postmaster
  uptime unbroken at 1d23h); all five tenant DBs reachable afterwards and every pg1-dependent
  Gatus check green. The log caught the storm minutes before the reload — three `checkpoint
  starting: wal` at 11:06:02 / 11:07:46 / 11:09:14, each carrying `distance ~2.19 GB`
  (`estimate 2196115 kB`), i.e. the checkpointer had settled on cycling every ~2.2 GB of WAL
  against a 4 GB ceiling.
- **No stats reset, deliberately.** `pg_stat_checkpointer.stats_reset` still reads
  2026-05-10, so the 22,353 / 6,761 baseline above is intact and the effect is measurable as
  a **delta** rather than needing a clean slate. (This side's standing `pg_stat_reset()`
  warning applies to pg1 too; infra did not repeat that suggestion.)
- **Raising `shared_buffers` is the option infra would not take, and did not.** 1536 MB is
  ~19 % of the box's 7.7 GiB total (4 vCPU), `free` shows 169 MiB genuinely free with 6.9 GiB
  in buff/cache, and the same box runs gitea, woodpecker, `direction` and gatus. There is no
  comfortable RAM to give the archive a bigger cache — so *the archive DB has no cache of its
  own* is the standing fact, un-contradicted.

**A post-change measurement is not comparable to the pre-change series above.** The driver
just got quieter. If the phase probe is re-run, state the prediction first, which is infra's:
the `:04`–`:12` excursions become **less frequent** (fewer size-forced checkpoints) rather
than smaller when they do occur. If they vanish entirely, that is the coupling confirmed a
third way. Give the new setting a few hours before reading anything into either.

### Post-change reading — 4 forced checkpoints/hour became 1, and **the phase moved**

Infra took the first-hour reading at the **checkpoint layer**, not through Gatus, and handed
it over on thread `8d5eb1ba` (measured 12:17–12:27Z, infra `b8ba11a`). **Relayed, not taken
here** — same fence as every other pg1 number in this document.

| | pre-change (4 GB) | post-change (8 GB) |
|---|---|---|
| forced checkpoints | **4 per hour, every hour of the day** | **1** (`num_requested` 6,761 → 6,762 across the hour) |
| distance per forced checkpoint | 2,196,929 kB | **4,405,533 kB — exactly double** |
| phase | first one of the hour at `:04:33`–`:04:48` | started **`:13:15`**, completed **`:17:04`** |

The doubling of the distance *is* the mechanism: the trigger is a WAL distance derived from
`max_wal_size`. So infra's prediction holds in the form it was recorded above — **less
frequent, and bigger; not smaller.**

Three things that bind:

- **The phase moved, because accumulating twice the distance takes twice as long.** The
  forced checkpoint no longer lands in `:04`–`:12`. Every window-keyed artefact on either
  side is now wrong in the *reassuring* direction: it sees nothing and reads as "the storm
  stopped". Infra has flagged the grep in their own reproduce block in place. On this side
  the affected knob is `scripts/journal-phase-probe.sh`'s `DENSE_FROM`/`DENSE_TO` — the
  header there now records the new phase, and running the probe **dense across the full
  hour** (`DENSE_FROM=0 DENSE_TO=59`) is the safe default while the phase is still settling.
- **The workload did not change — pg1's response to it did.** Ten minutes after that
  checkpoint completed, 4,057 MB of WAL had re-accumulated without tripping the next trigger,
  so the hour still turned over ~8.5 GB, inside the 7–9 GB/hour band measured at 4 GB.
  `direction-prod` is still v0.99.1.
- **4 → 1 therefore proves *nothing* about `direction`'s fix.** It is precisely what doubling
  the WAL trigger does on its own. **The surviving discriminator is unchanged: zero forced
  checkpoints across a non-first post-deploy hour.** Anyone reading a quiet checkpoint log as
  evidence that the corpus rewrite was fixed is reading the lever, not the driver.
  **That discriminator was met four hours running on the same day — *The storm is over*,
  below.** Note what stating it in advance bought: the lever and the fix landed within an
  hour of each other, and nobody ever had to untangle them.

This is the *"a phase read through a poller is the poller's phase"* rule with a second
edge on it: **a phase is also a property of the driver's current trigger, so changing the
trigger moves the phase.** A window derived under one setting has no authority under the
next one, and it fails silently — the check still runs, still passes, and now measures
nothing.

#### The Gatus day is infra's to take — and it has four hours that must be excluded

Infra will run the clean-day Gatus series and hand it over rather than have this side
duplicate it. Not a verdict yet, and infra say so themselves: the 12:00Z hour peaked at
6.126 s at 12:14:17Z (inside the *new* checkpoint window) against 4.684 s the hour before,
and 5.808 s at 12:04:17Z where `direction`'s job starts — but that is **300 s sampling
against a ~4 minute excursion, one sample per event**, and the quiet baseline drifted
3.089–4.634 s on its own across the two preceding hours. Exactly the Nyquist problem this
document already carries; do not read a verdict out of it.

**The caveat that binds any reader of that series: `03:00–07:00Z` on 2026-08-24 is all
`15.001 s` — the check's own timeout**, from a hub-side condition unrelated to the
checkpoint storm. A day-long aggregate that includes those four hours measures the timeout,
not the fold. Any clean-day number infra sends will name the hours it excludes.

**That timeout window is *ours*, and it is not explained.** It is the cchv hub, not pg1. It
is not live — a single probe from mbm5 at 12:33Z read `/v1/healthz/journal` **200 in 2.286 s**
and `/v1/healthz` **200 in 0.086 s**, ordinary baseline. So this is a four-hour historical
excursion of a different shape (saturating a 15 s ceiling, not a ~4 minute checkpoint dip),
and nothing here has diagnosed it. Do not fold it into the storm story; it is a separate
open question, and the first thing it needs is infra's per-check series for those hours.

## The storm is over — the producer was fixed, and it is not the lever that did it

Infra, thread `8d5eb1ba`, measured on pg1 15:59–17:28Z on 2026-08-24; their write-up is
`docs/2026-08-23-pg1-hourly-wal-checkpoint-storm.md` § 4, commit `9999316`. **Relayed, not
taken here** — same fence as every other pg1 number in this document. `direction-prod`
answers `/api/version` → `{"api":"0.99.3","mcp":"0.99.3"}`, so the incremental-reconcile fix
named in *Attribution* is deployed.

| hour UTC (2026-08-24) | forced checkpoints (`wal`) | timed (`time`) — positive control |
|---|---|---|
| 00:00–11:00 | 4 every hour, no exceptions | 4/hour |
| 12:00 | 1 (at 12:13:15) | 4 |
| 13:00, 14:00, 15:00, 16:00 | **0** | **4** each |

```
WAL generation   7–9 GB/hour  ->  0.03 GB/hour
distance/ckpt    ~2.19 GB     ->  2.7–29.2 MB, timed only
direction churn  561k rows/hour -> ins +0 / del +0 / upd +0
```

**Three method notes, because the discriminator was written down first and that is what
makes this readable.**

- **The falsifier was declared BEFORE `max_wal_size` doubled** — *"only zero is unambiguous;
  1–2/hour proves nothing, that is what the doubled trigger does on its own"* (this
  document's own *Post-change reading*, third bullet). A doubled trigger **cannot** reach
  zero against a live 7–9 GB/hour write: 8 GB would trip about every 55 minutes. So the
  confound between lever and fix never had to be untangled — the reading falls outside the
  range the lever can produce.
- **The zero is a real zero, not a dead log.** `checkpoint starting: time` still ran 4/hour
  through 16:00Z and the log's last line was written 17:15:12Z. A silent log and a silent
  storm are identical in a `grep -c`; the timed line is the positive control that separates
  them.
- **The 90 s tuple delta is corroboration, not proof, and infra flagged it themselves.** The
  offending job ran *hourly*, so a 90 s window would show `ins +0 / del +0` even if it were
  still running. The four-hour checkpoint history and the WAL rate carry the verdict; the
  tuple zero rides along. Same discipline as the poller-folding challenge that produced
  *"a phase read through a poller is the poller's phase"*.

**What it means for the fold.** The eviction pressure the day-fold was competing with is
gone **at the source**, not absorbed by a bigger buffer. The 474,655 → 979,363 block-read
factor in *Falsifier 2* had a producer, and the producer stopped writing. A materialised day
fold is therefore now a decision about **our own workload**, with no pg1-side confound to
design around — and with a smaller measured problem than the one that motivated it.

**And the setting the gate was about is now inert.** At 0.03 GB/hour, 8 GB of `max_wal_size`
is ~11 days of WAL rather than ~55 minutes. ac ruled 2026-08-24 to **keep it at 8 GB, no
revert** (infra#126 closed), so this side's hold is discharged — but infra's own description
is the honest one and worth preserving: *"keeping it costs nothing and buys nothing; it
stopped being urgent rather than being answered on the merits."* Do not carry 8 GB forward
as a tuning win.

### First-hand: what the fold does on a quiet instance

Everything above is pg1's checkpoint layer, relayed. The reading this side owns is the one
this document has always taken — the fold's own latency — and it was re-taken after the fix
landed, **from m4m loopback this time** rather than from ac-mbm5, so the network path is out
of it and the absolute numbers are not comparable to the mbm5 series above (a spot control
read 0.083 s against 0.30–0.60 s there).

Probe: `scripts/journal-phase-probe.sh`, dense across the whole hour (`DENSE_FROM=0
DENSE_TO=59`), 17:33Z–18:21Z on 2026-08-24 — bracketing **both** retired windows, `:04`–`:12`
and `:13`–`:17`, because a run that brackets only one of them cannot tell absence from a
phase move. Result: *see the table below.*

### The same-hour interlock — two instruments, neither aware of the other

Infra's checkpoint log and this session's probe both cover **17:00Z on 2026-08-23**. Lining
them up was retrospective, but the two datasets were collected independently and for
different reasons, so the alignment is evidence rather than fitting:

| time | pg1 checkpoint log (infra) | `/v1/healthz/journal` (here) |
|---|---|---|
| 17:02:07Z | — | 2.350 s |
| **17:04:43Z** | **start: wal** | |
| 17:05:11Z | *(writing)* | **4.148 s** |
| 17:06:22Z | complete — 53,135 buf, 27.0 % | |
| 17:06:36Z | start: wal | |
| 17:08:00Z | complete — 92,825 buf, **47.2 %** | |
| 17:08:10Z | start: wal | |
| 17:08:17Z | *(writing)* | 3.479 s |
| **17:09:47Z** | **complete — 32,047 buf, 16.3 % → burst ends** | |
| 17:11:23Z | — | 2.567 s |
| 17:14:27Z | — | 2.579 s |
| 17:23:10Z | start: **time** — 600 buf, 0.3 % | |

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
979,363 at `:05`** — same hour, ~5 min apart, a little over twice as many. With 1.5 GB of
`shared_buffers` for the whole instance and `direction` churning half a million rows through
it hourly (this hour's checkpoints wrote 29.7 %, 34.8 % and 51.9 % of it), our working set
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
- **Do not tune the fold against this driver — and as of 2026-08-24 there is no driver.**
  Insulating the fold from a cold cache (the index declined in #36, a materialised day fold,
  a smaller working set) was already only an *engineering* question rather than an incident
  response; now the excursion is gone at the source (*The storm is over*). Any such work
  must be justified by the fold's cost on a **quiet** instance, measured after 2026-08-24,
  not by the numbers in this document.
- **The gate on *speccing* a day fold is FULLY DISCHARGED — both halves.** For one day the
  hold was "do not spec a materialised day fold until ac has ruled on the `max_wal_size`
  change"; ac ruled 2026-08-24 (**keep 8 GB, no revert**, infra#126 closed). The weaker
  empirical replacement — *wait for a few hours of post-change readings, because the size of
  the problem insulation would solve is exactly what just changed* — is satisfied too: four
  post-fix hours at **zero** forced checkpoints, `direction`'s churn at ins +0 / del +0.
  Nothing is being waited on. What that leaves is not permission to build it but a **much
  smaller problem**: re-measure the fold on the quiet instance first, and let that number
  decide.
- **A window-keyed check is now a check that cannot fail — and re-running one measures
  absence, not phase.** `:04`–`:12` was derived under `max_wal_size = 4 GB`; the trigger
  moved it to `:13`–`:17` on 2026-08-24T11:20Z; the producer then stopped altogether around
  12:00Z and **both** windows now match nothing. So a grep, probe window or alert keyed to
  either one is silent for three different reasons that look identical from outside. Before
  trusting any window in this document, confirm it was re-derived against a *current* phase
  — and if there is no current phase, say so rather than reporting the silence. Repo rule it
  instantiates: *name the reading that would make it FAIL*.
- **The `03:00–07:00Z` 15.001 s block on 2026-08-24 is a hub-side open question, not part of
  this thread.** Four hours of the Gatus check hitting its own timeout, on our side of the
  fence, currently not reproducing (2.286 s at 12:33Z). It must be excluded from any
  aggregate of that day, and it deserves its own look — see *The Gatus day*, above.
- **The pg1-side knobs are not symmetric, and the earlier "both worse than the upstream fix"
  is superseded.** `max_wal_size` 4 GB → 8 GB was infra's *cheap lever* and is **applied**;
  `shared_buffers` is the one they would not take. Detail and numbers: *The lever*, above.
  **The lever is now inert** — 8 GB is ~11 days of WAL at the post-fix rate — and it is kept
  because reverting costs a change, not because it buys anything.
- **If the peak changes magnitude over the coming days, that is `direction` changing, not a
  cchv regression.** Read it that way before opening anything. As of 2026-08-24 the expected
  peak is **no peak**; an hourly excursion *reappearing* is `direction` regressing, and the
  first thing to check is `/api/version` on `direction-prod` against 0.99.3.
- **Leaving the `:05` poll in place is the informative choice.** Dropping it would make the
  excursion disappear from our series honestly, and the fold was for a while the only
  instrument on that box that noticed the storm at all. That argument is now dormant rather
  than wrong: with the producer fixed the poll measures our own fold, which is what it was
  always for.
- **Falsifier 2 is answered — see *Falsifier 2 — CONFIRMED* above.** Both falsifiers this
  document handed back came home: one from a log already on disk, one from a live capture,
  neither needing a credential this side did not have. That is the pattern worth keeping —
  hand back a falsifier with the reading that would kill it, and let the side that owns the
  fence take it.
