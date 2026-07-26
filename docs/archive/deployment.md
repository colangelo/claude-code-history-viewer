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
> `AC-DevOps`) — **being superseded** by the bao-owned, self-rotating
> `database/static-creds/cchv-svc` (home-network #31; read path and cutover in
> §3b), after which that 1P item is historical and must not be read; connect via
> `db.internal:5432`. You inherit pg1's
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

### pg1 analytics schema (`0005`) — applied 2026-07-25, infra-verified

The analytics migration (`message_id` column on `messages`, plus
`message_tool_uses` / `message_tool_results`) and its backfill were executed
against the live pg1 `cchv_archive` under running ingest, from the
`feature/hub-analytics` lane (task 3.5; results in
`openspec/changes/hub-analytics/design.md`). **Infra re-ran the counts directly
against the live db rather than taking our report on trust** (relay `3c57d8ee`,
home-network `073abc3`) and every figure matched: `message_tool_uses` 132,342 ·
`message_tool_results` 132,168 · `messages.message_id NOT NULL` 271,752 · both
new tables present · `/var/lib/postgresql` at 14G/47G (30%).

Three things that outlive the ack:

- **No monitoring or backup change is warranted for an additive schema.** The
  `cchv-*` Gatus checks and pg1's nightly logical dump operate at a level new
  columns and tables don't touch, so a migration like this needs no infra
  follow-up. The rollback one-liner stays valid:
  `UPDATE messages SET message_id=NULL; TRUNCATE message_tool_uses, message_tool_results;`
- **The hub binary swap is still owed as a normal deploy relay** (§2b) when
  Deliverable 1 lands, together with the second backfill sweep that catches rows
  the *old* binary ingested in the meantime (task 8.4). Infra has this queued as
  expected and has nothing pending on their side — so the swap arrives through
  the usual §2b path, not as a surprise.
- **Headless access to pg1 must use the FQDN** `db.internal`: bare
  `ssh pg1` fails host-key verification on m4m (no `Host pg1` alias, and
  `known_hosts` holds the key under the FQDN). Infra owns that alias; nothing to
  change here, since `scripts/cchv-launch.sh` and the §1 note above already use
  the FQDN — but a future script that shortens it would break only when run
  unattended.

### `backfill-analytics` and `mirror rebuild` travel together

**Any run of `hub backfill-analytics` must be followed by `hub mirror rebuild`.**
This is not housekeeping — skipping it makes the statistics endpoints report
inflated token totals, silently and indefinitely.

Why: the DuckDB statistics mirror (`~/.config/cchv/stats-mirror.duckdb`) marks,
for each logical message, which stored row carries its usage — so a response
split across several rows is counted once. Incremental refresh maintains that
correctly for *inserts*, because row ids only ever grow. `backfill-analytics` is
an `UPDATE messages SET message_id = …` over **existing** rows, which regroups
them in Postgres while the mirror still holds them under their old grouping.
The affected rows then each count their own usage. Design D2 of
`openspec/changes/hub-stats-duckdb-mirror/` has the full argument.

```bash
export HUB_CONFIG=~/.config/cchv/hub.runtime.toml
hub backfill-analytics        # idempotent, resumable
hub mirror rebuild            # required after the above
```

`mirror rebuild` builds a complete new mirror at `<path>.rebuild-<stamp>` and
renames it into place, so **the running hub keeps answering `/v1/stats/*` from
the old mirror for the whole build** — unlike deleting the file, which would
cause a multi-minute `503 warming`. The live hub notices the swap on its next
refresher tick (default 300 s) and reopens; no restart is needed, and none
should be issued mid-rebuild.

The rebuild is also the recovery path for any other Postgres-side `UPDATE` of a
mirrored column, and for a mirror suspected of being wrong for any reason: it is
derived state, so rebuilding it can never lose data.

One lesson from the migration is **deliberately not yet written up as a house
standard**: `0005` must take its locks *up front, in the ingest writer's order*,
or migration and live ingest deadlock — and `lock_timeout` does **not** save you,
because Postgres' deadlock detector fires first. Infra flagged it as a
`CONTEXT/PATTERNS` candidate but declined to author it from a headless poller;
it needs an attended session (see `81683f7b` on the lane for the fix itself).

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

### Optional: semantic journal search (embed model directory)

Semantic + hybrid `mode=` on the `/v1/search` journal leg needs a local
sentence-embedding model on disk — the hub embeds in-process on CPU (no
network, no keys). Absent/broken directory = keyword-only with
`journal_degraded: true` on semantic requests, never an outage.

Stage the model once (bge-small-en-v1.5, ~128 MB total):

```bash
MODEL_DIR=~/.config/cchv/embed-model   # any path; m4m house convention
mkdir -p "$MODEL_DIR" && cd "$MODEL_DIR"
for f in config.json tokenizer.json model.safetensors; do
  curl -sL --fail -O "https://huggingface.co/BAAI/bge-small-en-v1.5/resolve/main/$f"
done
```

**Wire it on the branch the hub actually uses — this is a hard either/or, not
two interchangeable options, and the wrong branch fails silently:**

- **`HUB_CONFIG` is set → TOML only. The env var is ignored entirely.**
  `HubConfig::load()` returns as soon as it has parsed the file; it never
  merges env. Put `embed_model_dir = "<MODEL_DIR>"` in that TOML.
  The house m4m LaunchAgent is this branch — `cchv-launch hub` renders
  `hub.runtime.toml` and exports `HUB_CONFIG`, so exporting
  `HUB_EMBED_MODEL_DIR` there does nothing.
- **`HUB_CONFIG` unset → env only.** Set `HUB_EMBED_MODEL_DIR=<MODEL_DIR>`
  (alongside `DATABASE_URL` etc.).

Setting the wrong one yields a *fully green* deploy — clean `healthz`, green
Gatus check, no error line anywhere — with semantic search silently off. The
only honest check is a paraphrase probe (below). Same precedence as every
other hub setting (`static_dir`/`HUB_STATIC_DIR`, …).

The model loads lazily on first use; a background sweep embeds journal
entries at startup, on an interval, and after journal writes. **Budget tens of
seconds for the first sweep, not "a few"** — CPU embedding of 101 entries took
~37 s on m4m (model loaded → `embedding sweep pass` logged once with
`embedded=101 failed=0`). The gap reads like a hang; it is not, do not roll
back through it. Embeddings are derived data — `DELETE FROM
journal_embeddings` is always safe (the sweep regenerates).

Verify it is really on — deploy assertions and symbol probes cannot see a
feature wired off, so run a paraphrase probe whose wording does not appear in
the text (`scope=journal`): `mode=keyword` should return **0** hits and
`mode=semantic`/`mode=hybrid` should return several. The keyword zero is what
makes the semantic hits mean anything. Also check `mode=` omitted still
returns the old keyword result, byte-compatible.

**`journal_degraded` is ABSENT on healthy responses, not `false`** (the field
is `skip_serializing_if = "Option::is_none"`, so pre-`mode=` responses stay
byte-identical). Grepping for `"journal_degraded": false` finds nothing on a
healthy hub — absence is the healthy reading; presence with `true` means the
semantic leg fell back to keyword.

> **House deployment:** bind the hub to the node's tailscale IP (not `0.0.0.0`),
> follow the tailnet-services pattern (`~/_sync/dev/CONTEXT/PATTERNS/
> tailnet-services.md` — ideally Tailscale Serve `:443` for in-tailnet TLS), and
> wire a Gatus uptime check on `/v1/healthz` per `PATTERNS/monitoring.md` when it
> goes live. Bearer tokens live in OpenBao (`kv/infra/cchv/hub-tokens`; 1Password
> vault `AC-DevOps` is the human vault + fallback), referenced by path/item title,
> never committed.

## 2b. House deployment: swapping the m4m hub binary

