## Context

Security P2 (2026-11-03, ruling 344b9cfa / 3d720785): pick (2) "drop the identity trust" then (3) "enforce"; not the unix socket, since a stolen token reads through serve anyway. Tailscale serve (v1.104.4 source, per infra 086e16a6) strips incoming identity headers and sets them only for user-owned source nodes, never for local or tagged traffic.

## Decisions

**D1. Measure before removing.** A per-principal-kind counter (`/v1/healthz/auth`: counts since start by kind, no names, no tokens) runs for at least 24 h on the release before the drop. Any non-zero `tailscale` count is traced to its caller (time and route only) and moved to a credential first. The window must cover one full cycle of the slowest caller (security 535b605d): every caller in D2 runs at least daily except on-demand ones (a manual `--backfill-from` loader run, `just tm-backfill`, a hand-run cchv-find). Each on-demand caller is run once by hand with its token, and shown working, before release B.

**D2. Callers today** (task 1 verifies each against the counter and the caller's own config):

| caller | route(s) | credential today |
|---|---|---|
| m4m sync daemon | `/v1/ingest` | machine bearer token (ingest never took the header) |
| ac-mbm5 sync daemon (via serve :8787) | `/v1/ingest` | machine bearer token |
| distiller (m4m launchd) | journal, sessions, messages | machine token + client-reader token (bearer) |
| infra backfill loader | `/v1/stats/daily` | stats_only reader token |
| quota-tap | `/v1/stats/tool-calls` | stats_only reader token |
| cchv-find (any agent) | search, browse | machine token from bao / op |
| agents blog | browse | reader token `agents-blog` |
| Gatus (infra) | `/v1/healthz*` (no auth), any authenticated probe? | to verify with infra |
| cchv's own checks | `/v1/healthz*` | none needed (unauthenticated routes) |
| archive webapp (ac's browser) | all read routes | **the identity header** -> moves to `/v1/login` |

**D3. Webapp login.** ConnectGate stops probing same-origin without a credential. It shows the token form, POSTs `/v1/login` with `credentials: "same-origin"`, and from then on sends the cookie. The token is never stored in local storage; a stored one from the old flow is deleted on load. Logout calls `DELETE /v1/login`.

**D4. Rate limit.** At most 10 failed logins per source address per 10 minutes, then 429 with `Retry-After` until the window passes. Successes do not count. The limiter lives in memory like the sessions.

**D5. Refuse the old key.** `trust_tailscale_identity` in hub.toml becomes an error at start ("removed: reads need a credential; see deployment.md"), so a forgotten line cannot silently keep or drop behaviour.

**D6. Enforce last.** After D1-D5 are live and the counter shows no header use, infra sets `enforce = true`. Under enforce, a lab principal (machine tokens included) sees no client row on any route; the distiller keeps its client reader. ac's browser logs in with a LAB reader by default, so client rows never show in an everyday view; a client-scope login is separate and explicit (security 535b605d). A full view by default is ac's preference to state (a desk row via manager-lab), and its minting is infra's.

## Risks

- A caller missed by D2 breaks with 401 at the drop. Mitigation: D1, and the drop ships with its own swap window and rollback (re-add the key with the previous binary).
- The 8 h cookie means ac logs in once per working day. Accepted (R2-H).
