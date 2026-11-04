## Why

The Gatus check on `/v1/healthz/journal` (tailnet_cchv-journal) is red so often that nobody treats a red as news: 66 % uptime over 24 h, 87 % over 7 d (infra, 2026-11-04 11:5xZ). The alert has been silenced since August (infra #118). ac asked why the status page "always" shows something cchv and wants it fixed for good. infra measured four causes:

1. **A paused distiller reads as stale.** Under ac's usage brake the distiller job was booted out at 05:00Z; 35 groups of 2026-11-03 then sat pending and the check said `stale`, with no way to say "paused on purpose".
2. **One batch per run.** A forward run takes at most 50 groups (`limit = 50`). A busy day closes with 50+ groups at once, and the check stays red each morning until several hourly runs have drained it.
3. **Throwaway paths are queued.** Groups under `/private/tmp`, `/private/var/folders/…/T/…` and agents' scratch state directories cost a model call each and hold the check red.
4. **Client groups the distiller may never read are counted.** The health verdict counts every pending group. When client scope enforces and no client reader exists for the distiller (today its bao read is refused, 403), client groups are not listed to it, so they can never drain, and every client day would read stale for `within_days`.

## What Changes

- **Excluded paths (a):** a hub config list of path patterns (`[journal] exclude_paths`). `/v1/journal/pending` and the health verdict both skip matching groups, so they cost no model call and never page. Built-in defaults cover the OS temp roots; the rest is set in `hub.toml`.
- **Client groups count only when someone can drain them (a, d default):** the health body reports client-class groups separately (`client_undistilled`). They count toward `stale` only while some principal can read them: client scope not enforcing, or at least one `[[client_readers]]` entry configured. Otherwise they are reported, never red. Whether the distiller gets a client reader is for security + manager-aruba (and ac's row if it is granted); this change only stops an undrainable group from paging.
- **Paused state (c):** `POST /v1/journal/ticks` accepts `mode = "paused"` with a `reason`. While the newest tick is `paused`, the check answers **200** with `status: "paused"`, the reason and since when. The next `forward` or `backfill` tick clears it. `cchv-distill --pause --reason TEXT` posts that tick and exits.
- **Drain to empty (b):** a forward run re-reads pending and keeps going until the queue is empty, a time budget is spent (`--budget-secs`, default 2700), or a batch makes no progress. Same total model calls, finished sooner. It changes nothing while the distiller is paused.
- **Connectors off:** every `claude -p` the distiller starts gets `--settings ~/.config/claude/direct-noconnectors.settings.json` when that file exists, and `ENABLE_CLAUDEAI_MCP_SERVERS=false` in its environment always (ac a1103-08, security #56).

## Capabilities

### Modified Capabilities

- `journal-health`: excluded paths, client-group counting, the `paused` status.
- `journal-entries`: `pending` honours excluded paths; ticks accept `paused`.

## Impact

- Hub: `crates/hub/src/health.rs`, `crates/hub/src/journal.rs`, `crates/hub/src/config.rs`; migration `0019_distiller_tick_reason.sql` (nullable `reason` column on `distiller_ticks`).
- Distiller: `scripts/cchv-distill.py` (`--pause`, drain loop, CLI settings/env).
- Monitoring: no URL change. After the release infra un-silences the Gatus alert (#118) and routes it to cchv.
- Not in scope: granting the distiller a client reader; resuming the distiller (ac's brake).
