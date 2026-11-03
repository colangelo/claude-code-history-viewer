## 0. Before anything ships

- [ ] 0.1 Caller inventory (R2): every D2 caller, on-demand ones included, run once with its own token and shown working; Gatus's credentials from infra.
- [ ] 0.2 Enforce precondition (R1, D6): infra mints the distiller's client-reader token; the distiller reads a client session with it.
- [ ] 0.3 Enforce on m4m (R1): timing with manager-lab (a latch); read-back: lab and stats readers see no client row, the distiller still reads its sessions.

## 1. Release A (tests first, each RED on today's main)

- [ ] 1.1 Principal counter by kind + `/v1/healthz/auth` (counts only); every header-accepted request logged with route and time.
- [ ] 1.2 Failed-credential limit across all routes, global (D4): over the limit a failing credential gets 429 + Retry-After; a valid credential is never refused; tokens under 32 chars refused at load.
- [ ] 1.3 Fingerprint check across tables (D7): a client hash equal to SHA256 of an ingest/reader token refuses the config; a restricted token stays restricted.
- [ ] 1.4 Webapp cookie login (D3): no same-origin header probe, no token in local storage (a stored one deleted); vitest; frontend review for the visible change.
- [ ] 1.5 Security single read of release A's code diff (security be7f0c79), then release A, swap. Then infra removes `trust_tailscale_identity` from hub.toml and its template (R2); counts and header log show 0 header hits over one full cycle of the slowest caller.

## 2. Release B (code only)

- [ ] 2.1 Tests (RED today): the old key fails to load even when empty, and so does `HUB_TRUST_TAILSCALE_IDENTITY`; an unknown top-level key fails (deny_unknown_fields or sentinel); a forged header gets 401 on every registered read route, walked with a LAB COOKIE as the control (R5); every D2 caller still reads.
- [ ] 2.2 Remove the identity path from `auth.rs` and the config; docs (`deployment.md`, the spec).
- [ ] 2.3 Security re-check; release B; swap.

## 3. Backlog

- [ ] 3.1 File B1-B6 as Gitea issues. B4 (cookie-only writes refused on every write route) lands before any write is ever allowed with the cookie (security be7f0c79); today every write needs a bearer token.
