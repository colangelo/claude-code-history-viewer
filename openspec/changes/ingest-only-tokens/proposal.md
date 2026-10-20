# Proposal

## Why

ac approved archiving the sessions of `agent1`, a fenced homelab agent guest, like the Macs'
sessions (asks a1020-03), with the condition **"ingest only; reading the archive from the
guest: no"**. Today every hub token is a full-scope machine token. The read extractor accepts
any valid bearer, so an ingest token for the guest would also read every machine's archived
history, Aruba client sessions included, and could write journal entries and distiller ticks.

## What Changes

- A token entry in `hub.toml` gains `ingest_only = true` (default `false`, so the Macs'
  tokens are unchanged).
- An ingest-only token is accepted by `/v1/ingest` and by nothing else. Every read route and
  every other machine-token write (`POST /v1/journal/entries`, `POST /v1/journal/ticks`)
  answers 401, the same as an unknown token.
- The default is deny: auth adds a separate `IngestMachine` extractor used only by ingest, so
  a new route built on the existing extractors refuses ingest-only tokens with no extra code.
- `scripts/cchv-launch.sh` renders agent1's ingest-only entry from the guest's own bao entry
  `kv/agents/cchv-agent1` (field `token`, 64 hex), with agent1's fixed machine id. A missing
  or malformed value renders the hub without agent1, so the Macs never depend on the guest;
  the shape check keeps a bad value from injecting TOML.

## Impact

- Code: `crates/hub/src/{auth,config,state,ingest,lib}.rs`, `scripts/cchv-launch.sh`.
- Specs: `archive-search-api` (authentication).
- Ops: infra creates `kv/agents/cchv-agent1` and grants the hub's AppRole read on it; a hub
  relaunch picks up the token. Release and swap are ac's call (asks a1021-01).
