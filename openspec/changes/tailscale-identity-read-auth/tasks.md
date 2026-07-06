# Tasks: tailscale-identity-read-auth

## 1. Hub identity read-auth

- [ ] 1.1 Config: `trust_tailscale_identity: Vec<String>` (TOML key +
      `HUB_TRUST_TAILSCALE_IDENTITY` comma-separated in env mode)
- [ ] 1.2 State: carry the allow-list in `AppState` (update `new()` + test
      constructors)
- [ ] 1.3 Auth: `Authenticated` extractor accepts a trusted
      `Tailscale-User-Login` header when bearer auth is absent/invalid;
      `AuthedMachine` (ingest) unchanged
- [ ] 1.4 Tests: trusted header → 200 on reads; untrusted/empty-list → 401;
      ingest with header only → 401
- [ ] 1.5 cargo test/clippy/fmt green

## 2. Webapp auto-connect

- [ ] 2.1 `hubApi`: omit `Authorization` header when token is empty
- [ ] 2.2 `ConnectGate`: same-origin probe on mount (no stored config);
      2xx → render browser with page origin, persist nothing; failure →
      show gate
- [ ] 2.3 Vitest: auto-connect on probe success (nothing persisted); gate
      on probe failure; stored-config path unchanged
- [ ] 2.4 Frontend gate: tsc, vitest, lint

## 3. Ship

- [ ] 3.1 Rebuild + restage artifacts (`~/.config/cchv/staging/`,
      `~/.config/cchv/webapp/`), sanity-run
- [ ] 3.2 Archive openspec change; commits pushed (internal + origin)
- [ ] 3.3 Reply-relay to home-network with staged paths + the config line;
      archive both inbox messages
