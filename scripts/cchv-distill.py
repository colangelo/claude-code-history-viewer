#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["requests"]
# ///
"""cchv-distill — journal-entries distiller for the cchv archive hub.

Drains the hub's journal pending list: for each (entry_date, project_path)
group it fetches the sessions' archived messages, generates a journal entry
with a single `claude -p` call (Haiku-tier by default, single turn, no tools),
validates it, and POSTs it back to the hub. State lives entirely in the hub
(`generated_at` watermarks), so runs are idempotent and resumable — a missed
day is just still pending on the next run.

Entry schema and prompt ported from engineering-notebook `src/summarize.ts`
(Apache-2.0, Prime Radiant): headline · 2-5 sentence summary · 3-8 topics ·
open_questions (dropped threads) · SKIP sentinel for non-substantive days.
See openspec/changes/journal-entries/ for the full contract (issue #12).

Modes:
  cchv-distill                      # forward: drain pending within --horizon-days
  cchv-distill --dry-run            # generate + validate, print, no POST
  cchv-distill --backfill --from 2026-05-01 --limit 50   # bounded, newest-first
  cchv-distill --backfill --limit 20                     # next 20, resumable

Secrets (house launchd-resilience contract — never prompt headless):
  1. $CCHV_HUB_TOKEN                (explicit override / testing)
  2. OpenBao AppRole                (~/.config/cchv/bao-approle, kv/infra/cchv/hub-tokens)
  3. `op read`                      (ATTENDED runs only; skipped when
                                     CCHV_NONINTERACTIVE=1 or stdin isn't a tty)
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass, field
from datetime import date, datetime, timedelta, timezone
from pathlib import Path

import requests

# --- config -----------------------------------------------------------------

DEFAULT_HUB_URL = "http://127.0.0.1:8790"  # hub is loopback-bound on m4m
DEFAULT_MODEL = "haiku"  # claude CLI alias; pin a full id via --model if needed
BAO_ADDR = os.environ.get("BAO_ADDR", "https://secrets.cat-bluegill.ts.net")
APPROLE_FILE = Path.home() / ".config/cchv/bao-approle"
PROMPT_BUDGET_CHARS = 120_000  # total transcript chars per LLM call
CLAUDE_TIMEOUT_SECS = 300
HTTP_TIMEOUT_SECS = 30


def log(msg: str) -> None:
    ts = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    print(f"[cchv-distill {ts}] {msg}", file=sys.stderr)


def non_interactive() -> bool:
    return os.environ.get("CCHV_NONINTERACTIVE", "0") == "1" or not sys.stdin.isatty()


# --- token resolution (bao-first, mirrors scripts/cchv-launch.sh) ------------


def bao_token() -> str | None:
    if not APPROLE_FILE.is_file():
        log(f"no AppRole creds at {APPROLE_FILE}")
        return None
    creds = dict(
        line.split("=", 1)
        for line in APPROLE_FILE.read_text().splitlines()
        if "=" in line
    )
    role_id, secret_id = creds.get("role_id"), creds.get("secret_id")
    if not role_id or not secret_id:
        log("AppRole creds file malformed")
        return None
    try:
        r = requests.post(
            f"{BAO_ADDR}/v1/auth/approle/login",
            json={"role_id": role_id, "secret_id": secret_id},
            timeout=10,
        )
        r.raise_for_status()
        client_token = r.json()["auth"]["client_token"]
        host = os.uname().nodename.split(".")[0]
        r = requests.get(
            f"{BAO_ADDR}/v1/kv/data/infra/cchv/hub-tokens",
            headers={"X-Vault-Token": client_token},
            timeout=10,
        )
        r.raise_for_status()
        return r.json()["data"]["data"].get(f"{host}_token") or None
    except (requests.RequestException, KeyError, OSError) as e:
        log(f"bao read failed: {e}")
        return None


def op_token() -> str | None:
    if non_interactive():
        log("non-interactive — skipping op fallback (would prompt Touch-ID)")
        return None
    host = os.uname().nodename.split(".")[0]
    ref = f"op://AC-DevOps/cchv - archive hub tokens/{host} token"
    try:
        out = subprocess.run(
            ["op", "read", ref], capture_output=True, text=True, timeout=120
        )
        return out.stdout.strip() or None
    except (OSError, subprocess.TimeoutExpired):
        return None


def resolve_token() -> str:
    tok = os.environ.get("CCHV_HUB_TOKEN") or bao_token() or op_token()
    if not tok:
        log("FATAL: no hub token (env CCHV_HUB_TOKEN, bao, op all unavailable)")
        sys.exit(1)
    return tok


# --- hub client ---------------------------------------------------------------


@dataclass
class Hub:
    url: str
    token: str
    session: requests.Session = field(default_factory=requests.Session)

    def __post_init__(self) -> None:
        self.session.headers["Authorization"] = f"Bearer {self.token}"

    def get(self, path: str, **params) -> requests.Response:
        r = self.session.get(
            f"{self.url}{path}",
            params={k: v for k, v in params.items() if v is not None},
            timeout=HTTP_TIMEOUT_SECS,
        )
        r.raise_for_status()
        return r

    def pending(self, from_date: str | None, limit: int) -> list[dict]:
        return self.get("/v1/journal/pending", limit=limit, **{"from": from_date}).json()

    def session_messages(self, session_id: int) -> list[dict]:
        msgs: list[dict] = []
        offset = 0
        while True:
            r = self.get(
                f"/v1/sessions/{session_id}/messages", limit=500, offset=offset
            )
            page = r.json()
            msgs.extend(page)
            total = int(r.headers.get("X-Total-Count", len(msgs)))
            offset += len(page)
            if not page or offset >= total:
                return msgs

    def post_entry(self, payload: dict) -> None:
        r = self.session.post(
            f"{self.url}/v1/journal/entries", json=payload, timeout=HTTP_TIMEOUT_SECS
        )
        if r.status_code >= 400:
            raise RuntimeError(f"POST /v1/journal/entries {r.status_code}: {r.text[:500]}")


# --- transcript building ------------------------------------------------------


def _content_text(content) -> str:
    """Flatten a normalized message content value to plain text."""
    if content is None:
        return ""
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = []
        for item in content:
            if not isinstance(item, dict):
                continue
            t = item.get("type")
            if t == "text":
                parts.append(item.get("text", ""))
            elif t == "thinking":
                parts.append(f"[thinking] {item.get('thinking', '')[:500]}")
            elif t == "tool_use":
                name = item.get("name", "?")
                inp = json.dumps(item.get("input", {}), ensure_ascii=False)[:300]
                parts.append(f"[tool_use {name}] {inp}")
            elif t == "tool_result":
                c = item.get("content")
                text = c if isinstance(c, str) else json.dumps(c, ensure_ascii=False)
                parts.append(f"[tool_result] {(text or '')[:500]}")
        return "\n".join(p for p in parts if p)
    if isinstance(content, dict):
        return json.dumps(content, ensure_ascii=False)[:500]
    return str(content)


def truncate(text: str, budget: int) -> str:
    """Deterministic head+tail truncation with an explicit marker."""
    if len(text) <= budget:
        return text
    head = int(budget * 0.6)
    tail = budget - head
    return f"{text[:head]}\n\n[... transcript truncated ...]\n\n{text[-tail:]}"


def build_transcript(hub: Hub, session_ids: list[int]) -> str:
    per_session = max(PROMPT_BUDGET_CHARS // max(len(session_ids), 1), 4_000)
    chunks = []
    for sid in session_ids:
        msgs = hub.session_messages(sid)
        lines = []
        for m in msgs:
            if m.get("is_sidechain"):
                continue
            role = m.get("role") or m.get("message_type") or "?"
            text = _content_text(m.get("content"))
            if text.strip():
                lines.append(f"{role}: {text}")
        chunks.append(truncate("\n".join(lines), per_session))
    return "\n\n---\n\n".join(chunks)


# --- entry generation ---------------------------------------------------------

PROMPT_TEMPLATE = """You are writing an engineering journal entry from AI coding-agent \
session transcripts. Write from the developer's perspective: problems solved, features \
shipped, failures hit, and threads that were started but dropped.

