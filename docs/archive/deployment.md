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

> **Deploy-request verify rule (house deployment).** When relaying a webapp swap
> to home-network (infra), quote the asset content-hash (`assets/archive-<hash>.js`)
> **only from the immutable released CI artifact** — never from a local `dist-archive/`
> build dir. Local rebuilds are not bit-reproducible, so a hash read from the build
> dir can go stale against the released tarball (this is how a stale `Cqi5MIOj` +
> `cac62595` pair got quoted once, costing a confirm round-trip). Corollary: a
> tarball sha1 is not reproducible either — use the content-hash from the release as
> authoritative, not the tarball checksum.

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

## 2b. House deployment: swapping the m4m hub binary

> The always-on m4m hub is **not** deployed from the GitHub Release. The path is:
> build locally → stage in `~/.config/cchv/staging/cchv-hub-<sha>` → relay
> home-network (infra) → binary swap. **Do not `cp` a new binary over the live
> one in place.** macOS caches the code signature per inode; overwriting in place
> with a differently-linker-signed binary trips the kernel's signature check and
> the process is killed on every spawn with `OS_REASON_CODESIGNING`. A hung
> `launchctl kickstart -k` then wedges the job in `spawn scheduled`.

Working sequence (validated on m4m 2026-07-13, thread 7938448b):

```bash
STAGED=~/.config/cchv/staging/cchv-hub-<sha>          # the new binary
LIVE=/usr/local/bin/cchv-hub                          # whatever the plist ExecStart points at
STAMP=$(date +%Y%m%d-%H%M)

# 1. Back up the currently-live binary (to staging, timestamped).
cp "$LIVE" ~/.config/cchv/staging/cchv-hub-preswap-$STAMP

# 2. rm the old binary FIRST — do not cp over it (inode codesign cache).
rm "$LIVE"

# 3. cp the staged binary → a fresh inode.
cp "$STAGED" "$LIVE"

# 4. Re-sign ad-hoc (the kernel rejects the cached signature otherwise).
codesign --force --sign - "$LIVE"

# 5. bootout + bootstrap — NOT `kickstart -k` (which can wedge in
#    "spawn scheduled"). If a prior kickstart hung, kill it first.
launchctl bootout  gui/$(id -u)/dev.cchv.hub 2>/dev/null || true
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.cchv.hub.plist
```

Verify: `curl -s https://m4m.cat-bluegill.ts.net:8788/v1/healthz` → `{"status":"ok",…}`
and the process is running clean (fresh pid, no respawn churn).

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
    <!-- house deployment (§3b): mark launchd starts headless so the launcher
         skips the interactive `op` fallback (never prompt Touch-ID under KeepAlive) -->
    <key>CCHV_NONINTERACTIVE</key><string>1</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <!-- launchd-resilience contract: cap KeepAlive respawn churn (default floor
       is 10s → ~8.6k respawns/night on a fast-failing job) to 5 min. -->
  <key>ThrottleInterval</key><integer>300</integer>
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
  `DAEMON_CONFIG`/`HUB_CONFIG` there. Keep the `# TEMPLATE — do NOT put real
  secrets here…` header comment at the top of both template files when
  (re)deploying: a bare `hub_token` placeholder line reads like failed
  substitution to anyone (or any agent) inspecting the file (relay 2026-07-11).
  The launcher strips that leading comment block at render time and stamps a
  `# RENDERED … DO NOT EDIT` header on the runtime file instead, so each file
  self-describes truthfully. Caution: the launcher rejects renders still
  matching `@[A-Z_]*@`, and non-leading comments survive the render — so
  comments below the header must not contain literal all-caps at-sign markers.
- **OpenBao source of truth**: `kv/infra/cchv/pg1` (hub DB creds) and
  `kv/infra/cchv/hub-tokens` (`<host>_token`, `<host>_machine_id`). Read via
  AppRole `cchv-daemon` (policy `cchv-read`, token TTL 15m — fine, the token is
  only used for the reads at launch).
