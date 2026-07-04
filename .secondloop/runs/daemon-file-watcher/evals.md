# Eval rubric — Sync-daemon debounced file-watcher

Feature: `crates/sync-daemon` gets a `notify`-backed file watcher that
triggers early `run_once` sync passes, on top of the existing
`scan_interval_secs` (1h) safety-net rescan (MVP decision D6, deferred at
spec time 2026-06-25, implemented here 2026-07-04). The watcher is a
**latency optimization only** — correctness always comes from the periodic
rescan plus the hub's idempotent ingest. A missed or coalesced filesystem
event is never a correctness bug; only "did a signal make it out, and is the
rate of resulting sync passes bounded" is tested.

All six criteria are backend-observable (Rust crate internals: a `notify`
watcher, a pure debounce/throttle type, and `serde` config defaults) so all
are T2. There is no frontend/UI surface for this feature at all, so there is
no T1 file. Nothing here is subjective, so there is no T3/rubric-only item.

Eval file: `crates/loop-evals/tests/daemon-file-watcher_eval.rs` (6 tests: 4
`#[tokio::test]` driving the real `notify`-backed watcher against
`tempfile::TempDir` roots, 2 plain `#[test]`s for the pure `PassThrottle`
logic and `toml` config parsing).

Every eval drives only the frozen public stub surface already committed:
`sync_daemon::watcher::{spawn, PassThrottle}` and
`sync_daemon::config::DaemonConfig`. The unmodified stub compiles against all
six tests but never signals (`spawn` registers nothing, `pass_due` always
returns `false`, and the watch config fields default to `0`), so every
criterion fails at runtime today — confirmed by running the suite against
the unmodified crate (all 6 fail in ~2s, none hang against their outer
timeout bounds).

## Criteria

### AC1 — a written file yields a signal (T2)
`watcher::spawn` on a single `tempfile::TempDir` root with a 1s debounce:
creating and writing a file under that root produces a unit signal on the
returned channel within an outer 30s bound.
Eval: `ac1_creating_and_writing_a_file_yields_a_signal`.

### AC2 — recursive watch covers directories created after spawn (T2)
A new subdirectory created *after* `spawn` (simulating a new provider
project directory appearing later) with a file written inside it also
produces a signal within an outer 30s bound — the watch must be recursive
and must not be a one-time snapshot of the directory tree at spawn time.
Eval: `ac2_new_subdirectory_created_after_spawn_is_watched_recursively`.

### AC3 — bursts coalesce into a bounded number of passes (T2)
30 rapid appends to one watched file (20ms apart) produce at least 1 and at
most 6 signals observed over a 10s window. This is the debounce contract:
backend-dependent exact counts are fine, but neither "one signal per byte
written" nor "zero signals for an entire burst" is acceptable.
Eval: `ac3_rapid_appends_produce_bounded_signal_count`.

### AC4 — a bad root degrades, it doesn't disable watching (T2)
`spawn` is called with a nonexistent path listed *before* a valid
`TempDir` root. It must still return `Ok`, and a file created under the
valid root must still signal within an outer 30s bound — a single
unwatchable root (missing, permission denied) must never take down watching
for the others, matching the "log + skip, never crash" behavior spec'd for
the run loop.
Eval: `ac4_bad_root_before_valid_root_degrades_not_disables`.

### AC5 — `PassThrottle` remembers a pending trigger across the gap (T2)
Pure logic driven with fabricated `Instant`s (`min_gap` = 30s): `pass_due` is
`false` with no trigger ever recorded; becomes `true` immediately after
`note_trigger(t0)` (no prior pass required); after `note_pass(t0)` a fresh
`note_trigger(t0+1s)` keeps `pass_due` `false` at `t0+1s` (inside the gap)
but `true` at `t0+31s` (gap elapsed, pending trigger survives and fires);
once consumed, `pass_due(t0+32s)` is `false` again. This is the "a burst
always ends with a pass" guarantee.
Eval: `ac5_pass_throttle_remembers_pending_trigger_across_the_gap`.

### AC6 — config defaults and honors explicit watch fields (T2)
`toml::from_str::<DaemonConfig>` on a config with no watch fields yields
`watch_debounce_secs == 2` and `watch_min_pass_gap_secs == 30` (the real
production defaults, replacing the stub's placeholder `0`s); a config with
`watch_debounce_secs = 5` / `watch_min_pass_gap_secs = 120` set explicitly
yields exactly those values.
Eval: `ac6_config_watch_fields_default_and_honor_explicit_values`.
