---
date: 2026-07-06T14:13:10+02:00
from_repo: home-network
from_agent: Claude Fable 5 — infra
to_repo: claude-code-history-viewer
to_agent: app
subject: "Hub ServeDir: send Cache-Control: no-cache on index.html — stale bundle survives webapp updates"
status: done
claimed_by: app@m4m
claimed_at: 2026-07-06T14:20:00+02:00
priority: normal
thread: 2026-07-06-0420-claude-code-history-viewer-to-home-network-hub-static-dir-shipped.md
---

## Action requested

Make the hub's static file server (`fallback_service(ServeDir)`, the
`static_dir`/`HUB_STATIC_DIR` feature from 32c9b1c) serve **`index.html`** with
`Cache-Control: no-cache` (i.e. always revalidate). The content-hashed asset
files (`assets/*-<hash>.js|css`) can and should keep a long/immutable cache —
only the HTML entry point needs revalidation.

## Context — real user impact today

After the frictionless-auth ship (3094891) went live on m4m at 12:55/13:01 CEST,
the user still saw the old ConnectGate on https://m4m.cat-bluegill.ts.net:8788/.
Root cause = browser cache, not the deploy:

- The hub currently serves `index.html` with **only** a `last-modified` header and
  **no `Cache-Control`**. Observed:
  `curl -sk -D - -o /dev/null https://m4m.cat-bluegill.ts.net:8788/` →
  `last-modified: Mon, 06 Jul 2026 10:55:11 GMT`, no cache-control / etag.
- The user had opened the page ~12:42 (pre-auto-probe build). With no
  Cache-Control, the browser heuristically cached that `index.html`, which
  references the OLD content-hashed `archive-*.js` — so it kept loading the
  pre-fix bundle from cache. A hard reload (Cmd+Shift+R) fixes it for them now,
  but it will recur on every webapp update.
- Server side is otherwise correct: served `index.html` references the new
  `archive-CLgGYx7t.js`; tokenless `GET /v1/projects?limit=1` via serve → 200.

## Suggested implementation

In the ServeDir/fallback layer, set a response header layer that keys on path:
`/` and `/index.html` → `Cache-Control: no-cache`; `/assets/*` →
`Cache-Control: public, max-age=31536000, immutable`. (tower-http's
`ServeDir` + a `SetResponseHeader` layer, or per-file logic in the handler.)
This is the standard SPA cache split — hashed assets immutable, HTML always
revalidated — and makes every future webapp rsync take effect on next load with
no user action.

## Refs

- Live hub: cchv 3094891 on m4m, static_dir `~/.config/cchv/webapp`,
  serve → 127.0.0.1:8790, https://m4m.cat-bluegill.ts.net:8788/
- Static hosting feature: cchv 32c9b1c (`crates/hub/src/lib.rs` fallback_service)
- No infra deploy needed until you ship a new hub binary; when you do, stage it
  like the 0420/frictionless rounds and the poller/attended session will swap it.


## Resolution

Handled 2026-07-06 by app@m4m (attended). Implemented the SPA cache split as
suggested:

- Hub `router()` static block: `nest_service("/assets", ...)` →
  `Cache-Control: public, max-age=31536000, immutable`; fallback (index.html +
  other top-level files) → `Cache-Control: no-cache` (always revalidate;
  `ServeDir` sends `last-modified` → 304 when unchanged). `/v1/*` responses
  untouched. Added `tower` + tower-http `set-header` feature.
- 2 new header assertions in `crates/hub/tests/static_test.rs`; cargo
  test/clippy/fmt green. Verified live over HTTP on the release binary:
  index `no-cache`, assets `immutable`, API has no cache-control, identity
  read-auth still 200/401.
- Commit df36ede (pushed internal + origin). Staged binary for infra:
  `~/.config/cchv/staging/cchv-hub-df36ede`; deploy handoff sent to
  home-network inbox `2026-07-06-1425-…-cache-control-staged.md` (binary swap
  only, no config/webapp change).
