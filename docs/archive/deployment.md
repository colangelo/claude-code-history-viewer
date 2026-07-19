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

## 2c. House deployment: swapping the m4m webapp (static-only)

A webapp-only bump (`dist-archive/` contents, no Rust change) is **much cheaper
than §2b**: the hub serves `static_dir` from disk per request, so there is **no
codesign step, no `launchctl` bootout/bootstrap, and no restart** — the next
request picks up the new files. Do not carry the binary-swap ceremony over to a
static bump.

> **The infra side automates all of this: `just cchv-webapp-deploy <version>`**
> (home-network `dd1aef2`, `tools/cchv-webapp-deploy`, documented in
> `hosts/m4m.md` § "cchv archive hub"). It accepts `0.10.3` / `v0.10.3` /
> `cchv-v0.10.3`, runs from either Mac (ssh-wraps itself when not on m4m), stages
> straight from the GitHub Release when nothing is staged locally, diffs
> **extracted trees** (below), takes a timestamped `mv` backup, enforces the
> post-swap assertions below, and **auto-restores the backup if verification
> fails** (keeping the bad tree at `staging/webapp-failed-<stamp>`). Nothing is
> ever deleted in either direction. So a relay handoff needs only "deploy
> vX.Y.Z" plus the release-artifact entry-chunk hashes — the manual steps below
> are the fallback/reference, not the expected path. Proven 2026-07-19 by an
> idempotent re-deploy of the live `v0.10.3` (all assertions green); the
> auto-rollback branch was exercised in a sandbox on m4m, not against prod.

> **"Staged at `~/.config/cchv/staging/…`" means *on m4m*, not on your Mac.**
> The recipe only ever looks at the hub host's filesystem. A tree staged on the
> build machine is invisible to it, so it silently takes the
> "nothing staged → stage from the GitHub Release" path and the
> staged-vs-released tree diff becomes a **no-op assertion** — safe (the release
> is the source of truth and exactly what gets deployed), but you did not get
> the check you thought you got. Either drop the staging claim from the relay
> and say "deploy the release for tag `cchv-vX.Y.Z`", or `scp -r` the tree to
> `m4m:~/.config/cchv/staging/webapp-cchv-vX.Y.Z` first so the diff actually
> fires. (Observed on the v0.10.4 deploy, 2026-07-19, thread 395b47ca.)

Validated on m4m 2026-07-19 (`cchv-v0.10.3`, thread 3fe4b63f):

```bash
cd ~/.config/cchv
STAMP=$(date +%Y%m%d-%H%M)
mv webapp staging/webapp-preswap-$STAMP-<oldversion>   # back up by moving, not copying
cp -R staging/webapp-<newversion> webapp
```

Rollback is the same two moves in reverse (and likewise needs no restart).

> **Provenance check: compare extracted trees, never tarball checksums.**
> To confirm a staged bundle matches the GitHub Release asset, download
> `cchv-webapp.tar.gz` for the tag, extract it, and `diff -r` the two trees. The
> two `.tar.gz` **sha256s will not match even for byte-identical contents** —
> gzip embeds metadata (mtime/name), so the archives differ while the trees are
> identical (observed on v0.10.3: `04f0397a` released vs `c800f11d` staged, trees
> `diff -r`-clean). This is the concrete form of the "tarball checksum is not
> reproducible" rule in §2's verify note.

Post-swap verification (no restart involved, so all of it is client-visible):

- the served entry chunks (`assets/archive-<hash>.js` / `.css`) equal the staged
  bundle's — these are the authoritative identity of the deploy
- the served entry chunk actually carries the version: probe it by the version
  chip's `title:"cchv-v<x.y.z>"` marker rather than by filename (a marker is
  stabler than a hashed asset name)
- a string unique to the new release is present (e.g. a new i18n key in
  `assets/i18n-en-<hash>.js`)
- `/v1/healthz` 200, `/v1/healthz/ingest?exclude=ac-mbp` 200, and the HTTPS
  front (`:8788`) 200 — **the `?exclude=` is not optional.** `ac-mbp` (the
  decommissioning Intel laptop) has a permanently stale ingest heartbeat
  (`last_seen` 2026-07-06), so the **bare** `/v1/healthz/ingest` is a standing
  503 today and every day. That is not an outage and must never roll a deploy
  back. The excluded host stays observable (`excluded:true`) but cannot flip the
  verdict; this is the same form the Gatus `cchv-ingest` check has used since
  hub `36870b4`.
- the asset-list diff vs the backup touches only the chunks the change should
  touch — a client-only patch that moves other chunks is a red flag

Visual/layout changes cannot be verified this way; a rendering claim needs a
human at a real window. Say so explicitly instead of marking it green.

