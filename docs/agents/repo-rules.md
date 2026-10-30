# Repo rules

> Moved word for word from `AGENTS.md` on 2026-10-30 (ac's yes, asks row a1030-39), which links here.

## Repo rules (learned 2026-08-21 — evidence in `docs/2026-08-21-journal-day-bucketing.md`
and `docs/2026-08-21-identity-surfaces-and-query-floor.md`)

Only the things that bind from now on. The story, the measurements and the other lessons
are in the two docs — morning (the wrong-day journal bug) and afternoon (the v0.21.0
release and the query-floor measurements).

- **A row in `messages` is not a conversation turn.** 91.1 % of `claude`-provider rows
  have `content IS NULL` — they are sidecar state records (`permission-mode`,
  `agent-color`, `worktree-state`, …). Never size, quote or reason from a raw row count
  without saying which you mean; a session that is "67,889 messages" is 5,997
  conversation items. It is also why roughly half of any journal backfill legitimately
  posts `skip`. Tracked as #41.
- **An installed script is a COPY — a green `main` says nothing about what is running.**
  `~/.local/bin/cchv-distill` is installed, not symlinked, and announces no version, so a
  fix can sit undeployed while every checkbox says otherwise. Verify the *installed* file
  (`cmp` against `git cat-file -p HEAD:<path>`) before believing any claim about distiller
  behaviour. The rule is about the **install boundary**: when one task turns out
  undeployed, re-check every task on the far side of it, not just that one. Tracked as #40.
- **When a change alters the granularity of a key, sweep every predicate that tests that
  key.** The journal's day fold moved session → day and left three predicates behind — the
  transcript window, the arrival timestamp and the dirty check — two of which reached
  production. Grep for the old key and re-derive each hit before shipping, rather than
  fixing them as they surface.
- **Before trusting a check, name the reading that would make it FAIL. If you cannot, it
  is not a check.** Three in one afternoon, each of which produced a confident wrong answer:
  `count(DISTINCT uuid)` to measure ingest duplication (returns 1.0 always — `convert.rs`
  fills a missing uuid with a **random v4** per re-parse, and `content_hash` is populated on
  **0** rows); `curl --max-time N` to time an endpoint (abandons the request but leaves the
  query running server-side — six piled up and starved each other, so the reading was of
  our own backlog); and `until [ "$x" != "null" ]` to wait for a field (a *failed read* is
  also `!= "null"`, so it announced a tick that had not happened). Same family as
  `--is-ancestor` collapsing exit 1 and 128.
- **Publish the number from the machine that will run it.** Migration `0009` measured
  0.93 ms locally against a table of prod's exact shape and 7.15 ms on prod — **7.7×**.
  A local stand-in predicts the order of magnitude and nothing finer; measuring the wrong
  machine is its own kind of estimate, even when you did measure.
- **A measurement taken inside a cycle needs its position in that cycle, or it is a
  measurement of the day.** `messages` is append-only, so VACUUM is insert-triggered
  (~1.5 M inserts) and visibility-map coverage over the hot range decays between runs: the
  share of a 7-day window needing `Heap Fetches` swings **0 % → ~60 %**. Record
  `n_ins_since_vacuum` and `relallvisible`/`relpages` with any index timing, and say which
  way the current position biases it. Related: a threshold is not a schedule —
  `last_autovacuum` being old is not a backlog when no trigger has been crossed.
