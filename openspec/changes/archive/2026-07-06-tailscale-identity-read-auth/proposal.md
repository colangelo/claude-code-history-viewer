# Proposal: tailscale-identity-read-auth

## Why

The archive browser is live behind Tailscale serve
(`https://m4m.cat-bluegill.ts.net:8788/`), but every browser still hits the
ConnectGate token prompt. Serve already injects verified
`Tailscale-User-Login` headers for tailnet clients, so the hub can grant
read access to allow-listed identities and the webapp can connect
same-origin with zero prompts. Asked by the user via home-network relay
`2026-07-06-1246` (thread `2026-07-06-0420...`); headers chosen over a
tsidp/OIDC flow for simplicity — same identity source, no extra moving parts.

## What Changes

- Hub: new opt-in `trust_tailscale_identity = ["login", …]` config (TOML key;
  `HUB_TRUST_TAILSCALE_IDENTITY` comma-separated in env mode). When the
  request carries a `Tailscale-User-Login` header matching the allow-list,
  READ endpoints accept it as authentication. Bearer tokens unchanged;
  ingest stays bearer-only (needs a machine identity). Default empty =
  today's behavior.
- Webapp: on load with no stored config, probe same-origin
  (`GET /v1/projects?limit=1`, no token); 2xx → skip the ConnectGate and use
  the page origin. Gate remains the fallback for token-based access.
- `hubApi` omits the `Authorization` header when the token is empty.

Accepted threat (per infra): local processes on the hub host can hit the
loopback port and spoof the header — same-user machine, they can already
read the transcripts on disk.

## Capabilities

### New Capabilities

<!-- none -->

### Modified Capabilities

- `archive-search-api`: read-endpoint authentication accepts trusted
  Tailscale identity headers in addition to bearer tokens (opt-in).
- `static-archive-webapp`: connect flow auto-connects same-origin when the
  host authenticates the request; gate becomes the fallback.

## Impact

- `crates/hub`: `config.rs` (new key), `state.rs` (trusted list),
  `auth.rs` (`Authenticated` extractor), tests.
- Frontend: `ConnectGate.tsx` (auto-probe state), `hubApi.ts` (conditional
  auth header), vitest.
- Deployment: hub.toml template on m4m + binary swap (infra flips it;
  staged-artifacts relay like 0420).
