# Sync-daemon: debounced file-watcher triggering early sync passes

## Description

Today the daemon is rescan-only: new sessions reach the hub up to
`scan_interval_secs` (1h) late. The MVP design (archived openspec change
`2026-06-25-history-archive-mvp`, decision D6) already chose the fix —
"`notify` for debounced file-watching, plus a periodic safety-net rescan" —
it was deferred, not rejected. This spec implements it as approved on
2026-07-04: **the watcher is a latency optimization that triggers an early
`run_once`; correctness stays with the rescan + idempotent hub ingest.**
Chosen over per-provider targeted sync because the checkpoint already makes
a full pass cheap (~seconds; unchanged files are stat-skipped).

Architecture (in `crates/sync-daemon`):

1. **Watcher** (`src/watcher.rs` — public STUB already committed; fill in
   the behavior, keep the committed signatures EXACTLY as-is since frozen
   evals compile against them):
   - `spawn(roots, debounce) -> anyhow::Result<(WatcherGuard, mpsc::Receiver<()>)>`
     registers a recursive watch on each root using the `notify` crate
     (FSEvents on macOS / inotify on Linux; add `notify`, and a debouncer
     crate if useful, to sync-daemon's dependencies). Debounced bursts of
     create/modify/rename events under any root collapse into unit signals
     on the channel (bounded channel; dropped sends are fine — a pending
     signal already means "pass soon"). Roots that fail to register
     (missing, permission denied) are logged via `tracing::warn!` and
     skipped; `spawn` still returns `Ok` and watches the rest. Dropping
     `WatcherGuard` stops the watcher. Watching must survive events for
     paths that vanish before handling (atomic-write temp files).
   - `PassThrottle` coalesces signals into bounded-rate passes:
     `note_trigger(now)` records a pending trigger, `note_pass(now)`
     records a completed pass, `pass_due(now)` returns true iff a trigger
     is pending AND `now - last_pass >= min_gap`, consuming the pending
     trigger. A trigger arriving mid-gap is remembered so a burst always
     ends with a pass. Pure logic, caller supplies `Instant`s.

2. **Config** (`src/config.rs` — stub fields already present with
   placeholder `#[serde(default)]` = 0): give them real serde default fns —
   `watch_debounce_secs` default **2**, `watch_min_pass_gap_secs` default
   **30**. Both also honored when set explicitly in the TOML. Env-var
   fallback path in `load()` may keep the defaults (no new env vars).

3. **Run loop** (`src/lib.rs::run`): build watch roots from
   `history_core::providers::detect_providers()` `base_path`s, minus
   providers in `providers_exclude` (same exclusion semantics as scanning —
   never watch what we won't scan; this is the cloud-mount-wedge guard).
   Spawn the watcher; on spawn failure `tracing::warn!` and continue
   rescan-only (the daemon must never crash or exit because watching is
   unavailable). Extend the existing `select!` so the sleep arm, a watcher
   signal (via `PassThrottle`), and Ctrl-C coexist: a due trigger runs
   `run_once` immediately; the periodic rescan keeps firing on its own
   schedule regardless of watcher activity. Keep clippy (workspace lints)
   and rustfmt clean.

Non-goals (do NOT implement): byte-offset append parsing, per-provider
targeted sync, re-detecting providers at runtime (restart picks up new
providers; rescan covers the gap), reacting to file deletions (archive is
cumulative), watching the state dir, hub-side changes, launchd changes.

Eval mechanics (T2, `loop-evals` crate): drive ONLY the committed stub
surface — `sync_daemon::watcher::{spawn, PassThrottle}` and
`sync_daemon::config::DaemonConfig` (via `toml::from_str`; `toml` is in
loop-evals dev-deps) — plus `tempfile::TempDir` roots created inside each
test. Never watch real home directories. The stub compiles but never
signals, `pass_due` is always false, and config defaults are 0 — so every
criterion below fails at RUNTIME against the unmodified crate (wrap waits
in outer `tokio::time::timeout` bounds; a fired outer bound = eval fails,
it must never hang). Real timers are fine (loop profile is
single-threaded); keep waits short via the 1–2s config values below.

## Acceptance Criteria

- (T2) `watcher::spawn` on one `TempDir` root with a 1s debounce: creating and writing a file under that root yields a signal on the channel within an outer 30s bound.
- (T2) A file created inside a NEW subdirectory made after `spawn` (nested dir under the root, like a new project dir) also yields a signal within an outer 30s bound — the watch is recursive and covers directories created later.
- (T2) 30 rapid appends to one watched file produce at least 1 and at most 6 signals observed over a 10s window (debounce coalesces bursts; exact count is backend-dependent but must be bounded).
- (T2) `spawn` with a nonexistent path listed BEFORE a valid `TempDir` root returns `Ok`, and a file created under the valid root still signals within an outer 30s bound (bad roots degrade, they don't disable watching).
- (T2) `PassThrottle` with `min_gap` 30s and a fabricated timeline: `pass_due` is false with no trigger; after `note_trigger(t0)` it is true at `t0`; immediately after `note_pass(t0)` a `note_trigger(t0+1s)` keeps `pass_due(t0+1s)` false but `pass_due(t0+31s)` true (pending trigger survives the gap); once consumed, `pass_due(t0+32s)` is false again.
- (T2) `toml::from_str::<DaemonConfig>` on a config WITHOUT watch fields yields `watch_debounce_secs == 2` and `watch_min_pass_gap_secs == 30`; WITH `watch_debounce_secs = 5` and `watch_min_pass_gap_secs = 120` it yields exactly those values.