- **A phase read through a poller is the POLLER's phase until you sample faster than the
  event.** The hourly `/v1/healthz/journal` excursion was called "a recurring `:05`/`:10`
  pair" for two rounds because Gatus polls every 300 s. Sampled at 150 s it is one event
  spanning **`:04`–`:12`** whose internal peak moves hour to hour — `:05` one hour, `:09`
  the next — because pg1 fires three to four WAL checkpoints in that window and the biggest
  flush is not always the first. The "pair" was two polls of one event, and the phase was
  an artifact of the instrument. Nyquist, and it bites hardest where the reading looks
  cleanest: a stable-looking `:05` repeated across many hours reads as a *strong* signal.
  Before keying anything to a minute — an alert window, a maintenance slot, a sampling
  schedule chosen to dodge the event — sample at least twice as fast as the thing you are
  timing and check the width. Evidence: `docs/2026-08-23-journal-hourly-excursion.md`.
  **And a phase is a property of the driver's current TRIGGER, so changing the trigger moves
  it.** `max_wal_size` 4 GB → 8 GB (2026-08-24T11:20Z) doubled the WAL distance a forced
  checkpoint must accumulate, so it takes twice as long to arm: the window went from
  `:04`–`:12` to a single checkpoint at `:13:15`–`:17:04`. Every window-keyed grep, probe
  window and alert on both sides then failed in the *reassuring* direction — still running,
  still passing, sampling a quiet stretch and reading as "the storm stopped". A window is a
  cached derivation; re-derive it whenever the thing that triggers the event changes, and
  treat a fixed window that suddenly goes quiet as unverified rather than as good news.
  **Then, ~40 minutes later, the storm really did stop** — `direction` shipped its
  incremental-reconcile fix (0.99.3) and pg1 measured **zero** forced checkpoints across four
  consecutive hours (infra, 2026-08-24, thread `8d5eb1ba`). So both windows now match
  nothing, and a window-keyed grep is silent for three indistinguishable reasons: wrong
  phase, no event, or dead instrument. **The way out is not a better window, it is a positive
  control in the same reading** — infra's zero is trustworthy only because
  `checkpoint starting: time` was still logging 4/hour beside it and the log's last line was
  minutes old. And the reason the fix could be told apart from the lever at all is that the
  discriminator was **written down before the lever was pulled**: *"only zero is unambiguous;
  1–2/hour is what the doubled trigger does on its own."* Doubling a trigger cannot reach
  zero against a live 7–9 GB/hour write. State the falsifier before the confound arrives and
  you never have to untangle it.
- **Say WHO took a reading, not just that it exists — and never inherit a "broken" claim
  without one.** Two instances, both caught by a successor rather than by the author:
  "verified from two vantage points" was one vantage (loopback on m4m) plus infra's
  reported Gatus reading, which a successor on the same box cannot reproduce; and
  `ac/infra#98` was named as a live cause of message loss all day by a session that **sent
  nothing over the relay** and so had no first-hand evidence either way — infra had in fact
  proved the 401 fixed at 2026-08-21T15:03:52Z in a message that sat undelivered on our own
  `interactive` queue. Either take the reading or label it inherited and name who last
  measured it. Also from that message: `auto` is a **shared** wildcard queue
  (`relay.msg.*.auto`), not per-host routing, so one green relay smoke test proves nothing
  about the other Mac.
