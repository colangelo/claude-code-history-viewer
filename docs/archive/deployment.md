# Cross-machine history archive — deployment

A durable, searchable archive of your AI coding history across every machine.
It solves the problem that Claude Code (and others) delete local history after a
fixed window: once a message reaches the archive it stays, even after the local
file is gone.

## Architecture

```
each machine:  sync-daemon ──(HTTPS over Tailscale, bearer token)──▶  hub ──▶ Postgres
                  │                                                    │
                  └─ reads ~/.claude, ~/.codex, …                      └─ /v1/ingest /v1/search /v1/projects …
                     via the shared history-core parser                   (the ONLY component with DB creds)
```

- **hub** (`crates/hub`) — the only component that touches Postgres. Exposes
  bearer-authenticated `/v1/ingest` (idempotent upserts) and a read API
  (`/v1/search`, `/v1/projects`, `/v1/sessions`, `/v1/sessions/{id}/messages`).
- **sync-daemon** (`crates/sync-daemon`) — runs on each machine, backfills then
  incrementally pushes local history to the hub. Holds only a hub URL + token.
- **Postgres** — normalized + raw-fidelity + full-text-searchable storage.
  Designed to add pgvector later without a breaking migration.

The archive is **cumulative**: the daemon only ever ingests; deleting a local
file never deletes anything from the hub.

## 1. Postgres

> **House deployment (this homelab): use the shared pg1, not a self-provisioned
> Postgres.** Follow `~/_sync/dev/CONTEXT/PATTERNS/shared-backends.md`: ask infra
> (home-network agent, via the relay) to provision role `cchv` + db `cchv_archive`
> on pg1; the credential lands in 1Password as `cchv - app role @ pg1` (vault
> `AC-DevOps`); connect via `pg1.cat-bluegill.ts.net:5432`. You inherit pg1's
> nightly logical backups + PVE backups for free. Never put literal passwords or
> tokens in `hub.toml` committed anywhere — they are resolved **at launch, bao-first**
> by `scripts/cchv-launch.sh` (see "House deployment: bao-first secrets" below).
> The generic instructions below are for deployments outside the homelab. The
> local dev/test setup (CI `postgres` service containers, `cchv_archive_dev/_test`)
> is unaffected — the shared-backend rule concerns the *deployed* archive only.

Generic (non-homelab) setup:

```bash
# Create a database and a role the hub will use.
createdb cchv_archive
psql -d cchv_archive -c "CREATE ROLE cchv LOGIN PASSWORD 'CHANGE_ME';"
psql -d cchv_archive -c "GRANT ALL ON DATABASE cchv_archive TO cchv;"
```

The hub applies the migrations in `migrations/` automatically on startup, so no
manual migration step is required.

## 2. Hub (on the always-on tailnet node)

Build it:

```bash
cargo build --release -p hub
# binary: target/release/hub
```

Create a config file (`/etc/cchv/hub.toml`). The `tokens` table maps a bearer
token to the machine id it authenticates — one entry per machine:

```toml
database_url = "postgres://cchv:CHANGE_ME@localhost/cchv_archive"
bind_addr = "0.0.0.0:8787"   # reachable over the tailnet

[[tokens]]
token = "GENERATE_A_LONG_RANDOM_SECRET_FOR_MBP"
machine_id = "11111111-1111-1111-1111-111111111111"
label = "mbp"

[[tokens]]
token = "GENERATE_A_LONG_RANDOM_SECRET_FOR_M4M"
machine_id = "22222222-2222-2222-2222-222222222222"
label = "m4m"
```

> The `machine_id` here must match the id the daemon reports (see step 3 — the
> daemon prints its id on first run, or you can pre-seed it).

Run it (systemd unit, Linux node):

```ini
# /etc/systemd/system/cchv-hub.service
[Unit]
Description=CCHV archive hub
After=network-online.target postgresql.service

[Service]
Environment=HUB_CONFIG=/etc/cchv/hub.toml
Environment=RUST_LOG=info
ExecStart=/usr/local/bin/hub
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable --now cchv-hub
curl http://<tailnet-host>:8787/v1/healthz   # {"status":"ok","db":"up"}
```

Transport security is provided by Tailscale (WireGuard); the bearer token gates
access. TLS termination (e.g. behind a reverse proxy) can be added later.

### Optional: serve the static archive browser at `/`

