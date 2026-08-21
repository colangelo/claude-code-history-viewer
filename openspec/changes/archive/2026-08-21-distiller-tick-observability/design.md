# distiller-tick-observability — Design

## Context

`healthz_journal` answers one question — *is there closed-day work nobody has
done?* — and the answer is a function of archive state alone. That is why it
catches every stall mode, including a run that succeeds and distills nothing
(the DST-race bug `distiller-self-healing` fixed). It is also why it cannot see
the distiller at all: an **idle tick leaves no trace anywhere in the hub**. The
tick's only hub interaction is `GET /v1/journal/pending`, and a GET writes
nothing.

`deployment.md` §3c has been arguing from that strength: *"This catches all stall
modes … which a distiller-side dead-man ping cannot."* True, and it does not
follow that a ping adds nothing. The two answer different questions and neither
subsumes the other:

| | tick recent | no recent tick |
|---|---|---|
| **stale groups** | running, not draining — a real stall | scheduler/host problem |
| **no stale groups** | healthy | healthy but unattended; nothing to do yet |

The top-left and top-right cells are the same 503 today. That ambiguity is what
let two careful derivations in a row name a clear-by time and miss it.

## Goals / Non-Goals

**Goals.** Make "has a distiller run recently?" answerable from the same
unauthenticated endpoint a monitor already polls. Keep the existing verdict
byte-identical for callers that do not opt in. Correct the cadence claims the
measurement refuted.

**Non-Goals.** Scheduling changes: nothing here makes the job fire while the host
sleeps, and nothing should — a `PowerNap`/`caffeinate` argument is a host
decision, not a hub one. Nor a completion record (see D3), nor per-group tick
attribution.

## Decisions

### D1: A heartbeat the distiller sends, not an inference from hub data

The alternative is to derive last-tick from data already present —
`max(journal_entries.generated_at)`. Rejected: that timestamp only moves when a
tick had work **and succeeded at it**, so a distiller that ticks every hour into
an empty work list is indistinguishable from one that never ran. That is
precisely the ambiguity being fixed; deriving it from entry timestamps
reproduces it one level down.

Also rejected: recording the tick as a side effect of `GET /v1/journal/pending`.
It needs no distiller change, and it is wrong twice — a GET that writes, and a
signal any operator's `curl` (or the webapp, or a probe) silently falsifies. The
heartbeat has to be a deliberate statement by the job, from the job.

### D2: Report the age; alert only when asked

`max_tick_age_secs` is absent by default and then never contributes to the
verdict. This is the `max_lag_rows` rule from `/v1/healthz/stats`, and it applies
here with more force: infra's measurement is that **this check cannot be given a
wall-clock deadline on a host that sleeps**. A default of, say, 2 h would be a
claim about ac's wake schedule — 40–106 sleep cycles a day — wearing a health
rule's clothes, and it would flap nightly. Keeping the policy in the Gatus check
also means tuning it never needs a hub redeploy, the same reasoning as
`healthz_ingest`'s `exclude`.

The suggested alternative from the relay — *page on "no distiller tick in N
wakes"* — is the right instinct expressed in the wrong place. The hub has no
notion of the host's wakes and should not grow one: it is not on that machine in
principle, and a wake count is host telemetry, not archive state. Reporting
`ticks_last_24h` gives a monitor the same discrimination — a host that got two
chances to tick shows 2 — without the hub knowing anything about power
management.

That is a discrimination *equivalent* to a wake count, not an equality with one,
and the difference is measurable: infra's 13-day sample (2026-08-21, corrected
same day from 194 completions to 204 starts) puts this host at 15.38 ticks/day
while it turns over 40–106 sleep cycles a day. The gap is **not** that DarkWakes
run no `StartInterval` agent — they do, and the claim that they do not was
retracted the same day; it is that the interval re-arms at exit (capping an awake
day near 21.7) and that sleep coalesces missed intervals into one catch-up.
Placement was the right call for a second reason we did not have at the time:
"wake" has no crisp definition on this host — DarkWake, full wake and
thermal-emergency wake all behave differently — so a hub-side wake counter would
have had to pick one, and would have picked wrong.