- **An incoming CORRECTION is a reading too — ask which query produced it before you edit a
  figure you derived.** 2026-08-24: infra corrected our `shared_buffers = 196608 × 8 kB =
  1.5 GB` to *"1.875 GB"*, and this side accepted it in ~20 minutes and rescaled every
  share-of-cache percentage in `docs/2026-08-23-journal-hourly-excursion.md` by 0.8×. **Our
  number was right.** Theirs came from `SELECT setting || coalesce(unit,'')`, which for a
  setting whose `unit` is `8kB` glues value to unit — `196608` + `8kB` → `1966088kB` — a
  *plausible, well-formed* number answering a different question. Same family as
  `--is-ancestor` collapsing 1 and 128, with a new edge: **it took a correct statement down
  with it**, and the bullet above is what let it — "we never measured it, they did" is not
  "their reading is sound". Arithmetic we performed outranks a transcription we did not see
  the query for. Read an `8kB`-unit setting as
  `pg_size_pretty(setting::bigint * 8192)`, and when a correction contradicts something
  *derived*, ask for the query before rewriting the document. **Blast radius, since it is
  the reason this one survived review** (infra, 2026-08-24): that probe corrupted every
  `8kB`-unit row and no others — `shared_buffers`, `effective_cache_size`, `wal_buffers` —
  and those are exactly the rows whose corrupted form still reads as an ordinary kB figure,
  so it was **wrong for every row it touched and visibly wrong for none**. Per-row
  plausibility cannot catch that class; only re-deriving from `setting` and `unit` can.
  **The complement, so this rule does not harden into refusing every correction** (infra,
  2026-08-24, same thread): they withdrew *"`max_wal_size = 8 GB` costs nothing"* two hours
  after sending it — pg_wal had gone 4.1 → 7.6 GB. We took that one **the same day**, and the
  discriminator was not who measured it but whether it **re-derives from what came with it**:
  483 × 16 MiB = 7,728 MiB = 7.55 GiB reproduces the headline from the parts, and free space
  falling 27 G → 24 G accounts for the same ~3.5 GB from a *different command*. The
  `setting||unit` figure shipped alone and unre-derivable. So: *ask which query produced it*
  is the question; **"can I rebuild this number from what the message contains, and does a
  second instrument agree?" is the usable test.**
- **A parameter with more than one effect is not "free" until each effect is measured
  separately.** `max_wal_size` is both a forced-checkpoint trigger *and* the cap on the
  recycled WAL pool. Infra measured the trigger (0.03 GB/hour → ~11 days to fill 8 GB),
  correctly called it inert, and then asserted inertness of **the setting**, which was one
  `du` away from being false — the pool had grown 3.5 GB. The failure is not the
  measurement, it is the quantifier: a reading of one mechanism was generalised to the knob.
  Same shape as the poller-phase and `setting||unit` errors — a confident number from an
  instrument never pointed at the question actually being answered. Before writing *inert*,
  *free*, *no-op* or *costs nothing* about a setting, enumerate what it controls and say which
  of those you measured. Evidence: `docs/2026-08-23-journal-hourly-excursion.md` § *The disk
  cost*. **And then the correction over-reached in the other direction, which is the half worth
  carrying (2026-08-30, relay `279aebbe`, infra#129): the pool DRAINED** — 7.48 → **1.86 GiB**
  over six days, below what it held at 4 GB, with `df avail` back **above** the July baseline.
  So the 3.5 GB was real and **temporary**, and our doc's *"permanently"* was never in the
  reading it cited. The mechanism is why it was invisible: segments leave by **removal at ≲1
  per checkpoint** (0.617/0.630/0.563 across three windows, `0 recycled` every time) — **linear
  in checkpoints, not exponential in the distance estimate** — which is ~12 segments of 483
  across the ~20 checkpoints the original claim watched, i.e. indistinguishable from noise.
  The "28× theory gap" both sides filed as unexplained was a category error: the EMA governs
  the *estimate*, a bounded per-checkpoint removal governs the *pool*. **A plateau observed on
  a lever too short to resolve the curve is not a plateau** — and note the asymmetry in who got
  burned: infra hedged at the time (*"whether it drains is UNRESOLVED"*), we did not, so the
  stronger word entered the record on our side. When a peer hedges a number and you are
  paraphrasing it, the hedge is part of the number. And note where the fence actually sits — this side **can** run
  `ssh ac@pg1… df -h /var/lib/postgresql` headless, so "every pg1 number is relayed" was
  itself too broad: `du` on the 0700 data dir is infra's, `df` is ours.
- **The release ceremony's own guards are in § Release Process and they are load-bearing**
  — `gh run list --commit <tag-sha>` across all workflows (not just `server-release.yml`),
  and a publication proof that reads `rc` by `case` because `--is-ancestor` exits 1 for
  "no" and 128 for "that object does not exist". Both were added after each was wrong
  twice in one day.
