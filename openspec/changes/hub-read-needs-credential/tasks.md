## 0. Before anything ships

- [ ] 0.1 Caller inventory (R2): every D2 caller, on-demand ones included, run once with its own token and shown working; Gatus's credentials from infra.
- [ ] 0.2 POSTPONED (D6): enforce precondition: infra mints the distiller's client-reader token; the distiller reads a client session with it.
- [ ] 0.3 POSTPONED (D6): enforce on m4m: timing with manager-lab (a latch); read-back: lab and stats readers see no client row, the distiller still reads its sessions.

## 1. Release A (tests first, each RED on today's main)

- [ ] 1.1 Principal counter by kind + `/v1/healthz/auth` (counts only); every header-accepted request logged with route and time.
- [ ] 1.2 Failed-credential limit across all routes, global (D4): over the limit a failing credential gets 429 + Retry-After; a valid credential is never refused; tokens under 32 chars refused at load.
- [ ] 1.3 Fingerprint check across tables (D7): a client hash equal to SHA256 of an ingest/reader token refuses the config; a restricted token stays restricted.
- [ ] 1.4 Webapp cookie login (D3): no token in local storage, a URL or a log (a stored one deleted); password-manager form; 401 brings the form back; vitest; frontend review for the visible change.
- [ ] 1.4b Persistent sessions (D3, security eff460e1): `browser_sessions` with sha256 ids and an HMAC token fingerprint under the hub's key file; scope from the current config at each lookup; 7 d idle / 30 d absolute; rotation on re-login; cap 5; `DELETE /v1/login` and `hub sessions revoke`; only readers log in; `__Host-` cookie. Tests: each of those, plus a restart under the same key keeps the session and another key does not.
- [ ] 1.5 Security TWO-vendor review of release A's code diff (eff460e1: a new session store), then release A, swap. Then infra removes `trust_tailscale_identity` from hub.toml and its template (R2); counts and header log show 0 header hits over one full cycle of the slowest caller.

## 2. Release B (code only)

- [ ] 2.1 Tests (RED today): the old key fails to load even when empty, and so does `HUB_TRUST_TAILSCALE_IDENTITY`; an unknown top-level key fails (deny_unknown_fields or sentinel); a forged header gets 401 on every registered read route, walked with a LAB COOKIE as the control (R5); every D2 caller still reads.
- [ ] 2.2 Remove the identity path from `auth.rs` and the config; docs (`deployment.md`, the spec).
- [ ] 2.3 Security re-check; release B; swap.

## 3. Backlog

- [ ] 3.1 File B1, B4, B5, B6 as Gitea issues (B2 and B3 are in release A, D3). B4 (cookie-only writes refused on every write route) lands before any write is ever allowed with the cookie (security be7f0c79); today every write needs a bearer token.
- [ ] 3.2 Security fe3fd22 P3 follow-ups as Gitea issues: F4 key file (no symlink, 0600 and owner checked, create without overwrite, "created" logged distinctly), F5 the 32-character rule on the env-var load path and on presented client tokens, F7 /v1/healthz/auth window fields only for a lab credential, F8 rotate any reader token the old webapp stored, state the cross-origin exception in the spec, F9 per-principal lock at login (cap under concurrency) and the missing tests (ingest token at login, a token moved between tables, cookie plus header); 38d9c6f: refuse `Sec-Fetch-Site: cross-site`, and `frame-ancestors 'none'` with B5.
- [ ] 3.3 Security e7e1e933 (release A re-check, OK) P3s as Gitea issues: read-only and client-reader BEARER writes refused in the layer with 403 (not a counted 401); `x-cchv-auth-failed` in CORS expose_headers; a length cap on Host and Origin in the header log line; an invalid bearer ends the request instead of falling through to the cookie and the header; a test of a live cookie plus the header.