Status precedence when the param **is** set: `no_tick` outranks `stale`, because
when both fire the tick is the cause and the stale groups are the symptom.

### D3: Record at start-of-work, not at completion

The POST goes immediately after the pending query returns, before any LLM call.
It therefore asserts exactly one thing: *a distiller process reached the hub and
obtained a work list.* It does **not** assert that the work got done.

That is the right assertion, because "work didn't get done" is already the
question `stale` answers. Pairing a start-of-work heartbeat with the existing
stale verdict fills in the table above; adding a completion record too would
distinguish a crash-looping distiller from a healthy one, which `ThrottleInterval
300` plus the stale verdict already covers between them. One POST per tick, and
the field names say `tick`, not `run`, so the limit is legible in the response.

A `--dry-run` records nothing: that flag's whole contract is that it never POSTs,
and a dry run drains nothing, so counting it as a tick would overstate liveness.
A `--backfill` **does** record — it genuinely drains work — and carries
`mode: "backfill"` so a reader can tell it from a scheduled forward tick.

### D4: A small append-only log, not a single upserted row

One row per tick, pruned to 30 days on insert. A single-row table would answer
`last_tick_at` and nothing else; the log also answers `ticks_last_24h`, which is
the number that makes the wake-driven cadence *visible* rather than inferred —
and it is the measurement infra could only get by watching the log file
continuously for an hour. At ~24 rows/day the table is ~720 rows steady-state;
the prune is a single indexed delete on the same statement's heels, and is
deliberately cheap rather than a separate job to forget about.

### D5: The day-close property gets a scenario, not just a comment

`grace_secs` counts from `latest_arrival`, so a group whose data landed more than
`grace_secs` before its day closed is stale the moment the day closes. This is
correct behaviour — the work really is undone and really is overdue — but it has
a counter-intuitive consequence worth pinning: **a day-close can only ever add
stale groups, never remove them on net**, so the endpoint has no wall-clock
recovery time. Twice now that has been re-derived wrongly from the other
direction. A scenario and an integration test cost less than the third round.

## Risks / Trade-offs

- [A stale binary reports "never ticked" forever] → the field is `null` and the
  alert is opt-in, so it is inert until infra sets `max_tick_age_secs`; the
  rollout order (hub, then distiller, then the Gatus param) makes that window
  explicit rather than accidental.
- [Manual `cchv-distill` runs count as ticks] → they are ticks: they drain real
  work. `last_tick_mode` and `last_tick_groups_pending` let a reader tell an
  operator's backfill from a scheduled forward tick.
- [Another write path on the hot ingest DB] → one INSERT + one bounded DELETE per
  hour, against a table that holds hundreds of rows.
- [`no_tick` is a status string monitors have not seen] → unreachable unless the
  new opt-in param is set, so no existing body assertion can start failing.

## Migration Plan

1. Land hub + distiller + tests on `main`.
2. Release, swap the hub binary, reinstall `cchv-distill` on the hub machine
   (`docs/archive/deployment.md` §2b, §3c) — one relay, both halves.
3. Verify: `GET /v1/healthz/journal` shows a `last_tick_at` that advances after a
   tick, and a `ticks_last_24h` that sits well under 24 — this host averages
   **15.38/day**, not the 40–106 sleep cycles it turns over (§3c).
4. Infra chooses a `max_tick_age_secs` for the `cchv-journal` check, if any. On
   this host the honest first value is generous — it is a "the job is gone"
   detector, not a latency SLO. **Chosen 2026-08-21: `43200` (12 h)**, from a
   13-day replay whose observed max gap is 7 h 41 m (`ac/infra#117`; the working
   and the threshold table are in `docs/archive/deployment.md` §3c).
