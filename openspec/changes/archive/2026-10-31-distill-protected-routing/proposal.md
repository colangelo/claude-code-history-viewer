## Why

The distiller sends each (day, project) group's transcript text to one LLM backend. Today that is
a GPT model through aiproxy for every project. Some projects hold client data whose owner allows
only exact Claude model ids, reached on the direct Claude login. The distiller has no way to tell
those groups apart, so client text went to GPT from 2026-07-18 until 2026-10-21, when the
distiller was paused and then moved wholesale onto Claude as an interim. The owner's decision
(2026-10-21): protected projects on Claude, everything else back on GPT.

## What Changes

- The distiller reads a routing file, `~/.config/cchv/distill-routing.toml` (override
  `CCHV_DISTILL_ROUTING`). The file lives outside the repo, so client names never enter this
  public fork. It names the protected model (one exact id) and two lists:
  - **path markers**: case-insensitive substrings of `project_path`, so one marker matches the
    same tree on every host;
  - **content terms**: case-insensitive substrings matched against the full prompt about to be
    sent.
- A group that matches either list is **protected**. It goes to `claude -p --model <exact id>`,
  with any `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` stripped so the call cannot leave the
  direct login. Every other group uses the configured general backend, which is unchanged.
- **Fail closed:** a missing, unreadable or invalid routing file protects every group.
- **A guard on every call:** the serving model is logged for every call (`modelUsage` from
  `claude -p`, the response `model` from aiproxy). A protected result whose `modelUsage` is not
  exactly `{<protected id>}` is REFUSED: not stored, logged as `REFUSED`, and the run exits
  non-zero. The group stays pending, so `/v1/healthz/journal` goes stale and Gatus pages.
- `journal_entries.model` records the serving model rather than the requested one.

## Impact

- `scripts/cchv-distill.py`, `scripts/test_cchv_distill.py`. No hub, webapp or daemon change.
- Ops: write the routing file on m4m before installing. Once the release is proven, remove the
  interim `CCHV_LLM_BACKEND=claude` / `CCHV_DISTILL_MODEL` overrides from the distiller plist.
