## Context

`/v1/healthz/journal` (crates/hub/src/health.rs) folds `SESSION_DAYS_CTE` into pending `(entry_date, project_path)` groups inside `within_days`, marks a group stale when its newest arrival is older than `grace_secs`, and answers 503 on `stale` (or `no_tick` when `max_tick_age_secs` is given). `GET /v1/journal/pending` (crates/hub/src/journal.rs) computes the same set for the distiller; the existing spec requires the two to agree ("Health agrees with pending…"). Ticks live in `distiller_ticks` (migration 0007): `mode` is `forward` | `backfill`, validated in `journal.rs` (`matches!(payload.mode.as_str(), "forward" | "backfill")`). The distiller (`scripts/cchv-distill.py`) runs `claude -p` per group (`_claude_generate`) and posts one tick per run before its single `pending(from, limit=50)` batch.

## Goals / Non-Goals

**Goals:** a red check means a group that should and could have been distilled was not. A pause on purpose reads as paused. Pending and health keep using one predicate.

**Non-Goals:** giving the distiller a client reader (security + manager-aruba, then an ac row); resuming the distiller (ac's brake); changing the Gatus URL.

## Decisions

### D1 — One exclusion predicate, built once, used by both queries

`[journal] exclude_paths = ["…"]` in the hub config: glob patterns where `*` matches any run of characters (including `/`). The built-in defaults always apply and the config adds to them: `/tmp/*`, `/private/tmp/*`, `/private/var/folders/*`. Each pattern is translated once at config load into a SQL `LIKE` pattern (escape `\`, `%`, `_`; `*` → `%`) and validated (non-empty, absolute, starts with `/` or `~/`; `~/` is expanded against `[journal] home`, which is required whenever a `~/` pattern is used, so the hub never guesses the archive owner's home). A single helper renders the `AND NOT (project_path LIKE ANY($n))` clause, and both the health query and the pending query call it with the same bound array. The equality test (D5) pins the agreement.

*Alternative rejected:* filtering in the distiller only. Health would still count the groups, so the check stays red.

### D2 — Client groups: counted, red only when drainable

A group is client when any of its sessions has `coalesce(client_class, 'none') <> 'none'`. The health row carries `is_client`. `client_groups_page := !config.client_scope.enforce || !config.client_readers.is_empty()`. A stale client group sets `any_stale` only when `client_groups_page` holds. The body gains `client_undistilled: <count of in-window client groups pending>` and `client_groups_page: bool`, and each group in `groups` gains `client: bool`. Paths stay hidden exactly as today (L1/D6: the path goes only to a reader allowed to see all its sessions).

`pending` is NOT changed for client groups: what a principal sees there is already decided by client scope.

### D3 — `paused` is a tick mode with a reason

Migration `0019_distiller_tick_reason.sql`: `ALTER TABLE distiller_ticks ADD COLUMN reason text` (nullable; no backfill). `POST /v1/journal/ticks` accepts `mode` ∈ {`forward`, `backfill`, `paused`}. `paused` requires a non-empty `reason` (≤ 500 chars, else 400). The other modes reject a reason (400), so a reason always means a pause. Same auth as today.

Health: when the newest tick's mode is `paused`, `status = "paused"`, HTTP **200**, and the body adds `paused_reason` and `paused_since` (that tick's `tick_at`). `paused` outranks `stale` and `no_tick`: a deliberate pause is the explanation for both. The groups list and counts are still reported. Any later `forward`/`backfill` tick ends the pause, because "newest tick" is the rule. `ticks_last_24h` keeps counting all modes, as it does today; document that a pause tick is counted.

### D4 — Drain to empty, bounded

Forward mode only: loop { `pending(from, 50)`; if empty → stop; process the batch; stop if the elapsed time exceeds `--budget-secs` (default 2700); stop if the batch had zero successes (no progress: the same failing groups would come back forever) }. The tick is still posted once per run, before the first batch, with that batch's size (the field means "work list at start"; unchanged semantics). Backfill keeps its single bounded batch (`--limit` is its contract).

### D5 — Connectors off for every model CLI

In `_claude_generate`: always set `env["ENABLE_CLAUDEAI_MCP_SERVERS"] = "false"`, and when `~/.config/claude/direct-noconnectors.settings.json` exists, add `--settings <that path>` to the `claude -p` argv. Both, because the env alone is the documented switch and the settings file is the fleet's agreed form (ac a1103-08); either alone must suffice. Any other model CLI the script starts gets the same env.

### D6 — `cchv-distill --pause --reason TEXT`

Posts the `paused` tick and exits 0 without resolving model credentials or reading pending. `--reason` is required with `--pause`. There is no `--resume` flag: the resume is the next scheduled forward run, whose tick ends the pause.

## Risks / Trade-offs

- **A forgotten pause hides a real outage.** Mitigation: `paused_since` is in the body. infra's Gatus condition can alert on a pause older than N days. Their call; noted in the relay.
- **An exclusion pattern too wide hides real work.** Mitigation: defaults are the OS temp roots only; everything else is explicit in `hub.toml`, which infra reviews. The pending-equals-health test covers the predicate, not the list.
- **Drain loop spends a burst of model calls.** Same total as today, sooner; it only runs while the distiller is not paused.

## Migration Plan

Migration 0019 is additive (a nullable column), applied at hub start as usual. An older distiller keeps posting `forward`/`backfill` without a reason: still valid. A newer distiller against an older hub: `--pause` gets 400 (unknown mode); the script reports it and exits 1. Order: hub first, then the distiller install.