The hub can host the standalone archive webapp (a backend-free build of the
viewer's Archive mode) so one process serves both the UI and the API —
same-origin, so no CORS or mixed-content concerns:

```bash
just archive-web-build        # in the cchv repo → dist-archive/
```

Then either add to `hub.toml`:

```toml
static_dir = "/path/to/dist-archive"
```

or set `HUB_STATIC_DIR=/path/to/dist-archive` when running from env vars
(TOML mode ignores env, same precedence as every other hub setting). `/v1/*`
routes always win over static files; static assets are served without auth
(the bearer token still gates all data endpoints). Unset = `/` stays 404,
exactly the pre-static behavior. First visit shows a connect screen (hub URL
+ read token, persisted in that browser's localStorage) — with same-origin
hosting the URL is just the page's own origin.

> **House deployment:** bind the hub to the node's tailscale IP (not `0.0.0.0`),
> follow the tailnet-services pattern (`~/_sync/dev/CONTEXT/PATTERNS/
> tailnet-services.md` — ideally Tailscale Serve `:443` for in-tailnet TLS), and
> wire a Gatus uptime check on `/v1/healthz` per `PATTERNS/monitoring.md` when it
> goes live. Bearer tokens live in OpenBao (`kv/infra/cchv/hub-tokens`; 1Password
> vault `AC-DevOps` is the human vault + fallback), referenced by path/item title,
> never committed.

## 3. Sync daemon (on each machine)

Build it:

```bash
cargo build --release -p sync-daemon
# binary: target/release/sync-daemon
```

Config (`~/.config/cchv/daemon.toml`):

```toml
hub_url = "http://<tailnet-host>:8787"
hub_token = "GENERATE_A_LONG_RANDOM_SECRET_FOR_MBP"  # this machine's token
scan_interval_secs = 3600
```

The daemon persists a stable machine id at `~/.claude-history-sync/machine_id`
on first run and prints it. Put that id in the hub's `hub.toml` for this
machine's token (or pre-create the file with a chosen UUID before first run).

Install (launchd, macOS):

```xml
<!-- ~/Library/LaunchAgents/dev.cchv.daemon.plist -->
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
  <key>Label</key><string>dev.cchv.daemon</string>
  <key>ProgramArguments</key>
  <array><string>/usr/local/bin/sync-daemon</string></array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>DAEMON_CONFIG</key><string>/Users/YOU/.config/cchv/daemon.toml</string>
    <key>RUST_LOG</key><string>info</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
</dict></plist>
```

```bash
launchctl load ~/Library/LaunchAgents/dev.cchv.daemon.plist
```

On Linux, a systemd **user** service with `Environment=DAEMON_CONFIG=…` works
equivalently.

## 3b. House deployment: bao-first secrets (`scripts/cchv-launch.sh`)

> Homelab machines don't run the binaries directly. Both launchd jobs run
> `~/.local/bin/cchv-launch <daemon|hub>` (installed from
> `scripts/cchv-launch.sh`), which resolves secrets at every launch —
> **OpenBao-first, `op read` fallback, last-known-good cache as the floor** —
> renders a 0600 runtime config, and `exec`s the binary. Flipped 2026-07-05
> (home-network #17); live on m4m for both `dev.cchv.daemon` and `dev.cchv.hub`.

- **Templates, not secrets, on disk**: `~/.config/cchv/daemon.toml` and
  `hub.toml` are templates with `@HUB_TOKEN@` / `@DB_PASSWORD@` /
  `@M4M_TOKEN@` / `@AC_MBM5_TOKEN@` placeholders. The launcher renders them to
  `daemon.runtime.toml` / `hub.runtime.toml` (0600) and points
  `DAEMON_CONFIG`/`HUB_CONFIG` there.
- **OpenBao source of truth**: `kv/infra/cchv/pg1` (hub DB creds) and
  `kv/infra/cchv/hub-tokens` (`<host>_token`, `<host>_machine_id`). Read via
  AppRole `cchv-daemon` (policy `cchv-read`, token TTL 15m — fine, the token is
  only used for the reads at launch).
- **Per-machine setup (once)**: materialize the AppRole creds file
  `~/.config/cchv/bao-approle` (`role_id=…` / `secret_id=…`, chmod 0600) from
  1P item `openbao - cchv-daemon approle` (vault `AC-DevOps`), install the
  script to `~/.local/bin/cchv-launch`, and point the plist's
  `ProgramArguments` at `cchv-launch daemon` (drop the `DAEMON_CONFIG` env —
  the launcher sets it).
- **Fallbacks**: if bao is unreachable, the launcher tries
  `op read` (may need Touch ID — fine for attended starts); if that fails too,
  it reuses the previous runtime render and logs a warning. KeepAlive retries
  cover transient outages.
- **Rotation**: rotate in 1P, re-copy to bao per home-network
  `docs/secrets-standard.md`, then `launchctl unload/load` the job — the next
  launch re-renders.

## 4. Verify end-to-end

```bash
# From any machine on the tailnet:
TOKEN=GENERATE_A_LONG_RANDOM_SECRET_FOR_MBP
HOST=http://<tailnet-host>:8787

curl -s -H "Authorization: Bearer $TOKEN" "$HOST/v1/projects" | jq '.[0]'
curl -s -H "Authorization: Bearer $TOKEN" "$HOST/v1/search?q=refactor" | jq '.results[0]'
```

## Notes & current limitations

- **`raw` fidelity (MVP):** the archived `raw` JSONB is the normalized record
  (lossless for all modeled fields). Byte-exact original passthrough is a
  planned enhancement.
- **Incremental sync (MVP):** a changed session file is re-parsed in full and
  re-sent; the hub's idempotent ingest drops duplicates. Byte-offset
  "parse only new lines" and `notify`-based watching are planned optimizations.
- **pgvector / semantic search and an MCP context server** are future phases;
  the schema already reserves a `message_embeddings` side table so they land
  without a breaking migration.
- **Desktop release:** the repo is now a Cargo workspace, so build artifacts
  live in the repo-root `target/` (not `src-tauri/target/`). The release
  workflows were updated accordingly — verify at the next desktop release.
```