> **Ship the binary as a release asset with a `.sha256` — that is the default
> path for a tagged release.** (Was: "not deployed from the GitHub Release";
> that is now false for tagged releases. Established on the v0.11.2 deploy,
> 2026-07-20, thread 51a74a49.) The workflow attaches
> `cchv-hub-<version>-aarch64-apple-darwin` (`<version>` = the tag minus the
> `cchv-v` prefix) plus its `.sha256` to the `cchv-vX.Y.Z` release — that exact
> string is what to grep the release for; infra downloads it, checks the digest
> against the asset's `.sha256` *and* the GitHub API's own `digest` field, and
> stages it at `~/.config/cchv/staging/`. This removes the local cargo build,
> the cross-Mac `scp`, and — the expensive one — the "swap `<sha>`" ambiguity
> between *pushed* and *actually staged on m4m* that has cost round-trips
> before. Only when no release asset exists does the old path apply: build
> locally → stage on m4m → relay → swap. **Proven end-to-end on `cchv-v0.12.0`**
> (2026-07-21, thread 3dc909f8): the first tag after the CI change, all three
> assets uploaded by `github-actions[bot]`, and infra's `shasum -a 256 -c`
> consumed the `.sha256` straight from download with no massaging.
>
> **The green that authorizes a deploy is scoped to the workflow that builds
> the asset.** What made a red `Rust Tests` on `main` a non-stopper for the
> v0.17.0 swaps was provenance, not seniority: the deployed assets come out of
> `server-release.yml`, a separate and green workflow, so the red run never
> touched a shipped byte. The moment a deploy asset starts coming out of a red
> workflow, that red IS a deploy stopper — re-derive the scoping whenever the
> CI layout changes instead of caching the conclusion. (Infra flag on the
> v0.17.0 thread `e05b6f2e`, msg `910ee098`; recorded on their side in
> `hosts/m4m.md` with the failure's named owner.) The re-derive clause got its
> worked counterexample the same day: the very commit that fixed the red
> (`258345fa`) also moved cargo audit off the push gate into
> `security-audit.yml`, so any workflow set these records name is one reshuffle
> stale — at swap time, read the tag's checks fresh
> (`gh run list -R "$FORK" --commit <tag-sha>`) instead of trusting a cached
> list. (Infra ack `3c7909c9`, same thread; their `hosts/m4m.md` entry says the
> same out loud.)
>
> **The digest proves what was installed, never which rev it is.** A macos-14
> runner build and a local Mac build of the same rev are differently
> linker-signed files, so a matching `.sha256` only says infra installed what we
> published. Naming the rev on the *installed* file still takes a symbol probe
> (`strings -a`, as for the sync-daemon below), so keep announcing the
> added/removed strings per rev in the handoff — the release asset does not
> make that line redundant.
>
> **And never re-hash the *installed* file to verify a swap took.** Step 4's
> ad-hoc `codesign` guarantees the live binary can never hash to the asset's
> sha256 — a mismatch there is the expected state, not a failed swap, however
> much it reads like one. Audited by infra on the v0.15.0 install
> (2026-07-25, thread `4bd0ad43`): live `80828947…` vs asset `65b0f6b0…`,
> the live file 71,184 bytes *smaller* — yet stripping both signatures
> (`codesign --remove-signature` on copies) leaves the same 15,243,616 bytes
> with a byte-identical payload (everything past the load commands hashes
> `b8c7c1bd…` on both; the whole stripped files differ in exactly one byte,
> inside the load-command region). The `codesign -dv` tell: live
> `Identifier=cchv-hub-<40 hex>` (ad-hoc, derived from the filename at
> install) vs asset `Identifier=hub-7d6fcdf5865f0c25` (CI). So the sha256
> agreement above is load-bearing only *before* the copy — relay digest,
> `.sha256` sidecar, GitHub API digest, which is where this recipe puts it;
> after the copy, a file hash answers a different question than the one being
> asked. Verify the installed rev by behaviour instead: a symbol probe, a
> route flip (v0.14.0's 404→200), or a response header (v0.13.1) — all
> signature-independent. Third member of the marker-trap family, after the
> v0.11.1 class fragment and the v0.14.0 CSS chunk: a check that looks
> stricter than the real one while answering something else. (Recorded
> infra-side in `hosts/m4m.md` next to the v0.15.0 rev probe.)
>
> Either way, **do not `cp` a new binary over the live one in place.** macOS
> caches the code signature per inode; overwriting in place with a
> differently-linker-signed binary trips the kernel's signature check and the
> process is killed on every spawn with `OS_REASON_CODESIGNING`. A hung
> `launchctl kickstart -k` then wedges the job in `spawn scheduled`.

Working sequence (validated on m4m 2026-07-13, thread 7938448b):

```bash
STAGED=~/.config/cchv/staging/cchv-hub-<sha>          # the new binary
LIVE=/usr/local/bin/cchv-hub                          # whatever the plist ExecStart points at
STAMP=$(date +%Y%m%d-%H%M)

# 1. Back up the currently-live binary (to staging, timestamped).
cp "$LIVE" ~/.config/cchv/staging/cchv-hub-preswap-$STAMP

# 2. Unlink the old binary FIRST — do not cp over it (inode codesign cache).
#    `trash` is the house rule and works the same here: either way the path is
#    unlinked and step 3 gets a fresh inode; step 1's backup is the rollback
#    point regardless.
rm "$LIVE"

# 2b. Assert the removal actually took before copying. This makes the
#     inode-codesign trap unreachable: the one way step 3 quietly becomes an
#     in-place overwrite is a removal that failed while the script carried on.
#     (Infra hardening from the v0.17.0 swap, thread e05b6f2e.)
[ ! -e "$LIVE" ]

# 3. cp the staged binary → a fresh inode.
cp "$STAGED" "$LIVE"

# 4. chmod — `gh release download` stages assets -rw-r--r--, and because step 2
#    removed the old inode, the cp above inherits the SOURCE mode: without this
#    the live binary is not executable and bootstrap fails to exec it.
chmod 755 "$LIVE"

# 5. Re-sign ad-hoc (the kernel rejects the cached signature otherwise).
codesign --force --sign - "$LIVE"

# 6. Eyeball the mode before restarting — must read -rwxr-xr-x.
ls -l "$LIVE"

# 7. bootout + bootstrap — NOT `kickstart -k` (which can wedge in
#    "spawn scheduled"). If a prior kickstart hung, kill it first.
launchctl bootout  gui/$(id -u)/dev.cchv.hub 2>/dev/null || true
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.cchv.hub.plist
```

Step 4 was missing until the `v0.16.0` swap (2026-07-26, thread `1b97e64b`),
where infra caught the live binary sitting `-rw-r--r--` *before* the restart
and fixed it by hand — and found `staging/cchv-hub-0.15.0-…` is `-rw-r--r--`
too, so every release-asset swap had been one forgotten off-recipe `chmod`
away from a failed bootstrap. The local-build fallback path masks the gap
(cargo emits the bit); only the release-asset path trips it.

Verify: `curl -s https://hub.internal:8788/v1/healthz` → `{"status":"ok",…}`
and the process is running clean (fresh pid, no respawn churn).

**2026-07-24, hub `v0.13.0` → `v0.13.1` swap (thread `497cf57c`): a new
response header can be the swap-proof probe.** The release added
`Cache-Control: no-store` to every `/v1/*` response (hub `28be09ca`), and
infra used exactly that as the rev probe — header absent on
`/v1/journal/entries` pre-swap, present post-swap — proving the new binary
took without a `strings` symbol probe. When a rev ships an observable
behavior change on a live endpoint, name it in the handoff as the probe; it
beats symbol-grepping. sha256 `8be0384f…33a017d` agreed across the `.sha256`
sidecar and the GitHub API digest; pid 10517; preswap backup
`staging/cchv-hub-preswap-20260724-1244` (the 0.13.0 binary). `/v1/healthz`
`{"db":"up","status":"ok"}`; `/v1/healthz/journal` 200, 5 groups all
`stale:false`; the static index still serves `no-cache` (the SPA cache-split
block untouched — `no-store` is scoped to `/v1/*`). Recorded infra-side in
`hosts/m4m.md`, home-network `bd04e4e`.

**2026-07-25, hub `v0.13.1` → `v0.14.0` (thread `7bf6a920`) — ALL THREE STEPS
LANDED and verified from both sides (infra confirm `bf77f768`; our re-probe
~16:10 — see the machine correction below before citing it as independent).**
Binary live (preswap backup
`staging/cchv-hub-preswap-20260725-1454` = the 0.13.1 rollback point); catch-up
`backfill-analytics` ran — the five old-binary-gap sessions
(`587988`/`588038`/`588039`/`588076`/`588159`) report their tool rows both in
the db (infra) and through the live `/v1/stats/sessions/{id}` API (us), same
numbers; webapp swapped — served entry chunk `archive-CEkkOQjO.js`, old
`archive-0BmPzvZw.js` 404s, Analytics tab renders. Hub analytics: the
swap-proof probe is a **new route**, which is stronger than a new header —
`GET /v1/stats/global` 404s on `v0.13.1` (verified pre-swap, 13 ms) and answers
200 on `v0.14.0`. **Correction, same day:** the pre- and post-swap probes in this
entry were originally recorded as run "from ac-mbm5". They were not — they ran
**on m4m itself**, the hub's own host. The timings and status codes stand (if
anything they understate what a remote client sees, since they skip the tailnet
hop), but they were never the independent cross-machine check the wording
claimed. The lesson is cheap and general: **an agent should not assert which
machine it is on without checking** — `scutil --get ComputerName` costs nothing,
and on m4m `hostname` can report the stale Bonjour suffix `m4m-2`. Asset
`cchv-hub-0.14.0-aarch64-apple-darwin`, sha256 `accf9daa…07b06b6`. Two things
that made this deploy unlike the others:

- **The schema landed a deploy early.** Migration `0005` and its backfill were
  applied to pg1 on 2026-07-25 (09:51Z / 10:04Z) while `v0.13.1` was still
  live — the column is nullable and the old binary never touched the new
  tables. So this is a binary swap with *no* migration step, and it needs a
  catch-up `hub backfill-analytics` afterwards for whatever the old binary
  ingested in between (idempotent, resumable).
- **A correct response can look like a hang.** `/v1/stats/global` unwindowed
  takes ~13–14 s over 2.5M messages (cchv Gitea #24). Any probe needs a
  generous `--max-time`, or a one-day window (`?from=…&to=…`), which is fast.
  Naming this in the relay is the difference between a verification and a
  false rollback.
- **The CSS entry chunk was NOT a valid swap marker for this release.**
  `assets/archive-BBzvspm0.css` is byte-identical between `v0.13.1` and
  `v0.14.0`, so asserting on it passes on the *old* bundle — the v0.11.1
  marker trap again, in a new costume. Before quoting entry-chunk hashes in a
  relay, diff them against what is **live** and quote only the ones that
  actually changed (here: `assets/archive-CEkkOQjO.js`, or the string
  `stats/global`, which exists only in the new bundle). Infra has recorded the
  standing rule on their side (`hosts/m4m.md`): **send one discriminating
  marker, not two convenient ones** — a second marker that also passes on the
  old bundle subtracts confidence rather than adding it.

Two lessons from how the deploy *ran*, for the next multi-step handoff:

- **A multi-step deploy does not fit in the relay handler's 900 s ceiling.**
  This ask was delivered three times: deliveries 1 and 2 were killed at the
  relay-supervisor timeout (`rc=124`) *after* landing side effects, so
  delivery 3 found the binary and webapp already swapped and the backfill
  running orphaned at `PPID 1`. Nothing was lost — only because infra's
  2026-07-19 rule (probe live state before re-running a mutating ask) stopped
  a blind re-swap, which would have captured the *already-swapped* binary as
  the "pre-swap" backup and destroyed the rollback point. Until the
  supervisor-side fix lands (home-network Gitea issue #34 — rc=124 timeout
  NAKs like a transient failure, so redelivery can replay a completed
  mutation; infra will notify on this relay when it lands), **split a
  multi-step deploy into separately-completable relays** — e.g. binary swap + rev probe
  in one message, catch-up backfill + webapp swap in a second — so each
  handler run finishes inside the ceiling.
- **`/v1/stats/sessions/{id}` takes the numeric `sessions.id` row id**
  (`Path<i64>`), so db-side row ids quoted in a relay are directly probeable
  through the live API — that is how the backfill was cross-verified here
  without db access. A nil UUID → 400 is the path-parse rejection, not a
  not-found regression; unknown *numeric* id → 404 as specced. And on this
  house deployment an unauthenticated in-tailnet request reads 200 via the
  cchv-v0.5.0 Tailscale-identity read-auth — do not read that as an auth
  regression when probing. The mirror image of that cost infra time on this
  same deploy: over the **loopback** bind there is no tailnet identity, so
  every read route 401s while `/v1/healthz` still answers 200 — see the probe
  matrix in the "Hub topology on m4m" note (§3).

**2026-07-25, hub `v0.14.0` → `v0.15.0` swap (thread `8edffabd`): the #25
credential watchdog is live — the 2026-08-24 rotation now heals unattended.**
Swapped and verified 17:56 local, pid 60157, downtime ~1 min. The single-step
sizing from the v0.14.0 lesson worked: swap + verify in one message fit well
inside the 900 s ceiling. Asset `cchv-hub-0.15.0-aarch64-apple-darwin`, sha256
`65b0f6b0…5438e7f` agreed **four ways** before anything was copied — our relay
digest, the `.sha256` sidecar, infra's local re-hash, and the GitHub API digest
(uploader `github-actions[bot]`, CI path). Rev probe: watchdog `strings` marker
count 0 on the live 0.14.0 binary pre-swap, 1 on the staged asset, 1 on the
live path post-swap. Ceremony was §2b exactly; preswap backup
`staging/cchv-hub-preswap-20260725-1756` (= the 0.14.0 rollback point).
`/v1/healthz` `{"db":"up","status":"ok"}` (re-probed from **ac-mbm5** — and
after the v0.14.0 machine-claim correction put that wording in doubt, the
provenance was *verified* same day, thread `4bd0ad43`: the archived handler
session `7f4ae851…` is stamped `machine_hostname: ac-mbm5.local` by the hub,
its transcript sits on ac-mbm5's own disk, and the probe was a bare `curl` to
the tailnet HTTPS front, no ssh wrapping — a genuine cross-machine check.
Precision: the curl ran 18:00:57 local; the "~18:04" in the relay reply was
when the reply was written). The relaunch log carried `rendered hub.runtime.toml (bao-first)` and
`db password from bao static-creds/cchv-svc (bao-owned, rotating)` — the
credential resolved through the rotating static-creds path in prod, not a
cached render, so the watchdog is guarding the credential it will actually see
rotate. Ingest resumed within seconds. Watchdog log semantics (§3b: 3× WARN
strikes → ERROR → relaunch = a successful rotation pickup; the same pattern
every 5 min = incident) are recorded infra-side in `hosts/m4m.md`
(home-network `15ca43a`) for the next log reader. Two operational notes from
the run, both worth knowing before the next swap:

- **A held `staging/.swap-lock` is not proof of an in-flight swap — check the
  stamp.** The lock from the 14:54 v0.14.0 swap was still held: its handler
  had been killed at the 900 s ceiling *after* completing the swap but
  *before* releasing the lock (the same rc=124 mechanism as the v0.14.0
  lessons above, biting a different resource). Infra stamp-checked it stale
  (`STAMP=20260725-1454` = an already-completed, verified swap), replaced it,
  and released it after verification. On hitting a held lock, compare its
  stamp against the last known swap before treating it as a live conflict —
  and never blind-break a lock whose stamp you can't account for.
- **The first `launchctl bootstrap` immediately after `bootout` can fail
  transiently** with `Bootstrap failed: 5: Input/output error` — the bootout
  is still settling. The identical command succeeded seconds later. Retry
  after a short pause before escalating; the first I/O error is not a wedged
  job (that failure mode is the hung `kickstart -k` described above).

**LANDED with `v0.16.0` — the release carrying `345f3459` (honest browse
lists): probe set, pre-measured baseline, and a header-grep trap.** `345f3459`
shipped in `cchv-v0.16.0` (swapped 2026-07-26, record below). The swap proof
actually used was the release's stronger new-route flip (`/v1/healthz/stats`,
404 → non-404), so the probe set below was **not individually re-run** — the
baseline readings stand as historical, and the ⚠ decoy warning is permanent
regardless. Infra measured the pre-swap baseline on m4m against the live
`v0.15.0` hub at 2026-07-25 19:03Z (thread `82f10631`, recorded infra-side in
`hosts/m4m.md`, home-network `739f098`), so each probe below had a verified
old-binary reading to flip against:

- `GET /v1/sessions?projectid=1` → 200 today, **must become 400** naming the
  unknown field (`deny_unknown_fields`). Clean and discriminating as written —
  this is the primary rev probe.
- `GET /v1/sessions?project_id=999999999&limit=3` → today returns the SAME
  three rows as unfiltered `limit=3` (ids 272 / 590461 / 590033 at baseline);
  **must become an empty list**.
- `GET /v1/sessions?limit=1` and `GET /v1/projects?limit=1` → today carry no
  `X-Total-Count` response header; **must carry one**. ⚠ **Always anchor this
  grep — the decoy is permanent, not a pre-swap hazard.** The hub's CorsLayer
  (`crates/hub/src/lib.rs` `expose_headers`, router-wide since `3e718bd6`)
  puts `access-control-expose-headers: x-total-count` on **every** response —
  healthz included — and `345f3459` does not touch `lib.rs`, so the NEW
  binary emits it too. An unanchored `curl -D - … | grep -i x-total-count`
  therefore passes on the old binary (reporting a swap that never happened)
  **and** passes post-swap for the wrong reason — as would every future
  header probe against this hub. Fourth member of the marker-trap family
  (v0.11.1 class fragment, v0.14.0 CSS chunk, v0.15.0 installed-file
  re-hash), and the first that never retires. Anchor on the header line
  itself: `grep -iE '^x-total-count:'`, or read `curl -sw '%{header_json}'`
  and check the key.
- `GET /v1/sessions/<uuid>/messages?limitt=1` → 200 today, **must become
  400**. (The messages endpoint already sent `X-Total-Count` pre-fix, so the
  header is not a probe there.)

**2026-07-26, hub `v0.15.0` → `v0.16.0` swap (thread `1b97e64b`, infra reply
`b48d04b3`): the DuckDB stats mirror is live, and the mirror finished its cold
build inside the handler window.** Swapped ~00:55 local, fresh pid 25286 (was
60157); preswap backup `staging/cchv-hub-preswap-20260726-0055` (= the 0.15.0
rollback point, 15,291,680 B). Digest `d636ab84…7331c4da` (57,631,264 B)
agreed **four ways** before anything was copied — relay, `.sha256` sidecar,
infra's local re-hash, GitHub API digest — and the installed file was *not*
re-hashed, per the v0.15.0 lesson. `otool -L` shows no `libduckdb` dylib
reference (statically linked, as shipped). Swap proof: `/v1/healthz/stats`
404 pre-swap → `503 warming` immediately post-swap → `200
{"status":"ok","ready":true,…}` once the mirror landed. Binary only — webapp,
`static_dir`, `hub.toml` all untouched, no migration. Operational readings
worth keeping:

- **Mirror cold build: 480 s** for 2,920,083 messages / 142,160 tool_uses /
  141,988 tool_results (the 497 s / 2,898,915-row estimate was within 4%);
  final file **789 MB** (confirming the corrected ~792 MB figure — the 119 MB
  draft number was wrong). Ingest ran clean throughout; `mirror rebuild`
  serves from the old mirror while building and needs no restart.
- **Perf reproduces on m4m**: `/v1/stats/global` 30-day `Europe/Rome`
  0.59 s / 0.55 s (dev box: 0.48 s); unwindowed 0.65 s (dev box: 0.57 s).
- **Probe hygiene**: `/v1/stats/*` `from`/`to` want `YYYY-MM-DD` — RFC3339
  gets a clean `400 invalid from date (want YYYY-MM-DD)`. That 400 is caller
  error, not a regression.
- **The exec-bit gap** (recipe step 4 above) was caught live on this swap:
  the live path (`~/.local/bin/cchv-hub` on m4m) came out `-rw-r--r--` after
  the `cp` and would have failed bootstrap; infra `chmod 755`'d before the
  restart. Recorded infra-side in `hosts/m4m.md`, home-network `f7cbb36`.
- ⚠ **m4m disk was at 98.9% used (22.5 GB free) at swap time** — the 121 GiB
  figure quoted in the handoff was a day stale, ~95 GiB consumed in between.
  Not cchv (all of `~/.config/cchv` is under 1.6 GB); leading theory is APFS
  local Time Machine snapshots, filed as home-network **#35** (needs sudo).
  Until resolved this bounds headroom for mirror rebuilds, which write a
  fresh ~800 MB file alongside the old one before swapping. The same volume
  then read **54.1 GB free at 02:07** with nothing deleted (infra report
  `067118a5`; two APFS local TM snapshots present at 02:10 — one being
  thinned is the right order of magnitude, but nobody sampled the snapshot
  list at 00:55, so consistent-with, not proven). Standing rule from that
  report: **any recorded free-space number on m4m is a timestamped sample,
  not a state** — re-measure at the moment of a rebuild instead of trusting
  a recorded figure.
- The `backfill-analytics` + `mirror rebuild` standing rule (old dedup
  grouping → silent token over-count) is recorded infra-side in
  `hosts/m4m.md`; nothing was backfilled on this swap. The Gatus check for
  `/v1/healthz/stats` goes out as a separate relay.

**2026-07-26, hub `v0.16.0` → `v0.17.0` swap (thread `e05b6f2e`, infra report
`8c4be5ef`, home-network `2634104`): step 2b/3 hardening lands, and the mirror's
no-503-window property gets its first live confirmation.** Swapped ~17:57 local,
pid 25286 → 77531, no respawn churn, downtime well under a minute; preswap
backup `staging/cchv-hub-preswap-20260726-1757` (= the 0.16.0 rollback point,
57,314,320 B). Digest `f75eeeaa…97baccf` (57,613,056 B) agreed **four ways**
before anything was copied — relay, `.sha256` sidecar, GitHub API digest,
infra's local re-hash — and the installed file was not re-hashed, per the
v0.15.0 lesson. Swap proof: `model_distribution` at the **project** stats scope
(`/v1/stats/projects/{identity_key}` — `ProjectStatsSummary` grew from 15 keys
to 16), measured pre-swap False/15 independently on both sides, post-swap
True/16 with a populated 4-entry array. Both of infra's readings ran on m4m
(stated, per the machine-honesty rule); the independent cross-machine check ran
from **ac-mbm5** (`scutil`-verified) after the report: bare `curl` over the
tailnet HTTPS front, no ssh wrapping — 200, 16 keys, `model_distribution`
populated, 5.85 s cold (first materialization of that project scope; warm
readings are ~0.3 s). Operational notes:

- **The step 4 `chmod 755` earned its keep on its first outing**: the 0.17.0
  release asset staged `-rw-r--r--` exactly as v0.16.0's did, step 4 fired, and
  `ls -l` read `-rwxr-xr-x` before the restart. The gap is closed in practice,
  not just on paper.
- **Recipe hardened (step 2b above, infra suggestion from this swap)**:
  `[ ! -e "$LIVE" ]` between the unlink and the cp makes the in-place-overwrite
  codesign trap unreachable rather than merely avoided.
- **`bootstrap` succeeded on the first attempt** — the transient
  `Input/output error` from the v0.15.0 swap did not recur. It is intermittent;
  the retry-after-a-pause guidance stands.
- **First live confirmation that mirror readiness persists across a restart**
  (the v0.16.0 design property): `/v1/healthz/stats` read `ready:true` within
  ~14 s of restart (`age_seconds` 14, `lag_rows` 454), never `warming`. Routine
  §2b deploys do not cold-build the mirror. The mirror file was untouched and
  reopened as-is; no backfill ran (none owed — no schema/grouping change).
- **`.swap-lock` was clean on arrival** (unlike v0.15.0), held for the swap,
  released after verification.
- **Resources, measured at swap time**: 98 Gi free / 95% used before, 97 Gi
  after; `staging/` 609 M → 718 M. Correction to our relay: the hub binary is
  **55 MB, not ~15 MB** (DuckDB statically linked since v0.16.0) — the ~15 MB
  figure was stale from the pre-DuckDB line and matters if it ever feeds a
  sizing estimate.
- Binary only — this entry is 1 of 2. The webapp swap (§2c) follows as its own
  relay, closing the chip-vs-API divergence (chip v0.16.0 against a v0.17.0
  API) that the every-release-ships-the-webapp rule exists to prevent.

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

> **The handoff shape a webapp deploy wants from us: tag + built-from rev +
> expected entry chunk + a counted marker.** Since home-network `3c84aa4`
> (2026-07-19) the recipe takes all four, so the checks we used to ask for by
> hand on v0.10.7 are now part of it rather than a one-off:
>
> - `--expect-entry archive-<hash>.js` — the entry chunk we built and expect to
>   be served, checked **before** the swap.
> - `--assert-count '<N>:<literal>'` (repeatable) — counts a marker of the
>   announced change in the release bundle pre-swap (fatal, before the live tree
>   is touched) and again in the **served** entry chunk post-swap (rolls back).
>   **Count, do not grep for presence:** the failure mode of a "narrow every call
>   site" change is a *surviving un-narrowed* call site, which is invisible to a
>   presence check and only shows up as a wrong count. Pick the literal from the
>   diff and count it in our own `dist-archive/` first — the number we send is
>   the assertion. **The bundle count and the rendered-DOM count legitimately
>   differ:** the version-chip literal is ×2 in the chunk but ×1 in the DOM —
>   the two `title` sites in `ConnectGate.tsx` are the connect gate and the
>   connected header, and only one mounts at a time. 2-vs-1 is a healthy
>   deploy, not a half-landed one (infra flagged the misread risk on the
>   v0.16.0 swap, thread `1d2b8f90`).
> - CSS byte-identity against the tree being replaced — **reported, never
>   fatal, and it narrows in ONE direction only.** Changed bytes ⇒ the eyeball
>   item re-opens and a new look is owed. **Identical bytes settle nothing
>   about rendering**: they prove no colour/spacing *token* moved, but the
>   utility classes a view *applies* live in the JS chunk, so a JS-only deploy
>   can restyle a view with classes the stylesheet already carried. Measured
>   counterexample, `cchv-v0.17.1`: same stylesheet by content hash AND asset
>   name (`archive-DetcOCbl.css`), yet the chart-card re-size (`3093718f`)
>   landed via an `items-start` utility already in the bundle from use
>   elsewhere. The prior form of this bullet — identical bytes ⇒ the pending
>   eyeball item "carries forward unchanged" — is struck; infra struck its
>   mirror-image standing rule in `hosts/m4m.md` the same day (home-network
>   `4880890`, 2026-07-26). It hashes *content*, so an asset-hash bump over
>   identical bytes still reads as unchanged — and either way it cannot answer
>   "has anyone looked yet", which is not headless.
> - **When a release splits into §2b + §2c halves, the webapp relay must
>   additionally NAME the joined integration check** — the rendered view that
>   proves the two halves actually join (for v0.17.0: a *project-scoped*
>   Analytics view rendering real per-project `model_distribution` data, not the
>   "Not reported by this hub version" fallback). A split release is the one
>   shape where every per-message assertion can pass while the feature is still
>   dead: the §2b half proves the API grew, this half proves the bundle is
>   served, and neither proves they join. The handler of the SECOND half owes
>   that check, so ask for it explicitly instead of leaving it to be inferred
>   (infra suggestion from the v0.17.0 swap, thread `e05b6f2e` — our
>   fallback-state note made the check constructible there, but only just).
> - **Write the assertions end-state-shaped, because a completed relay can be
>   handed out again.** On the NATS channel a redelivery is indistinguishable
>   from a first delivery (no status field, no archive to consult): the v0.17.0
>   webapp ask redelivered ~12 min after the swap had landed, and m4m answered
>   "is the end state already true?" instead of re-running — possible only
>   because `--expect-entry`/`--assert-count` describe the *end state*, not the
>   procedure. The 900 s relay split bounds slow handlers; it does nothing about
>   redelivery. Recipe idempotency is only the second line of defence — a replay
>   here would have been safe but not clean (see the dated entry below).
>   Recipient-side half distilled to `CONTEXT/PATTERNS/agent-relay.md`
>   (`7cc8bae`).
>
> Verified end-to-end on m4m, not syntax-checked: an idempotent v0.10.7 redeploy
> passes all four; a bogus `--expect-entry` and a wrong marker count each abort
> pre-swap with the live tree untouched; a malformed spec is rejected at parse.

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
  stabler than a hashed asset name). If the check goes one level further — a
  headless browser against the *rendered* page — **anchor on
  `data-testid="app-version"`, never a loose version grep**: the rendered DOM
  also carries version literals from **archived session content** (on the
  v0.16.0 render: `cchv-v0.13.0` ×2 and `cchv-v0.6.0` ×3, all from old
  transcripts), so an unanchored `grep cchv-v` happily reports somebody's
  transcript as the chip. Another member of §2b's marker-trap family, and the
  anchor is on both chip sites in `ConnectGate.tsx`, so it holds whichever
  view is mounted.
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
- `/v1/healthz/journal` present (200 or a *legitimate* 503 — a real undrained
  in-window day; check the body's `groups`). A 404 here means the deployed
  binary predates `distiller-self-healing` (cchv-v0.13.0) — the swap didn't take.
- the asset-list diff vs the backup touches only the chunks the change should
  touch — a client-only patch that moves other chunks is a red flag

Visual/layout changes cannot be verified this way; a rendering claim needs a
human at a real window. Say so explicitly instead of marking it green. The
recipe's CSS byte-identity report narrows *what is owed*, never discharges it —
and since `cchv-v0.17.1` (see the handoff-shape bullet above) only the changed
direction narrows at all: changed CSS means a fresh look is owed; unchanged CSS
means only that no token moved, and says nothing about whether the JS chunk
restyled a view with classes the stylesheet already carried.

**Discharged 2026-07-19 (relay `8451d81a`): the three items that close named.**
The user looked at the live hub themselves, in a real window, through
`cchv-v0.11.1` and confirmed all three at once: the v0.11.0 Locations block, the
v0.11.1 toolbar hierarchy, and the long-pending `cchv-v0.10.6` rose topic chips.
The backlog reopens on the next CSS-changing deploy.

**Discharged 2026-07-25 (relay `74263172`): the two older claims, closed by
name** — the `cchv-v0.10.4` ultrawide rail/feed centering (`4a0fc78`) and the
`cchv-v0.10.5` selected-day + date-pill contrast in **both** themes
(`7af316f`). The infra side had asked whether the earlier global "the backlog
is empty" sentence swallowed them (relay `513e4dd8`); the answer was no — each
carried a *condition* that pass gave no evidence of meeting (an ultrawide
viewport; a light/dark toggle), and a global sentence that closes an unnamed
item is exactly the inference the close-in-pieces rule forbids. An attended
look on 2026-07-25 met both conditions and the close relay named both items.

**…and then `74263172` itself overreached the same way** ("backlog empty,
nothing implied"), corrected minutes later by relay `6629892c`: **still open,
by name, is the `cchv-v0.12.0` degraded-hint CSS** (thread `3dc909f8`,
2026-07-21 — that release's CSS was *not* byte-identical; it added utilities
for the semantic-search "degraded" hint in the Browse search section, tracked
in the semantic-journal-search lane's memory, not in this section — which is
how two closes in a row missed it). The 07-25 look covered layout and
contrast and says nothing about a conditional hint that only renders when the
hub reports `journal_degraded: true`. To retire it: a real-window look at
Browse search with the hint actually rendered (hub degraded or hint forced),
relayed by name — or a code-level retire if the hint markup adds nothing
visible outside the degraded state. **The backlog is exactly that one item.**
Lesson for the next close: sweep *every* lane's open items (memory included)
before writing any sentence of the form "the backlog is empty".

Infra acked the 07-25 close the same day (`a57b9ba5`; home-network `4dbdf8c`
carries it as its own `hosts/m4m.md` entry). Two things from that ack are
durable, and one is superseded:

- **The `~/.local/share/cchv/eyeball/v0.11.0/` captures on m4m stay put.** They
  are no longer the reference set for the v0.10.x/v0.11.x items, but they
  remain the anchor for the pair-equality hashes recorded below, and infra
  prunes nothing there without our word.
- **A close is announced, never inferred** — infra reads our silence as
  still-owed, per sub-item. That is what made the `6629892c` correction cheap:
  their record moved the moment we said so.
- **Superseded: infra's "§2c is dormant, not retired" framing.** It was written
  against the empty-backlog sentence that `6629892c` retracted. §2c is *live* —
  the `cchv-v0.12.0` degraded-hint item above is open, so deploy reports on
  this line still carry it rather than reading plain "deployed and verified".

**Infra confirmed the `6629892c` correction (relay `fadd6d2d`, 2026-07-25;
home-network `5465e32`, `hosts/m4m.md`).** Their "backlog EMPTY / nothing
carried" sentences from `74263172` are superseded in place: their record now
carries exactly the one degraded-hint item, notes their v0.12.0 deploy entry
had in fact always held it OPEN with us owning the look (the close block had
silently dropped it), and records §2c as active for it — deploy reports on
their side carry it forward again. They also recorded that the 07-25 attended
look covered layout/contrast only and carries no evidence about a hint that
renders only on `journal_degraded: true`. Both sides' records now agree; the
close path is unchanged and ours (real-window look with the hint rendered, or
a code-level retire), and infra holds the item open until we relay that close
by name.

**2026-07-24, webapp `v0.12.0` → `v0.13.0` swap (thread `c8109762`): CSS
byte-identical.** Both trees ship the *same* chunk `archive-BBzvspm0.css` —
same name **and** same bytes (sha256 `5ac5d2de786c…` on the staged v0.13.0 tree
and on the v0.12.0 preswap backup; re-verified by infra, home-network
`e1bd473`). Only the JS entry chunk flipped (`archive-BAfYulLt.js` →
`archive-Dc0q_4px.js`, for the embedded version string), so the
v0.10.4/v0.10.5 backlog above carries forward untouched — no new eyeball item.

> **Gotcha: a "CSS DIFFERS" report line with blank hashes is an extraction
> failure, not a diff.** Our swap report for this deploy printed
> `CSS DIFFERS (->)` with *empty* hash values — the compare step failed to
> extract the hashes and the report defaulted to "differs" instead of
> "unknown". Infra caught it by hashing both trees (byte-identical, above).
> Treat any differs-verdict whose evidence fields are blank as *no verdict*:
> re-hash the two chunks by hand before recording anything, and never let it
> re-open (or silently carry) the eyeball backlog.

**2026-07-24, webapp `v0.13.0` → `v0.13.1` swap (thread `497cf57c`): verified
by infra** — the live `index.html` references the new entry chunk
`archive-0BmPzvZw.js`, backup `webapp-preswap-20260724-123924-cchv-v0.13.0`
in place. The change was JS-only (`hubApi.ts` fetch `cache:'no-store'` +
`JournalView` refocus refetch, `aa22bda7`), so no new eyeball item; the
v0.10.4/v0.10.5 backlog above carries forward untouched.

**2026-07-25, webapp `v0.14.0` → `v0.15.0` swap (thread `a43b9428`): 2 of 2 of
the v0.15.0 deploy (hub binary swap in §2b), landed 18:02 local.**
`just cchv-webapp-deploy 0.15.0 --expect-entry archive-CU7Vo6RB.js
--assert-count "2:cchv-v0.15.0"` — entry chunk matched pre-swap, and the
count-based marker held in both the release bundle and the SERVED chunk
post-swap. The count `2` was **grounded, not guessed**: infra derived it from
the live v0.14.0 bundle (2× `cchv-v0.14.0` at the two ConnectGate title
sites); since `src/` is unchanged between the releases the count must carry
over — a worthwhile refinement of the v0.11.1 count-marker rule (derive the
expected count from what is *live*, not from reading the source). CSS is
byte-identical to v0.14.0 (`archive-BBzvspm0.css`), report-only — since the
v0.14.0 swap the deploy tool rejects a CSS `--expect-entry` outright — so the
degraded-hint eyeball item above carries forward unchanged. Health:
`/v1/healthz`, `/v1/healthz/ingest?exclude=ac-mbp`, and the `:8788` tailnet
front all 200. The version chip is client-rendered from exactly the literal
asserted in the served chunk, so it reads v0.15.0 for any browser — *served,
never seen* (headless run), per the standing rule. Rollback point:
`staging/webapp-preswap-20260725-180203-cchv-v0.14.0`. No home-network commit
(routine bundle swap; `RELAY_AUDIT` is the trail per the 2026-07-19 standing
decision).

**2026-07-26, webapp `v0.15.0` → `v0.16.0` swap (thread `1d2b8f90`, infra
report `067118a5`): the chip-lag correction — and the first chip verified by
render, not just by serve.** The v0.16.0 hub swap (§2b above) deliberately
told infra to skip the webapp ("frontend byte-identical to v0.15.0"), which
was true of `src/` and wrong about the bundle: `package.json`'s version
compiles into the entry chunk and is exactly what the header chip renders, so
the live chip announced v0.15.0 against a v0.16.0 hub — user-spotted within
the hour. Rule from that miss (tasks 7.5b): **a release bumps the version
string, so the webapp ships with every release** — "no frontend change" is a
statement about behaviour, not a reason to skip the deploy.
`just cchv-webapp-deploy 0.16.0 --expect-entry archive-DCMJMiIB.js
--assert-count '2:cchv-v0.16.0'` landed 02:07:55, run on m4m itself. Nothing
was staged, so it staged straight from the release artifact — the announced
§2c no-op-diff path. Served entry `archive-DCMJMiIB.js` 200 (matched the
prediction **pre-swap**, so the live tree was never touched to find out); old
`archive-CU7Vo6RB.js` **404** (swap, not cache); marker `cchv-v0.16.0` ×2 in
the release bundle and in the served chunk; healthz trio 200. CSS
`archive-BBzvspm0.css` byte-identical on the live and preswap trees (sha256
`5ac5d2de786c…`, hashed on both), so the `cchv-v0.12.0` degraded-hint item
above stays the only open eyeball item. Rollback point:
`staging/webapp-preswap-20260726-020755-cchv-v0.15.0`. Infra then closed the
gap the standing "served, never seen" caveat names — since the chip *being
seen* was the whole point of this deploy, proving the bundle would have
re-created the miss in miniature: headless Chrome `--dump-dom` against the
live `:8788` front renders `<span data-testid="app-version" …
title="cchv-v0.16.0">(v0.16.0)</span>`. Their two findings from that render
(the `data-testid` anchor vs the transcript decoy; ×2-in-chunk vs ×1-in-DOM
is correct) are folded into the rules above, both verified against
`ConnectGate.tsx` here before recording. Infra-side record: home-network
`d7c4b43`, which also corrects their own v0.16.0 binary entry — "webapp
byte-identical to v0.15.0's frontend" was true of the source, false of the
built bundle, and that distinction is the argument for this deploy.

**2026-07-26, webapp `v0.16.0` → `v0.17.0` swap (thread `e05b6f2e`, infra reply
`d333bedd`, home-network `d78bfe9`): 2 of 2 of the v0.17.0 deploy (hub binary
in §2b above) — and the deploy that named the missing relay field, the joined
check.** Landed 18:04:02 local, run on m4m itself (no ssh). Ordering honoured —
binary 17:57, webapp 18:04 — so the new Analytics UI never rendered against a
hub that could not serve it, the exact chip-vs-API divergence class this
sequencing exists to prevent. Nothing was staged beforehand, so the recipe
staged straight from the release for the tag and the staged-vs-released diff
was a deliberate no-op (the announced §2c path). Assertions all green: entry
chunk `archive-DCMJMiIB.js` → `archive-CYB_PMQN.js`, the predicted chunk
matching the release **pre-swap** (live tree never touched to find out); marker
`cchv-v0.17.0` ×2 in the release bundle and in the served chunk; ×2-in-chunk vs
×1-in-DOM held again (rendered DOM: `cchv-v0.17.0` ×1, `cchv-v0.16.0` ×0), chip
reads (v0.17.0) and agrees with the API; healthz trio 200 (`/v1/healthz`,
`/v1/healthz/ingest?exclude=ac-mbp`, the `:8788` tailnet front). Rollback
point: `staging/webapp-preswap-20260726-180402-cchv-v0.16.0`. Notes that
outlive the green:

- **The joined check — what neither half's assertions prove.** The §2b relay
  proved the API grew `model_distribution`; this one proved the bundle is
  served; only a rendered **project-scoped** Analytics view proves they join.
  Infra ran it headlessly: selecting home-network issues
  `GET /v1/stats/projects/g%3Abf967c7d…?from=2026-06-26&tz=Europe/Rome` → 200
  and the model-distribution card renders genuinely per-project data
  (claude-opus-4-8 1370.3M leading, vs the global view's claude-fable-5
  3956.1M) — not the "Not reported by this hub version" fallback. Zero console
  errors on either view. The standing rule this produced is in the
  handoff-shape block above: a split release's second-half relay names the
  joined check. This time infra constructed it from our fallback-state note —
  enough, but inferred rather than asked for.
- **CSS moved this release**: `archive-BBzvspm0.css` → `archive-DetcOCbl.css`,
  sha256 `9453cec9…bc04bc05` — our announced prefix, verified by infra both on
  disk and over the wire. Free discriminator that comes with it: the OLD
  stylesheet 404s post-swap too, a second swap marker that **only exists when
  the CSS actually moves** (on v0.16.0 the filename survived and only the JS
  404 was available). Per the one-discriminating-marker rule, count it only on
  releases where the stylesheet filename flips.
- **The eyeball the CSS change re-opens is ours, and was already
  part-served.** The analytics restyle got its attended real-window look the
  same day, pre-deploy (analytics-ux-costs task 7.1, `ceef5244`: light+dark,
  cost card + heatmap against the live hub, project n/a state live,
  injected-cost layout). The residue was always the deploy-gated leg this very
  swap unblocked: **project-scope REAL figures in a real window on the live
  hub** — which infra's headless render above narrows but, per the standing
  rule, cannot discharge. Infra records nothing owed on their side. The eyeball
  backlog is therefore two named items: the `cchv-v0.12.0` degraded-hint
  (unchanged, above) and this project-scope-real-figures residue (tracked in
  `openspec/changes/analytics-ux-costs/tasks.md` 7.1).
- Machine caveat, stated per the honesty rule: all probes ran ON m4m, the hub
  host, skipping the tailnet hop — not an independent cross-machine check.

Note what did *not* close it: the scripted screenshot set below is what finally
retired the chips item only in the sense that it stopped being needed — the
close came from a person's eyes, per the rule in the next paragraph. And
`v0.11.1` shipped **byte-identical CSS** (`11ef16ad84bc`, same as `v0.11.0` —
every utility it needed was already emitted by the earlier build), so it would
have read "clean" to any headless check while the item was still open. The
split earned its keep here rather than being ceremony.

Closing that item is **ours to announce, not theirs to infer.** The infra side
holds a standing UNVERIFIED caveat on every v0.10.6+ deploy and words its own
reports "deployed and asserted", never "verified" (home-network `809533b`); a
byte-identical CSS run will never retire it, by design. So when a human has
actually looked, send one relay line saying so — otherwise the caveat stands
indefinitely while both sides believe the other closed it. That is now a rule
they read, not a courtesy we owe: `hosts/m4m.md` (home-network `9bf48c3`) gives
their close exactly one trigger, an inbound line from an attended session here,
and reads our silence as *still-owed*. So no deploy log, green check, or
byte-identical CSS run on their side will ever stand in for the line.

> **Screenshots narrow the eyeball item; they never discharge it — and a written
> PNG is not evidence that a state changed.** On the v0.10.6 chip look (thread
> `0f5b4d7c`) a scripted expand-click fell through to a broad `article` fallback
> locator, no-op'd without raising, and still wrote a file: the collapsed and
> "expanded" shots were byte-identical in both schemes (caught on the other side
> by hashing them, not by eye). If a shot is meant to be a distinct leg of
> evidence, **assert it differs from the previous one** before counting it.
> In this instance nothing was lost — topic chips render *outside* the
> `expanded` branch in `JournalEntryCard.tsx`, so the collapsed shot already
> covers the rose target and the expanded view adds only open-questions and
> session links, which carry no `--tag*` token. Read the component before
> re-shooting; the code can retire a missing screenshot that a rerun cannot.
>
> Two hygiene rules for those artifacts, both from the same thread: don't park
> them in `/tmp` (reboot-volatile on macOS, and files untouched ~3 days get
> pruned) — **copy them somewhere durable, don't assume they're gone**; and
> **never embed a hub token in the capture script** — read it from
> `~/.config/cchv/` the way `journal_verify.py` does, so nothing mode-644 in a
> shared path ever holds a live credential. The v0.11.0 set now lives at
> `~/.local/share/cchv/eyeball/v0.11.0/` on m4m (`~/Documents` is TCC-blocked to
> an ssh/agent process), with a `README.md` recording the hashes and the fact
> that its `-2-*-expanded.png` files are the same no-op misnomer.
>
> **Say the glob.** Both sides of that thread got the substance right and the
> *scope* wrong, silently: "shot 2 is the expanded state" and "nothing is left in
> `/tmp`" are each a claim whose depth is invisible in the report. `ls /tmp/*.png`
> and `find /tmp -name '*.png'` are different sentences — the artifacts were one
> directory down the whole time. Report the command, the way we now report the
> hash, so the reader can see how deep the check actually went. It is not only an
> absence-reporting rule: twenty minutes later the other side ran
> `ls -la /tmp/shot.py` → "No such file or directory" — correct output, wrong
> question, with the real path (`/private/tmp/cchv-eyeball-v0110/shot.py`) already
> in hand. **A shallow path check is a claim about a path, not about a file.**
>
> TCC generalizes too: `~/Documents`, `~/Desktop` and `~/Downloads` are all
> blocked to an ssh/agent process on these Macs (sshd has no Full Disk Access).
> `~/.local/share` is where agent artifacts go.

> **Hub topology on m4m** (documented on the infra side in `hosts/m4m.md`): the
> hub binds `127.0.0.1:8790` — **not** 8787, which is taken by workerd — with
> tailnet ingress via `tailscale serve` on `:8788` (https) and `:8787` (http).
> A failing loopback `:8787` probe is therefore *not* an outage.
>
> **Never verify a read route over the loopback bind — probe the tailnet front,
> even from m4m itself.** Read-auth is `trust_tailscale_identity`, so a request
> arriving on `127.0.0.1:8790` has no tailnet identity and every read route
> answers `{"error":"unauthorized"}` (401) — while `/v1/healthz` and the static
> assets answer 200 unauthenticated. That half-working loopback probe looks
> exactly like a broken analytics deploy and isn't one (cost infra time on the
> cchv-v0.14.0 verify, relay `40d5df93`). Measured on m4m 2026-07-25:
>
> | probe | `/v1/healthz` | `/v1/projects`, `/v1/stats/*` |
> |---|---|---|
> | `http://127.0.0.1:8790` (loopback bind) | 200 | **401 unauthorized** |
> | `http://hub.internal:8787` (MagicDNS name) | 200 | 200 |
> | `http://198.51.100.7:8787` (tailnet IP, no `Host:`) | **404** | 404 |
> | `http://198.51.100.7:8787` + `Host: hub.internal` | 200 | 200 |
> | `https://198.51.100.7:8788` (tailnet IP) | **empty, exit 000** | — |
> | `https://hub.internal:8788` (MagicDNS name) | 200 | 200 |
>
> So: **address the hub by its MagicDNS name.** `tailscale serve` routes on the
> Host header, which is why the raw IP 404s (the Gatus fix of 2026-07-13) and
> why the IP form needs `-H "Host: hub.internal"`. Its `:8788` https
> leg additionally needs the name for the cert, so **http `:8787` + name is the
> form to script against**; the https name form works too. Sample of the same
> run: `GET /v1/stats/global` → 200 in 16.8 s, `total_tokens` 20,213,333,261.

**2026-07-26, webapp `v0.16.0` → `v0.17.0` swap (thread `e05b6f2e`, message 2 of
2 behind the binary): the chip-vs-API divergence closed in ~6 min, and the CSS
eyeball item RE-OPENS.** Entry chunk `archive-DCMJMiIB.js` →
`archive-CYB_PMQN.js`, deployed straight from the release for the tag — nothing
was staged on either Mac and the relay *said so*, rather than letting the
staged-tree diff become a silent no-op (the v0.10.4 lesson). Assertions sent as
`--expect-entry archive-CYB_PMQN.js --assert-count '2:cchv-v0.17.0'`, counted in
the published tarball, not a local build. From our minute-resolution poller: the
binary field flipped between 17:57:27 and 17:58:28, the entry chunk between
18:03:39 and 18:04:39 — so the chip read `v0.16.0` against a `v0.17.0` API for
~6 min. That is what splitting the relays costs, and it is the right trade
against the 900 s handler ceiling.

Post-swap verification (ours), all against the **deployed** build:

- Chip `(v0.17.0)` read via `data-testid="app-version"` — the anchored form,
  never a loose `cchv-v` grep. Zero console errors on load.
- **The headline fix, which only a deploy could prove:** project scope renders a
  real estimated cost — `$1,502` at 23.9% coverage with a populated per-model
  breakdown (`claude-fable-5 $355 / 757.6M`, `claude-opus-5 $181 / 445.3M`, …)
  where `v0.16.0` could only show the "Not reported by this hub version" dash.
  Asserted the **dash is absent**, not merely that a `$` appeared somewhere.
- A rollup consistency check worth repeating on any future scope: the per-model
  `token_count`s sum to **exactly** `total_tokens` (983,408,016 both ways). A
  dedup bug in the new project-scope statement would surface here as inflation.
- Search containment, all four behaviors live: journal-hits section **322 px**
  (was an unbounded ~3000 px wall), no search input in Analytics, `/` there
  restores *and focuses* it, results survive the Analytics round-trip, and
  activating a hit dismisses the overlay.
- Cross-machine leg from **ac-mbm5** (`scutil`-verified on the remote, bare
  `curl` over the tailnet, no ssh wrapping): healthz `ok`, new chunk served.

> **The CSS changed this release** (`archive-BBzvspm0.css` `5ac5d2de…` →
> `archive-DetcOCbl.css` `9453cec9…`), so the eyeball item **re-opens and is
> still owed.** The real-window screenshots behind the verification above were
> taken by an *agent*, and this section is explicit that screenshots narrow such
> an item and never discharge it. No "a human has looked" line has been sent for
> `v0.17.0`; infra's standing UNVERIFIED caveat correctly stands until one is.
> Recorded here *and* in the memory lane — the last item that lived in only one
> of the two places survived three separate "backlog empty" closes.

> **A release gate that passes locally does not predict CI clippy.** The
> `v0.17.0` gate ran `cargo clippy --workspace --all-targets --all-features -D
> warnings` clean on **clippy 0.1.96**; CI runs stable, now **1.97.0**, whose
> `clippy::manual_filter` fires on `src-tauri/src/commands/stats.rs:2733` and
> fails `Rust Tests`. Not platform-gated code and not a workspace-membership gap
> (`src-tauri` is a root workspace member) — purely a toolchain-version delta, so
> no amount of local care catches it. The failure predates this release (same
> failure on `v0.16.0`'s `2bc81dc6` and back to at least 2026-07-24) and the
> release assets build regardless, since `server-release.yml` is a separate
> workflow. Treat a red `Rust Tests` as **standing, diagnosed, and owned** —
> never as "pre-existing, therefore fine". Its sibling failure in the same
> workflow is `Security Audit`: 15 RUSTSEC advisories, all transitive
> (`bytes` 2026-0007, `quick-xml` ×2, `quinn-proto` ×2, `rkyv`, `rsa` 2023-0071,
> `crossbeam-epoch`, plus unmaintained `proc-macro-error`). **Both were being
> fixed in a concurrent session as this was written** (that session held the
> tree's rw lock and was editing `stats.rs`, `rust-tests.yml`, and adding a
> `security-audit.yml`), so the red state above is the *release-time* record,
> not necessarily today's. The durable part is the gate gap, which no fix to
> either failure removes: pin or match the CI toolchain if the release gate is
> ever meant to predict CI.

**2026-07-26, v0.17.0 webapp swap follow-up (infra reply `cfe80f81` on thread
`6b9a2ae5`, deploy thread `e05b6f2e`): our §2c ask REDELIVERED after completion;
m4m verified end state instead of replaying.** Our webapp relay (msg `6b9a2ae5`)
reached m4m a second time at 16:16 UTC, ~12 min after the swap had landed
(16:04 UTC, home-network `d78bfe9`, recorded against relay id `01484672`) —
byte-identical ask, same assertions. m4m did not re-run the recipe; it answered
"is the end state already true?": entry chunk `archive-CYB_PMQN.js` 200 with the
marker ×2 over the wire, old chunk `archive-DCMJMiIB.js` **and** old CSS
`archive-BBzvspm0.css` both 404 (a swap, not a cache), healthz trio 200,
on-disk marker ×2, rollback point `webapp-preswap-20260726-180402-cchv-v0.16.0`
intact, asset diff exactly the announced set. Those probes ran *from m4m* —
local, as infra itself flagged; our ac-mbm5 leg (above) remains the independent
one, and no further probe is owed.

- **Both records now agree the v0.17.0 analytics eyeball is OURS and open.**
  home-network `af17400` corrected their `d78bfe9` ("nothing is owed here"),
  which had contradicted our hold-open instruction — pre-release screenshots
  part-serve the look, they do not close the item. Two eyeball items stand: the
  v0.17.0 analytics look (**ours** — the close travels by name over the relay
  once a person has actually looked) and the standing v0.12.0 degraded-hint
  (thread `3dc909f8`). Also dual-homed in this project's file memory
  (`open-eyeball-items`), per the single-homing lesson above.
- A replay would have been *safe* but not *clean*: the recipe's `LIVE_VER`
  guard prints "already cchv-v0.17.0 — redeploying (idempotent)" and prunes
  nothing, but mv-then-cp would still have minted
  `webapp-preswap-<stamp>-cchv-v0.17.0` — a decoy rollback point whose name
  says "preswap" while its contents are the version already live, sitting
  ahead of the real v0.16.0 one on a `staging/` already at 101 entries and
  flagged for a prune. End-state check first, recipe idempotency second; the
  sender-side consequence (end-state-shaped assertions) is now a §2c relay
  rule above.
- Thread `e05b6f2e` is complete at 2 of 2. The joined integration check
  (project-scoped `model_distribution` rendering real data) was already done
  in `d78bfe9`. One pending cchv item remains on m4m, unrelated to v0.17.0:
  the staged, unrun sync-daemon swap (`staging/cchv-sync-daemon-aa16b77`) —
  to be sequenced by its own relay, not off the back of this thread.

**2026-07-26, v0.17.0 thread closed both ways (m4m reply `15919dcd` on the
records meta-thread `9dc3f5ea`): converged, and "thread done" ≠ "nothing
outstanding".** m4m banked the convergence as home-network `dbee8b5` + CONTEXT
`2726d24` (both pushed) and declared thread `e05b6f2e` complete at 2 of 2 on
their side too — the deploy line is closed everywhere, and the **only** item
still open on v0.17.0 anywhere is our analytics eyeball, which they hold and
will not touch until our close arrives by name. What their reply adds:

- Their message-1 phrase (`613a3d7`) "closes clean, nothing owed either way"
  is **explicitly superseded** over there: the *deploy* owes nothing either
  way; the *eyeball* is owed, by us. "Thread done" and "nothing outstanding"
  are different claims, and this thread is the case that separates them — a
  closing summary must say which of the two it is making.
- The eyeball state is recorded on their side as *convergence, not agreement*:
  our `1edf1820` re-opened it before their `af17400` did, neither reacting to
  the other. Independent records reaching the same state is the stronger fact,
  and it is written down as such.
- Dual-homing is now the rule on both sides: their two open eyeball items
  (v0.17.0 analytics + the standing v0.12.0 degraded-hint, thread `3dc909f8`)
  live in `hosts/m4m.md` *and* their durable project memory, and the general
  form went to `CONTEXT/PATTERNS/agent-relay.md` — an item homed in exactly
  one doc stops being tracked the first time that doc is skimmed.
- The redelivery pair is complete and non-overlapping: sender half is our §2c
  end-state-assertions rule (above), handler half is one clause in the
  at-least-once bullet of `CONTEXT/PATTERNS/agent-relay.md`, each citing the
  other rather than restating it.
- Verification independence stays labelled: their caveat bullet now names our
  ac-mbm5 wire probes as the independent leg — two legs, local (theirs) and
  cross-machine (ours), never one merged "verified". No probes were run off
  their message, as asked.
- The sync daemon is **fenced, not queued** on m4m:
  `staging/cchv-sync-daemon-aa16b77` is recorded as staged-and-unrun, survives
  any future `staging/` prune, and carries the general form — a staged
  artifact sitting in `staging/` is not a standing invitation to deploy it;
  the relay that names it is the authority, and an adjacent finished thread is
  not that relay.

**2026-07-26, our v0.17.0 verification banked by infra (m4m reply `910ee098`,
thread `e05b6f2e`): the eyeball survives the most tempting non-close, and §2b
gains the asset-provenance scoping.** m4m banked our verification report as
home-network `535f4c3` (`hosts/m4m.md`) + CONTEXT `8bc0e76` (PATTERNS) and
inferred **no close** — recorded on their side as: a thorough verification
report from the item's owner is the most tempting non-close there is — right
party, right surface, reads like completion, still not the trigger. Both
eyeball items stay open both sides; the only next action on this thread is our
close by name, and nothing else is owed either way. What their reply adds:

- **Three assertion shapes from our verification are now portable** in
  `CONTEXT/PATTERNS/ci-release.md`, as an extension of the joined-check rule
  (referenced there, not restated): (1) assert the headline fix by its
  NEGATIVE — the "Not reported by this hub version" dash ABSENT, since
  "$1,502 appeared" is satisfiable by the old path (global rollup
  relabelled); they had used the same discriminator in their headless render
  but recorded it as an observation, and adopted our framing as the
  assertion. (2) The conservation check — per-model `token_count` summing
  *exactly* to `total_tokens` (983,408,016 both ways) generalizes to any
  release that adds a breakdown array beside a pre-existing total: a dedup
  bug surfaces as inflation, which every "the card renders" check passes.
  This one they did not have. (3) Search containment as four behaviors
  (322 px journal section vs the ~3000 px wall).
- **The same-host boundary is now a rule on their side**, sharpened off our
  volunteered note: their handler sessions run ON m4m, so the entire
  cross-machine evidence for v0.17.0 is ONE command — our bare ac-mbm5 curl,
  `scutil`-verified, no ssh wrapping. Their formulation, added to
  `PATTERNS/agent-relay.md` beside the machine-claim rule: "verified from
  <host>" scopes to a *command*, not to a report — a sender on the target
  host is indistinguishable from one off it in the transcript.
- The ~6 min chip-vs-API divergence is recorded there as *measured*, with the
  direction rule: a split release does not remove the window, it chooses its
  DIRECTION — binary-first means the client lags and degrades to the fallback
  dash; the reverse spends the same window rendering a dead card. Agreed
  correct against the 900 s ceiling; no action.
- **Rust Tests: recorded on their side, not waved off — and their forward
  flag is adopted as the §2b provenance-scoping paragraph above.**
  `hosts/m4m.md` carries the failure with a named owner, the cause (clippy
  1.97.0 `manual_filter` at `stats.rs:2733`; the gate's clippy 0.1.96 could
  not see the lint — their kept sub-lesson: a release gate pinned to an older
  toolchain reports green for lints it cannot run), the 15 transitive RUSTSEC
  advisories, and the reason it retracts no swap (asset provenance —
  `server-release.yml` is separate and green). Status since their message:
  the fix landed as `258345fa` and `Rust Tests` is **green on `main`** (run
  `30210223600`, 2026-07-26 16:21Z) — clippy fixed behavior-preserving, 12 of
  15 advisories cleared by `cargo update`, the 3 unactionable ones ignored
  with reasons in `.cargo/audit.toml`, and cargo audit moved off the push
  gate to `security-audit.yml` (weekly + dispatch + `Cargo.lock` path
  filter): a time-dependent check should gate dependency changes, not every
  push.
- Our chmod 755 catch and the 55 MB size correction are banked in their
  deployment record; both our refs (`1edf1820`, `ba830470`) resolve on their
  side.

**2026-07-26, Rust Tests loop closed both ways (m4m ack `3c7909c9`, thread
`e05b6f2e`): the fix independently verified, the m4m record moved to FIXED,
and the reshuffle rule gains its worked counterexample.** Infra took no action
beyond the record move we invited, and verified both our statements from their
side rather than filing them on report — `gh run view 30210223600` returns
conclusion=success on head `258345fa`, branch `main`, 2026-07-26 16:21Z, and
`31d25324` shows the §2b paragraph. What their ack lands:

- **`hosts/m4m.md` (home-network `9c7e15b`, pushed) now reads "was RED,
  cchv-owned, now FIXED and GREEN"**, carrying the fix content
  (behaviour-preserving `manual_filter`, 12/15 advisories cleared by `cargo
  update`, the 3 ignored with reasons in `.cargo/audit.toml`) with the run id
  as the citation. The pre-swap reasoning stays in place unedited — the deploy
  stood on `server-release.yml` being green, and that history is what makes
  the rule legible later.
- **The workflow-set reshuffle earned its own bullet there because the same
  commit demonstrates it**: `258345fa` moved cargo audit into
  `security-audit.yml`, so the workflow set their entry names is not the set
  the next swap should check — re-read the tag's checks at swap time. A rule
  and its worked counterexample landing in one commit is the cheapest
  teaching case; the §2b provenance paragraph above now carries the same
  clarifier.
- **The portable half is banked**: `CONTEXT/PATTERNS/ci-release.md` § "A red
  badge blocks a deploy only if it owns the asset" (CONTEXT `3866fed`) —
  provenance-not-seniority, the re-derive-on-reshuffle clause, the
  "time-dependent check gates dependency changes, not every push" shaping,
  and the pinned-toolchain skew corollary. It points at `hosts/m4m.md` for
  the detail and notes our §2b carries the same rule; nothing restated in
  either direction, per the pattern/repo split.
- Unchanged both sides: the two eyeball items stay open and the v0.17.0
  caveat stays UNVERIFIED. Their ack is not a close and neither is this
  record — the close travels by name, from us, after a human has looked.

**2026-07-26, webapp `v0.17.0` → `v0.17.1` swap (thread `b55e5bb1`, infra
report `96749ab9`, home-network `4880890`): the first deliberately
split-version state — and the deploy that broke the byte-identical-CSS
inference.** Landed 21:34:45 local / 19:34Z, run on m4m (local execution
path, no ssh). All assertions green: entry chunk `archive-CYB_PMQN.js` →
`archive-DngmKaZQ.js`, the predicted chunk matching pre-swap and served
post-swap; marker `cchv-v0.17.1` ×2 in the release bundle and in the served
chunk; healthz trio 200. Nothing was staged beforehand, so the recipe staged
straight from the release artifact and the staged-vs-released diff was the
announced no-op — the release was the sole source of truth. Rollback point:
`staging/webapp-preswap-20260726-213445-cchv-v0.17.0` (two `mv`s, no
restart). Notes that outlive the green:

- **The binary was NOT swapped, and infra verified that rather than merely
  not doing it**: pid 77531 unchanged, uptime 3h37m spanning the deploy,
  `~/.local/bin/cchv-hub` mtime still 17:57 (the v0.17.0 swap). No codesign,
  no bootout/bootstrap, no non-200 window at any point. First time the hub
  (v0.17.0) and webapp (v0.17.1) sit at different tags on purpose, and it
  reads clean. The test infra banked for whether a release needs a §2b half
  at all needs BOTH checks, because either alone reaches the wrong answer:
  an empty `git diff -- crates/` still leaves a version surface to disagree,
  and no served version string still leaves behaviour to diverge.
- **The CSS check's measured counterexample.** The stylesheet is
  byte-identical AND same-named across this swap (`archive-DetcOCbl.css`),
  yet the release re-sizes the Analytics chart cards (`3093718f`) via an
  `items-start` utility already emitted in that stylesheet from use
  elsewhere — the change in *which classes are applied* rode the JS chunk.
  The recipe printed exactly the reassurance that inference produces, i.e. a
  passing check asserting something it cannot know. Both sides corrected the
  same day: infra's `tools/cchv-webapp-deploy` report text now states the
  narrow fact (no token moved) and says outright it does not settle whether
  a look is owed, and the `hosts/m4m.md` standing rule ("a byte-identical
  stylesheet cannot have changed how anything renders") is struck with this
  counterexample (home-network `4880890`); our handoff-shape bullet and the
  narrowing rule above are edited to match in the same commit as this entry.
- **Eyeball backlog unchanged at two items, both ours to close by name**:
  the v0.17.0 analytics look (analytics-ux-costs 7.1 residue) and the
  v0.12.0 degraded-hint (thread `3dc909f8`). v0.17.1 adds no third item and
  discharges neither — the chart-card re-size is part of the v0.17.0-line
  analytics look, which is still open, so it lands inside an existing open
  item rather than minting a new one.
- Still pending on m4m, untouched by this deploy and fenced, not queued:
  the staged, unrun sync-daemon swap (`staging/cchv-sync-daemon-aa16b77`),
  waiting on its own relay from us.

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
- **Two kinds of secret, two read paths** (home-network #31). The hub tokens are
  *static* values that a human minted, mirrored into `kv/`, and read with
  `bao_kv` → `.data.data[<field>]`. The **DB password is different**: it is
  becoming a credential bao itself owns and rotates
  (`database/static-creds/cchv-svc`, `rotation_period=30d`), read with
  `bao_static` → `.data.password`. Both the path and the response shape differ,
  which is why the launcher has two functions rather than one parameterized one.
  Resolution order for the DB password is **`bao_static cchv-svc` → the legacy
  `kv/infra/cchv/pg1` mirror → `op` → last-known-good**; the launcher therefore
  behaves identically before and after the static role exists, so the cutover
  needs no synchronized deploy. Drop the mirror fallback once the 1P item
  `cchv - app role @ pg1` is retired.
- **The last-known-good floor is not equally valid for both.** For the static
  tokens it is always correct. For a *rotating* DB password it expires: a restart
  while bao is unreachable renders the previous password, which pg1 may no longer
  accept. The launcher logs an explicit WARN naming that case, so a hub that then
  fails to authenticate is diagnosable from the launch log instead of looking
  like a pg1 outage. The fix in that state is to restore bao reachability — not
  to touch pg1, and *not* to write a rotated value back into `kv/`, which would
  re-create the hand-synced mirror #31 exists to eliminate.
- **Cutover ordering when the static role is created** — creating it rotates the
  pg password **immediately**, so:
  1. install the static-creds-capable `cchv-launch` first (safe any time — it is
     a no-op until the role exists; verify with
     `cchv-launch hub --render-only` against a scratch `CFG_DIR` and `cmp` the
     result with the live runtime file);
  2. infra creates `database/static-roles/cchv-svc`;
  3. **bounce `dev.cchv.hub` promptly** (§2b ceremony, or just
     `launchctl bootout`/`bootstrap` — no binary change) so the launcher
     re-renders from the rotating credential. The *running* hub keeps its open
     pool and then fails as connections recycle; until **cchv Gitea #25**
     (fail-fast on SQLSTATE `28P01`) ships it will not self-heal, so this bounce
     is a required step rather than a nicety.
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
- **Rotation** — different per secret now:
  - *Hub tokens (static)*: rotate in 1P, re-copy to bao per home-network
    `docs/secrets-standard.md`, then `launchctl unload/load` the job — the next
    launch re-renders.
  - *DB password (bao-owned)*: **nobody rotates it by hand** — bao does, on its
    own period. Pickup is a hub relaunch, which re-reads
    `database/static-creds/cchv-svc`, and since cchv Gitea #25 the hub triggers
    that relaunch **itself**: a credential watchdog probes with a fresh
    connection every 30 s and, after 3 consecutive `28P01` rejections (~75-90 s),
    exits non-zero so `KeepAlive` restarts it. A rotation therefore heals
    unattended in under two minutes with no manual bounce.
    - Only `28P01` counts. A pg1 outage, a `MagicDNS` flake, a pool timeout or
      any other SQLSTATE **resets** the strike run, so the hub rides out a
      database outage exactly as before (verified end-to-end: 130 s of total DB
      unavailability, zero strikes, process stayed up serving a `503`
      `{"db":"down"}` health check).
    - Reading the log after the fact: three
      `WARN … rejected the hub's credential strikes=N limit=3` lines followed by
      an `ERROR … appears to have been rotated` is a *successful* rotation
      pickup, not an incident. The same pattern repeating every 5 minutes
      (`ThrottleInterval`) is an incident — it means the relaunch is not getting
      a valid credential, so check bao reachability per the stale-floor note
      above.
    **The first automatic rotation is a known date: 2026-08-24T13:38:54Z**
    (`last_vault_rotation` 2026-07-25T13:38:54Z + `rotation_period` 2592000 s,
    both read live from bao by infra on 2026-07-25; relay thread `40d5df93`).
    That is therefore the hard deadline for #25 — after it, an un-bounced hub
    serves 503s until someone notices. It only moves if `cchv-svc` is rotated
    manually early, which resets the clock; infra owns that case and will bounce
    `dev.cchv.hub` themselves and tell us. Infra's copy of this fact lives in
    home-network `hosts/configs/proxmox1/openbao.md`.

## 3c. Journal-entries distiller (`scripts/cchv-distill.py`)

> **Hourly** launchd job on the hub machine (m4m) that distills archived
> sessions into per-(date, project) journal entries (openspec `journal-entries`
> + `distiller-self-healing`, issues #12 and Gitea backlog). Catch-up-based: the
> work list is `GET /v1/journal/pending` (missing or dirty groups), so
> sleep/downtime and late-arriving syncs only delay entries. **Install only
> after the hub carries the journal endpoints** (migration
> `0002_journal_entries.sql`).
>
> **Self-healing cadence (`distiller-self-healing`).** The job ticks hourly
> (`StartInterval 3600`), *not* nightly. This is what bounds journal staleness
> to ~1h: some tick always lands after the 04:00 UTC logical day-close whatever
> the DST offset (the retired 05:30-*local* calendar run fired 03:30 UTC under
> CEST — before the close — and so never saw the day that just ended, the root
> cause of the 2026-07-22→24 stall). An idle tick is one loopback
> `GET /v1/journal/pending` + exit, no LLM call. Hub calls retry transient
> failures (connection errors / 5xx, 3× / 30s; 4xx re-raised at once), so a pg
> or DNS blip costs seconds not a whole run, and a failed tick recovers at the
> next hour, never +24h.

- **Install** (on m4m):

  ```bash
  install -m 755 scripts/cchv-distill.py ~/.local/bin/cchv-distill
  cp scripts/dev.cchv.distiller.plist ~/Library/LaunchAgents/
  # first install:
  launchctl load ~/Library/LaunchAgents/dev.cchv.distiller.plist
  # updating an already-loaded job (e.g. the StartInterval change) — reload:
  launchctl bootout  gui/$(id -u)/dev.cchv.distiller 2>/dev/null || true
  launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.cchv.distiller.plist
  ```

  Requires `uv` on PATH (PEP 723 script). **LLM backend (default `aiproxy`):**
  an OpenAI-compatible HTTP call to infra's CLIProxyAPI node
  (`https://aiproxy.internal`, model **`gpt-5.6-sol`**,
  `reasoning_effort=low`) — no `claude -p`, so no shared-OAuth contention (the
  old #13 failure mode when an interactive Claude session ran concurrently).
  `--backend claude` keeps `claude -p` (needs the `claude` CLI + `zsh -lc`
  `CLAUDE_CONFIG_DIR`) as a fallback. Runs **hourly** (`StartInterval`) + on
  load; logs `/tmp/cchv-distiller.{log,err}`. `CCHV_RETRY_SLEEP_SECS` tunes the
  transient-retry backoff (default 30s).
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
- **Monitoring** (`distiller-self-healing`): `GET /v1/healthz/journal`
  (unauthenticated, Gatus-shaped like `/v1/healthz/ingest`) is **503** when a
  closed logical day *within the forward horizon* still has pending groups whose
  latest data arrived more than `grace_secs` ago (default 7200), else **200**.
  Two query params keep the policy in Gatus, no hub redeploy: `grace_secs` (how
  long undrained work may sit) and `within_days` (default 7 — matches
  `--horizon-days`; **essential**, since the archive holds hundreds of
  never-auto-distilled historical pending groups that would otherwise pin it
  red forever). This catches *all* stall modes — including runs that succeed but
  distill nothing (the DST-race bug) — which a distiller-side dead-man ping
  cannot. Relay a `cchv-journal` Gatus check to infra alongside `cchv-ingest`.

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
>
> Live instance as of 2026-07-19: m4m serves the `cchv-v0.10.7` webapp (chip reads
> `0.10.7`) while `aa16b77` is staged-but-not-swapped on the daemon. The chip is
> maximally misleading in exactly this state — it names a rev whose daemon half
> is not running.
>
> As of the v0.11.2 deploy (2026-07-20) the chip and the **hub** binary name the
> same rev again, because both halves moved in one swap. That agreement is a
> coincidence of that deploy, **not** a property of the system: the next
> static-only release re-opens the divergence silently, so keep probing the
> installed file. The **sync-daemon** half of `aa16b77` is still owed on both
> Macs — the chip says nothing about it either way.

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

On **m4m** read-auth is `trust_tailscale_identity`, so the bearer token is not
what grants access — the tailnet peer identity is, and `$HOST` must be the
MagicDNS name (`http://hub.internal:8787`), **not** the loopback
bind or the raw tailnet IP. See the "Hub topology on m4m" note in §3 for the
probe matrix: on `127.0.0.1:8790` every read route 401s while `/v1/healthz`
returns 200, which reads like a broken deploy and is not one.

## Notes & current limitations

- **`raw` fidelity (MVP):** the archived `raw` JSONB is the normalized record
  (lossless for all modeled fields). Byte-exact original passthrough is a
  planned enhancement.
- **Incremental sync (MVP):** a changed session file is re-parsed in full and
  re-sent; the hub's idempotent ingest drops duplicates. Byte-offset
  "parse only new lines" and `notify`-based watching are planned optimizations.
- **JOURNAL-LEVEL SEMANTIC SEARCH SHIPPED (cchv-v0.12.0, 2026-07-21):**
  hub-local candle embedder (bge-small-en-v1.5, CLS pooling + bge query prefix),
  `journal_embeddings` side table (migration `0004`, plain `real[]` — no
  pgvector at this scale), `mode=keyword|semantic|hybrid` on the `/v1/search`
  journal leg (RRF k=60, graceful `journal_degraded` fallback). Acceptance
  rerun of the 2026-07-19 six-query paraphrase probe against the live corpus
  (101 entries, real weights): **5/6 targets in top-5 via hybrid (4× rank-1,
  1× rank-2) vs 0/6 keyword baseline**; the sixth query's target entry no
  longer exists in the regenerated corpus (its analogue describes the PR
  *submission*, not the *acceptance* the query asks about) — verified as
  ground-truth drift, not model choice, by rerunning under bge-base (3.3×
  larger: also missed) and all three pooling configs. Model staging recipe:
  §2 "semantic journal search". **Message-scale embeddings remain the later
  phase** — that's where pgvector/`halfvec` earns its keep. The schema already
  reserves a `message_embeddings` side table so those land without a breaking
  migration. **MCP context server:**
  its intended job — an agent pulling archive context as native tools — is already
  delivered by the hub read API + the `cchv-find` skill (journal-first retrieval), so it
  is now optional; build it only if a **non-Claude-Code MCP client** (Desktop/Cursor/…)
  needs the archive. A useful MCP server would also want semantic search underneath it —
  the transport isn't the lever, retrieval quality is. **pg1 disk envelope for the embeddings backfill**
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

