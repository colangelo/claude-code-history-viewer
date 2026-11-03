## Why

After the hub adopts a new client-scope generation, every session is unscanned until `client-scope sweep` stamps it, and a lab reader never sees an unscanned session (security O1). `/v1/stats/daily` and `/v1/stats/tool-calls` then answered 200 with the unscanned sessions silently missing. On 2026-11-03 a loader got 0 rows with a 200 for about 2h40m (03:23Z incident) and again from 06:50Z until the sweep at 07:04-07:18Z. A 200 must mean the counts are complete.

## What Changes

- Both lab stats routes check, before answering, whether any session in the window is **lab-eligible and unscanned**, meaning its class is `none` or unset and its generation is not the live one. A session whose class is client is hidden from a lab reader whether it is scanned or not, so it cannot make lab counts incomplete and is not counted.
- An eligible unscanned session whose last message is older than a fixed margin (`STALE_MARGIN_MINUTES = 15`) makes the route answer **503** with a reason naming the count and the live generation (sweep needed).
- An eligible unscanned session inside the margin (still being written: the sweep skips sessions an ingest touches) is left out, and the 200 answer carries the count in a new field `unscanned_excluded`.
- A full-scope reader sees every session, scanned or not; for it the check does not apply and `unscanned_excluded` is 0.
- No new writer: the hub still never sweeps by itself (security ruling 2026-11-03, ac/security 86b402a and 3124a67).

## Capabilities

### Modified Capabilities

- `archive-analytics`: the daily-stats and tool-call-count requirements gain the completeness rule.

## Impact

- `crates/hub/src/stats_api.rs`: `daily` and `tool_calls` handlers, a shared check on Postgres, and the new answer field.
- Tests in `crates/hub/tests/stats_daily_test.rs` and `stats_tool_calls_test.rs`.
- Readers: the monitoring loader and quota-tap get a 503 (to retry) instead of an empty 200 during a generation change. The JSON gains one field; no field is removed.
