# Design

## Decisions

- **A flag on the token entry, not a scope list.** One boolean covers the only need (a writer
  that must not read). A general scope system is not justified by one guest.
- **Deny by default.** `AuthedMachine` and `Authenticated` refuse ingest-only tokens;
  `IngestMachine` (ingest only) accepts them. A future route has to opt in explicitly.
- **401, not 403.** An ingest-only token on a read route gets the same answer as an unknown
  token, so the response does not confirm the token is valid.
- **State shape.** `AppState.ingest_only: HashSet<String>` beside the existing token map, set by
  a builder (`with_ingest_only_tokens`), so `AppState::new` and its many test callers are
  unchanged.
- **One source for the guest's token.** The hub reads the guest's own entry instead of a copy
  in `kv/infra/cchv/hub-tokens`, which the guest must not read (it holds every machine's
  token). A rotation is one write; a hub relaunch picks it up.