- **Per-machine setup (once)**: materialize the AppRole creds file
  `~/.config/cchv/bao-approle` (`role_id=…` / `secret_id=…`, chmod 0600) from
  1P item `openbao - cchv-daemon approle` (vault `AC-DevOps`), install the
  script to `~/.local/bin/cchv-launch`, and point the plist's
  `ProgramArguments` at `cchv-launch daemon` (drop the `DAEMON_CONFIG` env —
  the launcher sets it). Keep the plist's `CCHV_NONINTERACTIVE=1` and
  `ThrottleInterval=300` (above) — they make the launcher conform to the house
  launchd-resilience contract (`macos-setup docs/launchd-resilience.md`).
- **Fallbacks** (launchd-resilience-conformant): bao is skipped when the tailnet
  name doesn't resolve (MagicDNS down at wake — no point eating curl timeouts).
  `op read` is tried **only in an attended start**; under launchd it's skipped
  (no tty / `CCHV_NONINTERACTIVE=1`) so a down-tailnet reboot can't storm
  Touch-ID/TCC prompts. When both are unavailable the launcher reuses the
  previous runtime render (last-known-good) and logs a warning — a clean idle,
  not a crash-loop. `ThrottleInterval` caps `KeepAlive` respawn churn to 5 min.
  (Regression origin: 2026-07-08 m4m tailnet-down prompt storm — see CHANGELOG.)
- **Rotation**: rotate in 1P, re-copy to bao per home-network
  `docs/secrets-standard.md`, then `launchctl unload/load` the job — the next
  launch re-renders.

## 3c. Journal-entries distiller (`scripts/cchv-distill.py`)

