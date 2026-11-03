## 1. Measure (release A)

- [ ] 1.1 Test first: a principal counter by kind; `/v1/healthz/auth` returns counts only (no login, no token, no path).
- [ ] 1.2 Test first: failed `/v1/login` attempts are rate-limited per source (10 per 10 min, then 429 + Retry-After); successes are not counted.
- [ ] 1.3 Webapp: ConnectGate logs in through `/v1/login` (cookie), no same-origin header probe, deletes a stored token; vitest for the flow; frontend review (house rule) for the visible change.
- [ ] 1.4 Release A, swap; infra confirms Gatus's checks and their credentials (D2 row).
- [ ] 1.5 24 h of counts: every `tailscale` hit traced and moved to a credential.

## 2. Drop the trust (release B)

- [ ] 2.1 Test first: a forged `Tailscale-User-Login` gets 401 on every registered read route; serve-style request with no credential gets 401; every D2 caller's credential still reads its routes.
- [ ] 2.2 Remove the identity path from `auth.rs`; `trust_tailscale_identity` refused at start (D5); docs (`deployment.md`, the spec).
- [ ] 2.3 Security two-vendor review of the diff; release B; infra removes the key from hub.toml in the same swap.

## 3. Enforce

- [ ] 3.1 Test exists (route-walk under enforce): a lab principal sees no client row on any route.
- [ ] 3.2 infra sets `[client_scope] enforce = true` on m4m; read-back: lab and stats readers see no client row; the distiller still reads its sessions.