> **Hub topology on m4m** (documented on the infra side in `hosts/m4m.md`): the
> hub binds `127.0.0.1:8790` — **not** 8787, which is taken by workerd — with
> tailnet ingress via `tailscale serve` on `:8788`. A failing loopback `:8787`
> probe is therefore *not* an outage.

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

### Rollout status (2026-07-19)

Steps 1–3 are **done and verified on m4m** (hub + daemon swapped, migration
`0003` applied, `/v1/identities` 200, v0.10.0 in the served entry chunk).
Step 2 on **ac-mbm5 is deferred** — the attended window closed before the swap
(infra relay, thread `8df6880`). No urgency: the Jul-11 daemon keeps working
against the new hub; the only consequence is ac-mbm5 project grouping /
`identity_key` lagging until it updates.

ac-mbm5 state as of the deferral (infra recon — no need to redo it): arm64,
macOS 26.5.2; daemon `~/.local/bin/cchv-sync-daemon` (Jul 11 build, 9.6M);
launchd label `dev.cchv.daemon` running, plist
`~/Library/LaunchAgents/dev.cchv.daemon.plist`; `cchv-launch` present at
`~/.local/bin/cchv-launch`; `~/.config/cchv/staging/` does **not** exist.

### Staging protocol for daemon-affecting releases

A staged binary is inert until swapped, so **stage every machine when the
release is cut**, not when someone happens to be at the keyboard — otherwise a
Mac→Mac ssh (1Password Touch-ID) round-trip burns an attended window. Cut the
release, stage, and relay the swap incantation with it; then any attended
session on either Mac executes the swap immediately.

Stage (from the release checkout, arm64 → arm64):

```bash
cargo build --release -p sync-daemon
REV=$(git rev-parse --short HEAD)
ssh ac-mbm5 'mkdir -p ~/.config/cchv/staging'
scp target/release/sync-daemon "ac-mbm5:~/.config/cchv/staging/cchv-sync-daemon-$REV"
```

Swap (attended, on the target machine — same codesign-aware shape as §2b:
rm-first, re-sign, `bootout`+`bootstrap`, never `kickstart -k`):

```bash
REV=<rev>
cp ~/.local/bin/cchv-sync-daemon ~/.local/bin/cchv-sync-daemon.bak.$(date +%Y%m%d)
launchctl bootout gui/$(id -u)/dev.cchv.daemon 2>/dev/null || true
rm -f ~/.local/bin/cchv-sync-daemon
cp ~/.config/cchv/staging/cchv-sync-daemon-$REV ~/.local/bin/cchv-sync-daemon
chmod 755 ~/.local/bin/cchv-sync-daemon
codesign --force --sign - ~/.local/bin/cchv-sync-daemon
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.cchv.daemon.plist
```

**Concurrent swaps.** Two agents staging/swapping the same daemon in the same
window is a real collision (it happened on m4m 2026-07-19: one session swapped
to `5cc660a` at 13:51 while another held the follow-up fix uncommitted in the
working tree — the second swap to `e419f4a` at 14:29 was correct only by luck of
ordering). Convention, cheap enough to always follow:

```bash
# claim before touching ~/.local/bin — fails if someone else holds it
LOCK=~/.config/cchv/staging/.swap-lock
( set -o noclobber; echo "$(id -un)@$(hostname -s) $(date -Iseconds) rev=$REV" > "$LOCK" ) \
  || { echo "swap in progress: $(cat "$LOCK")"; exit 1; }
trap 'rm -f "$LOCK"' EXIT
```

Whoever swaps also **commits and pushes the rev first** — a swapped binary whose
source is only in a working tree is unreconstructable. Identify what is actually
live (not what you think you staged) with a symbol probe rather than a hash: the
installed copy is re-signed after the copy, so its hash never matches the staged
file, e.g. `strings -a ~/.local/bin/cchv-sync-daemon | grep -c sessions_deferred`.

Verify from any tailnet host: the machine's rows gain `identity_key` after the
next scan pass —
`curl -s -H "Authorization: Bearer $TOKEN" "$HOST/v1/projects" | jq '[.[] | select(.identity_key != null)] | length'`.

> **That endpoint check proves *this* rev is live, and nothing later.** Every rev
> after it still serves `identity_key`, so a payload that carries a feature only
> tells you the binary is *at least* the rev that added it. Most perf revs (e.g.
> `aa16b77`'s daemon `search_text` clamp) are invisible to every response by
> construction. To confirm a specific rev, symbol-probe the installed file for a
> string that rev introduced — never a payload field, never the webapp version
> chip (static-only webapp deploys move the chip without touching a binary).

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

