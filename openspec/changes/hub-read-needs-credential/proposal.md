## Why

The hub grants lab read to any request whose `Tailscale-User-Login` header names ac's login, and checks only the header's value, never its source. Two paths follow (security P2, an addendum to its 2026-10-31 review of GPT reads of the hub):

- Any process on m4m can reach `127.0.0.1:8790` and set the header itself, with no credential.
- Tailscale serve injects ac's login for every connection from one of ac's other user-owned devices (ac-mbm5 included), so any process there that may make an HTTPS call reads, with no credential.

Client scope is in observe mode, so on the browse and search routes "lab read" is every row. The 2026-11-03 Woodpecker hook tokens (repos 1 and 57) were readable this way. Denying GPT seats the token stores, the existing control, does not help while a read needs no token.

## What Changes

- **Measure first:** the hub counts requests per principal kind (bearer token, reader token, client reader, cookie, Tailscale identity) and exposes the counts without names, so every caller that relies on the header is found before the trust is dropped.
- **Give every caller a credential:** each caller of the read routes today is listed with the credential it sends. A caller with none gets a token before the trust is removed (security's addition (a)).
- **The webapp logs in:** the archive webapp exchanges a token for the `/v1/login` cookie (HttpOnly, Secure, SameSite=Strict, 8 h; already built in R2-H) instead of relying on the header. It no longer keeps a token in local storage.
- **Login is rate-limited:** failed `POST /v1/login` attempts are limited per source address (security's addition (b)).
- **Drop the trust:** remove `trust_tailscale_identity` from the hub's config and code path. **BREAKING** for any caller still on the header: it gets 401.
- **Then enforce:** switch `[client_scope] enforce = true` on m4m, so a lab principal sees no client row on any route. ac's browser then needs a client-scope reader for a full view.

## Capabilities

### Modified Capabilities

- `archive-search-api`: the Authentication requirement drops the identity-header path, and gains the login rate limit and the principal counts.

## Impact

- Hub: `auth.rs` (the identity path goes), `login.rs` (rate limit), `config.rs` (the key is refused with a clear message, so a stale hub.toml fails loudly), a principal counter (healthz).
- Webapp: `ConnectGate.tsx` and `platform.ts` (cookie login; no same-origin header probe).
- Ops (infra): tokens for any caller without one; hub.toml edits; Gatus checks.
- Readers: the m4m daemon, distiller, backfill loader, quota-tap, cchv-find and the blog already send bearer tokens (verified per caller in task 1).
