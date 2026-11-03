## Context

Security's P3 of 2026-11-03 (ac/security 86b402a) offered two fixes: the hub sweeps by itself after adopting a generation, or the stats routes say stale. cchv picked stale (no new writer); security ruled it not strict (3124a67): a fixed short margin in minutes may be excluded if the 200 carries the excluded count.

## Decisions

**D1. Lab-eligible only.** The rule counts sessions with `client_class IS NULL OR client_class = 'none'` and `client_gen IS DISTINCT FROM` the live generation. A client-class session is hidden from a lab reader whatever its stamp (classes only rise), so it cannot make lab counts incomplete. Measured 2026-11-03 09:33Z: the only unscanned session was client-class; with D1 it does not trip the rule.

**D2. Postgres, not the mirror.** `sessions` in Postgres is the source of the class and stamp; the mirror copies it on refresh. The check reads Postgres once per request (one indexed count).

**D3. Window overlap.** Daily: a session is in the window when `last_message_time >= from` (local midnight in `tz`) and `first_message_time < to + 1 day`. Tool-calls: `last_message_time >= since`. A session without message times holds no counted rows and is ignored.

**D4. Margin.** `STALE_MARGIN_MINUTES = 15`, a constant (security: fixed, short, in minutes). Inside it a session is left out and counted in `unscanned_excluded`. A sweep skips sessions an ingest touches while it runs; 15 minutes covers a daemon's push cadence (about one minute) with room for a slow batch.

**D5. Full-scope readers.** `Visibility::All` sees unscanned sessions, so the check is skipped and `unscanned_excluded` is 0.

**D6. The 503.** `HubError::Unavailable` with the reason `N lab sessions in the window are unscanned under generation G; a client-scope sweep stamps them`. The count is an integer over all sessions, never an id, a name or a path.

## Risks

- A lab session that a sweep raced and that then goes quiet keeps the routes at 503 until someone runs a sweep. This is intended: the 503 is the signal to sweep.
