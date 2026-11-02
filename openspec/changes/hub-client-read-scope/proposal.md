# Hub client-read scope

## Why

security's R5 ruling (2026-11-01 23:5xZ, relayed by manager-lab, bell c78fc25a): today the hub
answers every read the same way. Any accepted caller can read every session, client sessions
included:

- a machine token;
- the blog's read-only token;
- the trusted Tailscale identity. Any process on m4m gets that identity through `tailscale serve`,
  without a token.

Client material must reach Claude models only. A GPT-routed session that can reach the hub can
read a client session through `/v1/search`, `/v1/sessions/{id}/messages`, the browse routes or the
journal. The distiller already keeps client days away from GPT (protected routing, v0.27.0, made
whole-session in v0.34.0). The hub has no such line.

Target: designed, reviewed and built this week, before security's 11-07 review.

## What Changes

- **Session flag.** Every session gets a client flag, computed by the hub from the same routing
  file the distiller reads (`~/.config/cchv/distill-routing.toml`, outside the repo). It uses the
  distiller's match rules and is fail closed:
  - set at ingest;
  - backfilled by a sweep;
  - never cleared without an approved narrowing.
- **Two read scopes.**
  - *lab read*: what every read principal has today.
  - *client read*: lab read plus client sessions. Held only by named principals that are
    Claude-only.

  Machine tokens, the blog's reader token and the trusted Tailscale identity get lab read only.
- **Every read route filters for a lab-read caller.**
  - A client session is absent from lists, search hits, stats and identities.
  - Its messages answer 404.
  - A journal entry built from a client session is absent.

  A route-walk test fails if any registered GET route leaks a client canary to a lab-read caller.
- **New principals.** A token entry (`[[tokens]]` or `[[readers]]`) may set `client_read = true`.
  These principals get it:
  - the distiller's own token;
  - the blog's client-audience reader;
  - a human client-read token for ac's browser.

  The launcher renders them from a separate bao path.
- **Rollout in stages.**
  1. *Observe*: flags computed, nothing hidden, counts logged.
  2. Security reads the counts.
  3. *Enforce*.

## Impact

- **Code:**
  - `crates/hub/src/{auth.rs, config.rs, state.rs, ingest.rs, browse.rs, search.rs, journal.rs, journal_threads.rs, stats_api.rs, identities.rs, findings.rs}`;
  - a new `client_scope.rs`;
  - a migration (`sessions.client_class`, `client_scope_state`);
  - `scripts/cchv-launch.sh`;
  - `scripts/cchv-distill.py`: its token, and reading the hub's flag.
- **Specs:** `archive-search-api`, `archive-ingestion`, `journal-entries`.
- **Consumers:**
  - the webapp: a client-read sign-in for ac;
  - cchv-find: lab read by default;
  - the blog: a second, client-audience token, written by a Claude-only writer.
- **Not in scope:**
  - the distiller's own routing, which is unchanged;
  - GPT sessions' network allow-lists (security's 11-07 item);
  - copies already read by a GPT session before enforcement.