> Daily launchd job on the hub machine (m4m) that distills archived sessions
> into per-(date, project) journal entries (openspec `journal-entries`,
> issue #12). Catch-up-based: the work list is `GET /v1/journal/pending`
> (missing or dirty groups), so sleep/downtime and late-arriving syncs only
> delay entries. **Install only after the hub carries the journal endpoints**
> (migration `0002_journal_entries.sql`).

- **Install** (on m4m):

  ```bash
  install -m 755 scripts/cchv-distill.py ~/.local/bin/cchv-distill
  cp scripts/dev.cchv.distiller.plist ~/Library/LaunchAgents/
  launchctl load ~/Library/LaunchAgents/dev.cchv.distiller.plist
  ```

  Requires `uv` on PATH (PEP 723 script). **LLM backend (default `aiproxy`):**
  an OpenAI-compatible HTTP call to infra's CLIProxyAPI node
  (`https://aiproxy.cat-bluegill.ts.net`, model **`gpt-5.6-sol`**,
  `reasoning_effort=low`) — no `claude -p`, so no shared-OAuth contention (the
  old #13 failure mode when an interactive Claude session ran concurrently).
  `--backend claude` keeps `claude -p` (needs the `claude` CLI + `zsh -lc`
  `CLAUDE_CONFIG_DIR`) as a fallback. Runs daily 05:30 + on load; logs
  `/tmp/cchv-distiller.{log,err}`.
- **Secrets** (both same env → bao → 0600-cache floor shape):
  - **Hub token** — `$CCHV_HUB_TOKEN` → AppRole reading
    `kv/infra/cchv/hub-tokens/<host>_token` → `~/.config/cchv/distill-hub-token`
    cache. Authorizes the hub reads + `POST /v1/journal/entries`.
  - **aiproxy key** (backend=aiproxy) — `$CCHV_AIPROXY_KEY` → AppRole reading
    `kv/infra/aiproxy/proxy-keys` field `agents` → `distill-aiproxy-key` cache.
    `kv/infra/aiproxy/*` is infra-owned; the `cchv-daemon` AppRole was granted
    read on it (home-network 38e48d8, `cchv-read` policy; relay 2026-07-19), so
    the headless bao read now self-heals past key rotation. The cache floor
    remains only as a bao/DNS-flake fallback, no longer the load-bearing path.
  - `op read` is the attended-only fallback (skipped under launchd,
    `CCHV_NONINTERACTIVE=1`, so it can't storm Touch-ID).
- **Forward mode** (the launchd default) only processes groups newer than
  `--horizon-days` (7). **Backfill is deliberate and bounded** — never
  automatic:

  ```bash
  # newest-first, resumable; re-run to continue where the last chunk stopped
  cchv-distill --backfill --limit 20
  cchv-distill --backfill --from 2026-05-01 --limit 50
  cchv-distill --dry-run            # inspect an entry without writing
  ```

  Chunk the historical sweep (9 months of archive) and check quota + entry
  quality (`GET /v1/journal/entries`, `cchv-find` eval) between chunks.
- **Failure semantics**: schema-invalid LLM output is rejected locally and the
  group stays pending (retried next run); the hub validates independently.
  Exit code 1 when any group failed — visible in `/tmp/cchv-distiller.err`.

## 3d. Project identity (cchv-v0.10.0): rollout order

The git-fingerprint identity feature (migration `0003`, `identity:<key>`
filters, `/v1/identities` + aliases) is fully additive and order-independent,
but the intended rollout is **hub first, then daemons**:

1. **Hub**: swap per §2b. Migration `0003` auto-runs at startup (nullable
   columns + `project_identity_aliases` table — existing rows stay valid with
   NULL fingerprints; a rollback binary simply ignores them).
2. **Daemons** (m4m, ac-mbm5): swap per §3. No config change — the next scan
   pass captures git fingerprints for every live project dir (guarded,
   5s-timeboxed `git` subprocesses; failures degrade to no-fingerprint) and
   the normal upsert backfills the columns. Old daemons against the new hub
   (and vice versa) keep working: absent facts never clobber stored ones.
3. **Webapp**: ships in the same release bundle; the identity-grouped sidebar
   and worktree toggle appear once the hub exposes the new fields.

Moved-away paths archived before fingerprinting exist can't be fingerprinted
retroactively — link them from the webapp (orphan-path suggestions on the
identity's member panel create a reversible alias; nothing rewrites archived
rows).

## 4. Verify end-to-end

```bash
# From any machine on the tailnet:
TOKEN=GENERATE_A_LONG_RANDOM_SECRET_FOR_MBP
HOST=http://<tailnet-host>:8787

curl -s -H "Authorization: Bearer $TOKEN" "$HOST/v1/projects" | jq '.[0]'
curl -s -H "Authorization: Bearer $TOKEN" "$HOST/v1/search?q=refactor" | jq '.results[0]'

# Project identity (after 3d): fingerprints + identity grouping
curl -s -H "Authorization: Bearer $TOKEN" "$HOST/v1/projects" \
  | jq '[.[] | select(.identity_key != null)] | length'   # fingerprinted rows
curl -s -H "Authorization: Bearer $TOKEN" "$HOST/v1/identities" \
  | jq '.[0] | {identity_key, display_name, members: [.members[].project_path]}'
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
  without a breaking migration. **pg1 disk envelope for the embeddings backfill**
  (infra note, home-network relay 2026-07-19, `hosts/configs/proxmox1/pg1.md`):
  the pg1 data disk was pre-grown 32 → 48 GB online, so there is now **~24 GB
  free**. pgvector **0.8.5** is installed and **`halfvec` is verified working**.
  Sizing the migration: 768-dim `halfvec` (~6–7 GB) + HNSW slack fits comfortably;
  768-dim f32 (~12.2 GB) fits with room; 1536-dim f32 (~24.2 GB) does **not** fit —
  use `halfvec` or ask infra for another grow first. Budget the **nightly pg_dump**
  growth too, not just heap+HNSW: backups live on the same disk with 14-day
  retention, so a +6–7 GB embeddings table inflates every dump. Send infra the firm
  dimension/type when scoped and they will re-check; tens-of-GB grows are fine on
  request, 100 GB+ needs a `/mnt/state` cleanup conversation first.
- **Desktop release:** the repo is now a Cargo workspace, so build artifacts
  live in the repo-root `target/` (not `src-tauri/target/`). The release
  workflows were updated accordingly — verify at the next desktop release.
```

