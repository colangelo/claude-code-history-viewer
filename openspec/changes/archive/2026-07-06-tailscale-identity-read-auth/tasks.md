# Tasks: tailscale-identity-read-auth

## 1. Hub identity read-auth

- [x] 1.1 Config: `trust_tailscale_identity: Vec<String>` (TOML key +
      `HUB_TRUST_TAILSCALE_IDENTITY` comma-separated in env mode)
- [x] 1.2 State: carry the allow-list in `AppState` (update `new()` + test
      constructors)
- [x] 1.3 Auth: `Authenticated` extractor accepts a trusted
      `Tailscale-User-Login` header when bearer auth is absent/invalid;
      `AuthedMachine` (ingest) unchanged
- [x] 1.4 Tests: trusted header → 200 on reads; untrusted/empty-list → 401;
      ingest with header only → 401
- [x] 1.5 cargo test/clippy/fmt green

## 2. Webapp auto-connect

- [x] 2.1 `hubApi`: omit `Authorization` header when token is empty
- [x] 2.2 `ConnectGate`: same-origin probe on mount (no stored config);
      2xx → render browser with page origin, persist nothing; failure →
      show gate
- [x] 2.3 Vitest: auto-connect on probe success (nothing persisted); gate
      on probe failure; stored-config path unchanged
- [x] 2.4 Frontend gate: tsc, vitest, lint

## 3. Ship

- [x] 3.1 Rebuild + restage artifacts (`~/.config/cchv/staging/`,
      `~/.config/cchv/webapp/`), sanity-run
- [x] 3.2 Archive openspec change; commits pushed (internal + origin)
- [x] 3.3 Reply-relay to home-network with staged paths + the config line;
      archive both inbox messages
