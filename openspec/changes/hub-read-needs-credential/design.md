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

**D4. Failed-credential limit (R4).** Behind Tailscale serve every request comes from loopback, so a per-source limit is meaningless. The hub counts FAILED credential checks on all routes (bearer, cookie, login) in one global window, and over the limit answers 429 + `Retry-After` to requests whose credential fails. A request with a valid credential is never refused by the limiter. Tokens shorter than 32 characters are refused at config load (measured: all 8 live plaintext tokens are 64 characters).

**D5. Drop the trust by config on the A binary, refuse it in B (R2, R5).** After the D2 inventory (each caller run once with its own token, the on-demand ones included), infra removes `trust_tailscale_identity` from hub.toml AND from its template while release A runs, so release B only removes code. A rollback never re-adds the key. Release A logs every request accepted by the header (route and time, never the login). In release B the key is refused at load even when empty (`trust_tailscale_identity = []`), and so is the env var `HUB_TRUST_TAILSCALE_IDENTITY`; the top-level config gets `deny_unknown_fields` (or a sentinel field for the removed key) so a misspelled or removed key fails loudly.

**D7. Token fingerprints across tables (R3, Astra).** Today's duplicate check puts plaintext `[[tokens]]`/`[[readers]]` strings and `[[client_readers]]` HASHES in one map, so it never notices a client hash equal to SHA256 of an ingest or reader token, which would give that token client reads. The check compares SHA256 of every plaintext token with every client hash, and refuses the config on any match. A token refused as restricted (ingest-only, stats_only, reader) is final: no other table can widen it. Measured 2026-11-03: 0 collisions, 0 client readers today.

**D6. Enforce first (R1).** Enforce does not depend on dropping the trust, so it goes on before release A or right after it; the timing is manager-lab's, because enforce is a latch. Precondition, measured 2026-11-03: the live config has NO `[[client_readers]]`, so the distiller reads with a machine token and under enforce would lose every client session. infra mints the distiller's client-reader token (kv/agents/cchv-client-readers/distiller) and the distiller is shown reading a client session with it before the latch. Under enforce, a lab principal (machine tokens included) sees no client row on any route; the distiller keeps its client reader. ac's browser logs in with a LAB reader by default, so client rows never show in an everyday view; a client-scope login is separate and explicit (security 535b605d). A full view by default is ac's preference to state (a desk row via manager-lab), and its minting is infra's.

## Backlog (security B1-B6, not in this change)

B1 a registered-route inventory test as the walk's source of truth; B2 sessions: a re-login invalidates the previous cookie, a session cap, wall-clock expiry; B3 a `__Host-` cookie name; B4 cookie-only writes refused on every write route; B5 a CSP on the webapp; B6 where client-reader tokens live (bao path, who can read it). Filed as Gitea issues when this change lands.

## Risks

- A caller missed by D2 breaks with 401 at the drop. Mitigation: D1, and the drop ships with its own swap window and rollback (re-add the key with the previous binary).
- The 8 h cookie means ac logs in once per working day. Accepted (R2-H).