Some transcripts may be truncated or contain "[... transcript truncated ...]" markers \
where content was cut. Work with whatever is available. Do not skip merely because \
transcripts are short — if real engineering work was discussed, write the entry.

Write an entry for {entry_date}, project "{project}".

Respond with a single JSON object and nothing else (no code fences, no commentary):
{{"status": "entry", "headline": "<one-line summary of what happened>", "summary": \
"<one paragraph, 2-5 sentences: wins, failures, dropped threads>", "topics": \
["<3-8 short topic phrases>"], "open_questions": ["<0-5 phrases for unresolved \
issues or dropped threads>"]}}

If the transcripts show no substantive engineering work (pure chit-chat, empty \
sessions, only automated health checks), respond instead with:
{{"status": "skip", "skip_reason": "<brief reason>"}}

Transcripts:

{transcript}
"""


def generate(model: str, entry_date: str, project: str, transcript: str) -> dict:
    prompt = PROMPT_TEMPLATE.format(
        entry_date=entry_date, project=project, transcript=transcript
    )
    proc = subprocess.run(
        ["claude", "-p", "--model", model, "--output-format", "json"],
        input=prompt,
        capture_output=True,
        text=True,
        timeout=CLAUDE_TIMEOUT_SECS,
        cwd=Path.home(),
    )
    if proc.returncode != 0:
        raise RuntimeError(f"claude -p failed: {proc.stderr[:500]}")
    wrapper = json.loads(proc.stdout)
    raw = wrapper.get("result", "")
    # tolerate stray code fences despite the instruction
    raw = re.sub(r"^```(?:json)?\s*|\s*```$", "", raw.strip())
    return json.loads(raw)


def validate(entry: dict) -> str | None:
    """Return an error string, or None when the entry is schema-valid."""
    status = entry.get("status")
    if status == "skip":
        return None
    if status != "entry":
        return f"unknown status {status!r}"
    if not (entry.get("headline") or "").strip():
        return "empty headline"
    if not (entry.get("summary") or "").strip():
        return "empty summary"
    topics = entry.get("topics")
    if not isinstance(topics, list) or not 3 <= len(topics) <= 8:
        return f"topics must be a list of 3-8 (got {topics!r})"
    oq = entry.get("open_questions")
    if oq is not None and not isinstance(oq, list):
        return "open_questions must be a list"
    return None


# --- main ---------------------------------------------------------------------


def process_group(hub: Hub, group: dict, model: str, dry_run: bool) -> bool:
    entry_date = group["entry_date"]
    project_path = group["project_path"]
    session_ids = group["session_ids"]
    label = f"{entry_date} {project_path}"
    log(f"distilling {label} ({len(session_ids)} sessions)")

    transcript = build_transcript(hub, session_ids)
    if not transcript.strip():
        entry = {"status": "skip", "skip_reason": "no textual content in sessions"}
    else:
        entry = generate(model, entry_date, project_path, transcript)

    if err := validate(entry):
        log(f"REJECTED {label}: {err} — leaving pending")
        return False

    payload = {
        "entry_date": entry_date,
        "project_path": project_path,
        # Echo the pending endpoint's snapshot: anchors dirty-detection to the
        # moment this group was READ, so data committing while we generate
        # keeps the group dirty (hub rejects/re-pends as appropriate).
        "as_of": group.get("as_of"),
        "status": entry["status"],
        "headline": entry.get("headline"),
        "summary": entry.get("summary"),
        "topics": entry.get("topics") or [],
        "open_questions": entry.get("open_questions") or [],
        "session_ids": session_ids,
        "model": model,
    }
    if dry_run:
        print(json.dumps(payload, indent=2, ensure_ascii=False))
        log(f"dry-run: validated {label} ({entry['status']}), not POSTed")
        return True
    hub.post_entry(payload)
    log(f"stored {label} ({entry['status']})")
    return True


def main() -> int:
    ap = argparse.ArgumentParser(
        description="cchv-distill — journal-entries distiller for the cchv archive hub"
    )
    ap.add_argument("--hub-url", default=os.environ.get("CCHV_HUB_URL", DEFAULT_HUB_URL))
    ap.add_argument("--model", default=os.environ.get("CCHV_DISTILL_MODEL", DEFAULT_MODEL))
    ap.add_argument("--horizon-days", type=int, default=7,
                    help="forward mode: only process groups newer than this (default 7)")
    ap.add_argument("--backfill", action="store_true",
                    help="process historical groups (newest-first, bounded)")
    ap.add_argument("--from", dest="from_date",
                    help="backfill: date lower bound (YYYY-MM-DD)")
    ap.add_argument("--limit", type=int, default=None,
                    help="max groups this run (default: 50 forward, 20 backfill)")
    ap.add_argument("--dry-run", action="store_true",
                    help="generate + validate + print, never POST")
    args = ap.parse_args()

    if args.from_date and not args.backfill:
        ap.error("--from requires --backfill")
    if args.from_date:
        date.fromisoformat(args.from_date)  # fail fast on bad input

    if args.backfill:
        from_date, limit = args.from_date, args.limit or 20
    else:
        from_date = (date.today() - timedelta(days=args.horizon_days)).isoformat()
        limit = args.limit or 50

    hub = Hub(args.hub_url, resolve_token())
    try:
        groups = hub.pending(from_date, limit)
    except requests.RequestException as e:
        log(f"FATAL: pending query failed: {e}")
        return 1

    if not groups:
        log("nothing pending")
        return 0
    log(f"{len(groups)} group(s) pending (from={from_date or 'archive start'}, limit={limit})")

    failures = 0
    for group in groups:
        try:
            if not process_group(hub, group, args.model, args.dry_run):
                failures += 1
        except (RuntimeError, requests.RequestException, subprocess.TimeoutExpired,
                json.JSONDecodeError, KeyError) as e:
            failures += 1
            log(f"ERROR on {group.get('entry_date')} {group.get('project_path')}: {e}")

    log(f"done: {len(groups) - failures} ok, {failures} failed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
