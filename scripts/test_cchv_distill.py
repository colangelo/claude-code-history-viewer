"""Tests for the journal distiller's day windowing.

Run: `just distill-test` (or
`uv run --with pytest --with requests pytest scripts/test_cchv_distill.py -q`).

Everything here is offline: the hub is a stub, and no LLM is reachable. What is
under test is which *window* the distiller asks for and what it does when that
window turns out to be empty — the two halves of #35 that live in this script
rather than in the hub.
"""

from __future__ import annotations

import importlib.util
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parent.parent


def _load():
    """Import `cchv-distill.py`, whose hyphen makes it unimportable by name."""
    spec = importlib.util.spec_from_file_location(
        "cchv_distill", REPO / "scripts" / "cchv-distill.py"
    )
    mod = importlib.util.module_from_spec(spec)
    # Register BEFORE exec: `@dataclass` resolves its class's module out of
    # `sys.modules`, so a module executed while absent from it dies with an
    # opaque `'NoneType' object has no attribute '__dict__'`.
    sys.modules[spec.name] = mod
    spec.loader.exec_module(mod)
    return mod


d = _load()


def test_day_start_hour_matches_the_hub() -> None:
    """The hub groups the sessions; this script windows their messages.

    If the two constants drift, every group is distilled from a window that is
    not the window it was grouped by — and nothing fails, it just quietly
    summarizes the wrong hours. So the constant is asserted against its source
    rather than commented next to it.
    """
    src = (REPO / "crates" / "hub" / "src" / "journal.rs").read_text()
    m = re.search(r"pub\(crate\) const DAY_START_HOUR: i32 = (\d+);", src)
    assert m, "DAY_START_HOUR not found in journal.rs — did it move or get renamed?"
    assert d.DAY_START_HOUR == int(m.group(1))


def test_journal_today_folds_the_way_the_hub_does() -> None:
    """The forward horizon is measured from the logical day, not the calendar.

    Between midnight and the fold hour these differ, and that is exactly when
    the nightly tick runs.
    """
    def at(stamp: str) -> datetime:
        return datetime.fromisoformat(stamp).replace(tzinfo=timezone.utc)

    assert d.journal_today(at("2026-08-21T00:46:00")).isoformat() == "2026-08-20"
    assert d.journal_today(at("2026-08-21T03:59:59")).isoformat() == "2026-08-20"
    assert d.journal_today(at("2026-08-21T04:00:00")).isoformat() == "2026-08-21"
    assert d.journal_today(at("2026-08-21T23:30:00")).isoformat() == "2026-08-21"


def test_forward_horizon_anchor_matches_the_hub() -> None:
    """The tick's window and the health check's window must be the same window.

    `/v1/healthz/journal` pages when in-horizon groups sit undrained. If its
    anchor and this script's anchor disagree, it pages for groups the tick will
    never pick up — silently, since both halves still "work". So the hub's
    anchor is asserted against its source, like `DAY_START_HOUR` above.
    """
    src = (REPO / "crates" / "hub" / "src" / "health.rs").read_text()
    m = re.search(
        r"fn horizon_from\([^)]*\) -> NaiveDate \{(.*?)\n\}", src, re.DOTALL
    )
    assert m, "horizon_from not found in health.rs — did it move or get renamed?"
    body = " ".join(m.group(1).split())
    assert body == (
        "(now - chrono::Duration::hours(i64::from(day_start_hour))).date_naive() "
        "- chrono::Duration::days(i64::from(within_days))"
    ), (
        "the hub's journal-health horizon anchor changed. If it no longer folds "
        "by DAY_START_HOUR before subtracting the window, it disagrees with "
        f"journal_today() here and the check pages for undoable work. Body: {body!r}"
    )


def test_day_window_is_half_open_and_shifted() -> None:
    assert d.day_window("2026-08-19") == (
        "2026-08-19T04:00:00Z",
        "2026-08-20T04:00:00Z",
    )


class StubHub:
    """Records the query params of every message fetch, returns one message."""

    def __init__(self, text: str = "hello") -> None:
        self.calls: list[tuple[int, tuple[str, str] | None]] = []
        self.text = text

    def session_messages(self, session_id: int, window=None):
        self.calls.append((session_id, window))
        if not self.text:
            return []
        return [
            {
                "role": "user",
                "is_sidechain": False,
                "content": [{"type": "text", "text": self.text}],
            }
        ]


def test_build_transcript_requests_only_the_group_day() -> None:
    """The bug in one assertion.

    Unwindowed, a session running from the 19th into the 20th delivered all of
    its messages to the 19th's distillation, and the 60/40 head-tail truncation
    guaranteed the 20th survived into the prompt.
    """
    hub = StubHub()
    text = d.build_transcript(hub, [11, 22], "2026-08-19")

    assert [sid for sid, _ in hub.calls] == [11, 22]
    for _, window in hub.calls:
        assert window == ("2026-08-19T04:00:00Z", "2026-08-20T04:00:00Z")
    assert "hello" in text


def test_empty_day_yields_a_skip_and_no_llm_call() -> None:
    """A group whose window holds nothing usable must not reach the model.

    Group membership counts every message; the transcript drops sidechains. A
    day that is all sidechain therefore groups but does not summarize.
    """

    class ExplodingLLM:
        model = "should-not-be-called"

        def __getattr__(self, name):  # pragma: no cover - only on failure
            raise AssertionError(f"the LLM was consulted ({name}) for an empty day")

    posted: list[dict] = []

    class PostingHub(StubHub):
        def post_entry(self, payload):
            posted.append(payload)

    hub = PostingHub(text="")
    llm = ExplodingLLM()
    group = {
        "entry_date": "2026-08-19",
        "project_path": "/w/p",
        "session_ids": [11],
        "as_of": None,
    }
    assert d.process_group(hub, group, llm, dry_run=False) is True
    assert len(posted) == 1
    assert posted[0]["status"] == "skip"


def test_session_messages_pages_on_the_windowed_total() -> None:
    """The paging loop must terminate on the filtered count, not the session's.

    `X-Total-Count` is windowed when a bound is sent (the hub guarantees it), so
    a loop reading the session total would page past the end of the window.
    """

    class Resp:
        def __init__(self, page, total):
            self._page = page
            self.headers = {"X-Total-Count": str(total)}

        def json(self):
            return self._page

    seen: list[dict] = []

    class FakeHub(d.Hub):
        def get(self, path, **params):
            seen.append(params)
            offset = params["offset"]
            # 3 messages in the window; the session holds many more.
            page = [{"i": i} for i in range(offset, min(offset + 500, 3))]
            return Resp(page, 3)

    hub = FakeHub(url="http://stub", token="t")
    msgs = hub.session_messages(7, window=("A", "B"))
    assert len(msgs) == 3
    assert len(seen) == 1, "one page covers the window; it must not keep asking"
    assert seen[0]["from"] == "A" and seen[0]["to"] == "B"


def test_a_failed_tick_record_never_costs_the_run() -> None:
    """The tick record is telemetry about the work, not the work.

    A tick that reached the hub, got a work list, and then failed to record
    itself must still distill. Losing one datapoint in a health endpoint is
    strictly better than losing an hour — and on a sleeping host, a wake — of
    journal entries.
    """
    import requests

    class BrokenHub(d.Hub):
        def __post_init__(self) -> None:
            pass

        def post_tick(self, mode, groups_pending):  # exercise the real body
            self.session = _RaisingSession()
            return d.Hub.post_tick(self, mode, groups_pending)

    class _RaisingSession:
        def post(self, *a, **k):
            raise requests.ConnectionError("hub down")

    hub = BrokenHub(url="http://stub", token="t")
    # No exception escapes: the caller in main() is not guarded, by design.
    hub.post_tick("forward", 3)


def test_tick_record_states_its_mode_and_work_list_size() -> None:
    """An idle tick is recorded too — it is the one whose absence must show.

    `groups_pending` is captured before any LLM call, so it describes the work
    list the tick picked up, not the work it finished.
    """
    sent: list[dict] = []

    class Resp:
        status_code = 200
        text = "{}"

    class RecordingSession:
        headers: dict = {}

        def post(self, url, json, timeout):
            sent.append({"url": url, "body": json})
            return Resp()

    class FakeHub(d.Hub):
        def __post_init__(self) -> None:
            self.session = RecordingSession()

    hub = FakeHub(url="http://stub", token="t")
    hub.post_tick("forward", 0)
    hub.post_tick("backfill", 12)

    assert [s["url"] for s in sent] == ["http://stub/v1/journal/ticks"] * 2
    # #40: the record also says WHICH distiller copy ticked — the release its
    # script was cut at and the blob id of the file that ran — so the hub can
    # report it and a reader need not ssh in and `cmp`.
    blob = d.script_blob_id()
    assert sent[0]["body"] == {
        "mode": "forward",
        "groups_pending": 0,
        "distiller_version": d.DISTILL_VERSION,
        "distiller_blob": blob,
    }
    assert sent[1]["body"] == {
        "mode": "backfill",
        "groups_pending": 12,
        "distiller_version": d.DISTILL_VERSION,
        "distiller_blob": blob,
    }


def test_dry_run_records_no_tick() -> None:
    """`--dry-run` writes nothing, and a dry run drains nothing.

    Pinned by reading `main()`: the call is guarded, and the guard is the whole
    reason a dry run cannot inflate the liveness signal.
    """
    src = (REPO / "scripts" / "cchv-distill.py").read_text(encoding="utf-8")
    guard = re.search(
        r"if not args\.dry_run:\s*\n\s*hub\.post_tick\(", src
    )
    assert guard, (
        "main() must guard the tick record with `if not args.dry_run:` — "
        "without it a --dry-run reports a tick that drained nothing"
    )


# ---------------------------------------------------------------------------
# identity (#40): an installed script is a COPY, and a copy that says nothing
# about itself let a Jul-24 build run for hours behind a green main.
# ---------------------------------------------------------------------------


def test_script_blob_id_is_the_git_blob_id() -> None:
    """`script_blob_id()` must equal `git hash-object` on the same file.

    That equality is the whole value: the logged id can be compared against
    `git rev-parse <rev>:scripts/cchv-distill.py` by anyone with the repo,
    without touching the host. sha256 would be just as exact and comparable to
    nothing git prints.
    """
    import subprocess

    path = REPO / "scripts" / "cchv-distill.py"
    expected = subprocess.run(
        ["git", "hash-object", str(path)], capture_output=True, text=True, check=True
    ).stdout.strip()
    got = d.script_blob_id()
    assert re.fullmatch(r"[0-9a-f]{40}", got), got
    assert got == expected


def test_distill_version_is_the_release_version() -> None:
    """`DISTILL_VERSION` is a `just sync-version` target, not a hand-bumped
    constant: it must equal package.json's version or the release recipe has
    drifted (this is the check that fails BEFORE the release does)."""
    import json as _json

    pkg = _json.loads((REPO / "package.json").read_text(encoding="utf-8"))
    assert d.DISTILL_VERSION == pkg["version"]
    # The marker sync-version.cjs anchors on. Reformat it and the sync silently
    # stops — so the marker itself is pinned here.
    src = (REPO / "scripts" / "cchv-distill.py").read_text(encoding="utf-8")
    assert re.search(
        r'^DISTILL_VERSION = "\d+\.\d+\.\d+"  # sync-version$', src, re.M
    ), "DISTILL_VERSION line must keep the exact `# sync-version` marker form"


def test_first_log_line_names_the_running_copy(monkeypatch, capsys) -> None:
    """Before any hub call, in any mode, the log names version + blob + mode.

    Pinned on --dry-run specifically: a run that dies on secrets resolution
    must still have said who it was, so the line comes before resolve_token().
    """
    # Make every later step fail fast and loudly if reached, so the assertion
    # is about ORDER: identity first, secrets second.
    def boom(*a, **k):
        raise RuntimeError("reached secrets before announcing identity")

    monkeypatch.setattr(d, "resolve_token", boom)
    monkeypatch.setattr(d, "resolve_aiproxy_key", boom)
    monkeypatch.setattr(sys, "argv", ["cchv-distill", "--dry-run", "--backend", "claude"])
    with pytest.raises(RuntimeError):
        d.main()
    lines = [l for l in capsys.readouterr().err.splitlines() if "[cchv-distill " in l]
    assert lines, "nothing logged"
    first = lines[0]
    blob12 = d.script_blob_id()[:12]
    assert f"cchv-distill {d.DISTILL_VERSION} blob={blob12} mode=dry-run" in first, first


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(pytest.main([__file__, "-q"]))


# --- open-question threads (#15) ------------------------------------------------


def test_prompt_offers_threads_as_data_with_their_numbers() -> None:
    threads = [{"thread_id": 12, "question": "why is\n  X slow", "last_seen": "2026-10-08"}]
    prompt = d.build_prompt("2026-10-10", "/w/p", "TRANSCRIPT", threads)
    assert "<open_threads>\nT12 (last seen 2026-10-08): why is X slow\n</open_threads>" in prompt
    assert '"continues"' in prompt and '"resolved"' in prompt
    # The transcript still comes first, the schema after the threads.
    assert prompt.index("TRANSCRIPT") < prompt.index("<open_threads>") < prompt.index('"status"')


def test_prompt_without_threads_says_so() -> None:
    prompt = d.build_prompt("2026-10-10", "/w/p", "TRANSCRIPT", [])
    assert "<open_threads>" not in prompt
    assert d.NO_THREADS_TEXT in prompt


def test_thread_links_turn_model_slips_into_new_threads() -> None:
    entry = {
        "open_questions": [
            "plain string",
            {"q": "continues 12", "continues": "T12"},
            {"q": "not offered", "continues": 99},
            {"q": "also resolved", "continues": 13},
            {"question": "alt key", "continues": 14},
        ],
        "resolved": [13, "15", True],
    }
    questions, links, resolved = d.thread_links(entry, {12, 13, 14, 15})
    assert questions == ["plain string", "continues 12", "not offered", "also resolved", "alt key"]
    assert links == [None, 12, None, None, 14]
    assert resolved == [15]  # 13 was both continued and resolved; True is no id


def test_validate_accepts_both_question_shapes_and_rejects_others() -> None:
    base = {"status": "entry", "headline": "h", "summary": "s", "topics": ["a", "b", "c"]}
    assert d.validate({**base, "open_questions": ["x", {"q": "y", "continues": None}]}) is None
    assert d.validate({**base, "open_questions": [{"continues": 3}]}) is not None
    assert d.validate({**base, "open_questions": [], "resolved": "12"}) is not None


def test_process_group_posts_links_for_offered_threads(monkeypatch) -> None:
    seen: dict = {}

    def fake_generate(llm, entry_date, project, transcript, threads):
        seen["threads"] = threads
        return {
            "status": "entry",
            "headline": "h",
            "summary": "s",
            "topics": ["a", "b", "c"],
            "open_questions": [{"q": "again", "continues": 12}, {"q": "new", "continues": None}],
            "resolved": [13],
        }, "m"

    monkeypatch.setattr(d, "generate", fake_generate)
    posted: list[dict] = []

    class ThreadHub(StubHub):
        def open_threads(self, project_path, before):
            assert (project_path, before) == ("/w/p", "2026-10-10")
            return [{"thread_id": 12, "question": "q12"}, {"thread_id": 13, "question": "q13"}]

        def post_entry(self, payload):
            posted.append(payload)

    class Model:
        model = "m"

    group = {"entry_date": "2026-10-10", "project_path": "/w/p", "session_ids": [11], "as_of": None}
    assert d.process_group(ThreadHub(), group, Model(), dry_run=False) is True
    assert [t["thread_id"] for t in seen["threads"]] == [12, 13]
    assert posted[0]["open_questions"] == ["again", "new"]
    assert posted[0]["open_question_threads"] == [12, None]
    assert posted[0]["resolved_threads"] == [13]


def test_open_threads_failure_never_costs_the_entry(monkeypatch) -> None:
    hub = d.Hub(url="http://127.0.0.1:9", token="t")

    def boom(*a, **k):
        raise d.requests.HTTPError("404 Not Found")

    monkeypatch.setattr(hub, "get", boom)
    assert hub.open_threads("/w/p", "2026-10-10") == []


# --- protected routing (openspec/changes/distill-protected-routing) ------------
#
# Markers and terms below are made up: the real ones live in the routing file
# outside the repo, so a client name never enters this public fork.

ENTRY_JSON = '{"status": "entry", "headline": "h", "summary": "s", "topics": ["a", "b", "c"]}'


def _routing(tmp_path, body: str):
    f = tmp_path / "distill-routing.toml"
    f.write_text(body)
    return d.load_routing(f)


ROUTING_TOML = """
[protected]
model = "claude-haiku-5-5"
path_markers = ["/_clientx/", "/Clientx-Ops"]
content_terms = ["clientx.example", "BuildThing"]
"""


def test_routing_file_is_read(tmp_path) -> None:
    r = _routing(tmp_path, ROUTING_TOML)
    assert not r.protect_all
    assert r.model == "claude-haiku-5-5"


def test_path_marker_protects_on_any_host_case_insensitively(tmp_path) -> None:
    r = _routing(tmp_path, ROUTING_TOML)
    assert r.is_protected("/Users/ac/_sync/dev/_clientx/repo", "hello")
    assert r.is_protected("/home/agent/_CLIENTX/repo", "hello")
    assert r.is_protected("/Users/ac/x/clientx-ops/tool", "hello")
    assert not r.is_protected("/Users/ac/_sync/dev/other", "hello")


def test_content_term_protects_a_group_outside_the_markers(tmp_path) -> None:
    r = _routing(tmp_path, ROUTING_TOML)
    assert r.is_protected("/Users/ac/_sync/dev/other", "see https://CLIENTX.example/x")
    assert r.is_protected("/Users/ac/_sync/dev/other", "the buildthing job")


@pytest.mark.parametrize("body", [None, "not = [valid", "[protected]\npath_markers = []\n"])
def test_missing_or_invalid_routing_protects_everything(tmp_path, body) -> None:
    f = tmp_path / "distill-routing.toml"
    if body is not None:
        f.write_text(body)
    r = d.load_routing(f)
    assert r.protect_all and r.reason
    assert r.model == d.FALLBACK_PROTECTED_MODEL
    assert r.is_protected("/anything", "anything")


def _llm(routing, backend="aiproxy", model="gpt-6.1-sol"):
    return d.LLM(backend=backend, model=model, effort="low", routing=routing)


def test_protected_group_goes_to_claude_with_the_exact_id(tmp_path, monkeypatch) -> None:
    calls = []
    monkeypatch.setattr(d, "_aiproxy_generate", lambda *a: pytest.fail("GPT was called"))
    monkeypatch.setattr(
        d, "_claude_generate",
        lambda model, prompt, direct=False: calls.append((model, direct)) or (ENTRY_JSON, {"claude-haiku-5-5"}),
    )
    llm = _llm(_routing(tmp_path, ROUTING_TOML))
    entry, served = d.generate(llm, "2026-10-20", "/Users/ac/_sync/dev/_clientx/r", "hi", [])
    assert calls == [("claude-haiku-5-5", True)]
    assert entry["status"] == "entry" and served == "claude-haiku-5-5"


def test_general_group_goes_to_the_general_backend(tmp_path, monkeypatch) -> None:
    monkeypatch.setattr(d, "_claude_generate", lambda *a, **k: pytest.fail("claude was called"))
    monkeypatch.setattr(d, "_aiproxy_generate", lambda llm, prompt: (ENTRY_JSON, "gpt-6.1-sol"))
    llm = _llm(_routing(tmp_path, ROUTING_TOML))
    _, served = d.generate(llm, "2026-10-20", "/Users/ac/_sync/dev/other", "hi", [])
    assert served == "gpt-6.1-sol"


def test_threads_count_toward_the_content_match(tmp_path, monkeypatch) -> None:
    monkeypatch.setattr(d, "_aiproxy_generate", lambda *a: pytest.fail("GPT was called"))
    monkeypatch.setattr(d, "_claude_generate", lambda m, p, direct=False: (ENTRY_JSON, {m}))
    llm = _llm(_routing(tmp_path, ROUTING_TOML))
    threads = [{"thread_id": 1, "question": "ask BuildThing about the mirror"}]
    _, served = d.generate(llm, "2026-10-20", "/w/other", "hi", threads)
    assert served == "claude-haiku-5-5"


@pytest.mark.parametrize("usage", [set(), {"claude-haiku-5-5", "gpt-6.1-sol"}, {"claude-fable-5-dd-x"}])
def test_guard_refuses_a_protected_result_from_any_other_model(tmp_path, monkeypatch, usage) -> None:
    monkeypatch.setattr(d, "_claude_generate", lambda m, p, direct=False: (ENTRY_JSON, usage))
    llm = _llm(_routing(tmp_path, ROUTING_TOML))
    with pytest.raises(d.Refused):
        d.generate(llm, "2026-10-20", "/x/_clientx/r", "hi", [])


def test_a_refused_group_is_not_posted(tmp_path, monkeypatch) -> None:
    monkeypatch.setattr(d, "_claude_generate", lambda m, p, direct=False: (ENTRY_JSON, {"other"}))
    posted: list[dict] = []

    class PostingHub(StubHub):
        def open_threads(self, project_path, before):
            return []

        def post_entry(self, payload):
            posted.append(payload)

    group = {"entry_date": "2026-10-20", "project_path": "/x/_clientx/r", "session_ids": [11], "as_of": None}
    with pytest.raises(d.Refused):
        d.process_group(PostingHub(), group, _llm(_routing(tmp_path, ROUTING_TOML)), dry_run=False)
    assert posted == []


def test_stored_model_is_the_serving_model(tmp_path, monkeypatch) -> None:
    monkeypatch.setattr(d, "_aiproxy_generate", lambda llm, prompt: (ENTRY_JSON, "gpt-6.1-sol-2026"))
    posted: list[dict] = []

    class PostingHub(StubHub):
        def open_threads(self, project_path, before):
            return []

        def post_entry(self, payload):
            posted.append(payload)

    group = {"entry_date": "2026-10-20", "project_path": "/w/other", "session_ids": [11], "as_of": None}
    assert d.process_group(PostingHub(), group, _llm(_routing(tmp_path, ROUTING_TOML)), dry_run=False)
    assert posted[0]["model"] == "gpt-6.1-sol-2026"


def test_direct_claude_call_strips_proxy_env_and_reports_model_usage(monkeypatch) -> None:
    monkeypatch.setenv("ANTHROPIC_BASE_URL", "https://proxy.invalid")
    monkeypatch.setenv("ANTHROPIC_AUTH_TOKEN", "x")
    seen = {}

    def fake_run(cmd, **kw):
        seen["cmd"], seen["env"] = cmd, kw.get("env")
        out = d.json.dumps({"result": "ok", "modelUsage": {"claude-haiku-5-5": {}}})
        return d.subprocess.CompletedProcess(cmd, 0, stdout=out, stderr="")

    monkeypatch.setattr(d.subprocess, "run", fake_run)
    result, served = d._claude_generate("claude-haiku-5-5", "p", direct=True)
    assert (result, served) == ("ok", {"claude-haiku-5-5"})
    assert seen["cmd"][:2] == ["claude", "-p"]
    assert seen["cmd"][seen["cmd"].index("--model") + 1] == "claude-haiku-5-5"
    # B-1: a persisted print-mode session would re-archive the protected prompt.
    assert "--no-session-persistence" in seen["cmd"]
    assert "ANTHROPIC_BASE_URL" not in seen["env"] and "ANTHROPIC_AUTH_TOKEN" not in seen["env"]


# --- whole-session routing (openspec/changes/distill-route-full-session) -------
#
# Made-up client marker and term, as above. "_clientx" is the folder name; the
# lab twin "_clientxtra" only starts with it and must stay general (C2).

ROUTING_FULL_TOML = """
[protected]
model = "claude-haiku-5-5"
path_markers = ["/_clientx", "/Clientx-Ops"]
content_terms = ["clientx.example", "BuildThing"]
"""

LAB = "/Users/ac/_sync/dev/lab"


def _msg(*items, role="user", sidechain=False, cwd=..., content=None):
    m = {"role": role, "is_sidechain": sidechain,
         "content": content if content is not None else list(items)}
    if cwd is not ...:
        m["cwd"] = cwd
    return m


def _text(t):
    return {"type": "text", "text": t}


class RoutingHub:
    """Per-session messages; records every window it was asked for."""

    def __init__(self, sessions: dict, fail: set | None = None) -> None:
        self.sessions, self.fail, self.windows = sessions, fail or set(), []

    def session_messages(self, session_id, window=None):
        self.windows.append((session_id, window))
        if session_id in self.fail:
            raise d.requests.ConnectionError("hub read failed")
        return self.sessions[session_id]


def _route(tmp_path, sessions, project=LAB, fail=None):
    r = _routing(tmp_path, ROUTING_FULL_TOML)
    hub = RoutingHub(sessions, fail)
    return d.route_group(hub, r, project, list(sessions)), hub


# C2: the segment boundary, one fixture per delimiter, each with a lab twin.
@pytest.mark.parametrize("after", [" next", "`", "'", '"', ": x", "\nnext", ""])
def test_folder_name_as_a_segment_protects_and_its_longer_twin_does_not(tmp_path, after) -> None:
    hit, _ = _route(tmp_path, {1: [_msg(_text(f"look in _clientx{after}"), cwd=None)]})
    assert hit == "text_marker"
    twin, _ = _route(tmp_path, {1: [_msg(_text(f"look in _clientxtra{after}"), cwd=None)]})
    assert twin is None


# C3: an absent cwd key protects; a null cwd is no signal.
def test_a_message_without_the_cwd_key_protects(tmp_path) -> None:
    hit, _ = _route(tmp_path, {1: [_msg(_text("neutral"))]})
    assert hit == "cwd_unavailable"


def test_a_null_cwd_is_no_signal(tmp_path) -> None:
    hit, _ = _route(tmp_path, {1: [_msg(_text("neutral"), cwd=None)]})
    assert hit is None


def test_a_cwd_under_a_marker_protects(tmp_path) -> None:
    hit, _ = _route(tmp_path, {1: [_msg(_text("neutral"), cwd="/Users/ac/_sync/dev/_clientx/repo")]})
    assert hit == "cwd"
    lab, _ = _route(tmp_path, {1: [_msg(_text("neutral"), cwd="/Users/ac/_sync/dev/lab")]})
    assert lab is None


# C4: every outbound string of a general call passes the gate.
def test_the_gate_stops_a_marker_in_an_offered_thread_before_any_send(tmp_path, monkeypatch) -> None:
    sent: list[str] = []

    def record_post(url, json=None, **kw):
        sent.append(d.json.dumps(json))
        raise AssertionError("general backend reached")

    def record_run(cmd, input=None, **kw):
        sent.append(f"{cmd} {input}")
        out = d.json.dumps({"result": ENTRY_JSON, "modelUsage": {"claude-haiku-5-5": {}}})
        return d.subprocess.CompletedProcess(cmd, 0, stdout=out, stderr="")

    monkeypatch.setattr(d.requests, "post", record_post)
    monkeypatch.setattr(d.subprocess, "run", record_run)
    llm = _llm(_routing(tmp_path, ROUTING_FULL_TOML))
    threads = [{"thread_id": 1, "question": "revisit /Users/ac/_sync/dev/_clientx/repo layout"}]
    entry, served = d.generate(llm, "2026-10-20", LAB, "neutral day", threads)
    assert served == "claude-haiku-5-5"
    assert len(sent) == 1 and "claude-haiku-5-5" in sent[0]


def test_a_neutral_general_call_passes_the_gate(tmp_path, monkeypatch) -> None:
    monkeypatch.setattr(d, "_claude_generate", lambda *a, **k: pytest.fail("claude was called"))
    monkeypatch.setattr(d, "_aiproxy_generate", lambda llm, prompt: (ENTRY_JSON, "gpt-6.1-sol"))
    llm = _llm(_routing(tmp_path, ROUTING_FULL_TOML))
    _, served = d.generate(llm, "2026-10-20", LAB, "neutral day", [])
    assert served == "gpt-6.1-sol"


# M2, M5, M6: markers in text, tool results, sidechains, other days.
@pytest.mark.parametrize("message", [
    _msg(_text("please read /Users/ac/_sync/dev/_clientx/repo/README"), cwd=None),
    _msg({"type": "tool_result", "content": "x" * 900 + " /Users/ac/_sync/dev/_clientx/r/a.py"}, cwd=None),
    _msg(_text("Summary: earlier we edited /_clientx/r"), role="assistant", cwd=None),
    _msg({"type": "tool_use", "name": "Read", "input": {"file_path": "/w/_clientx/r/a.py"}}, sidechain=True, cwd=None),
])
def test_a_marker_anywhere_in_the_session_protects(tmp_path, message) -> None:
    hit, _ = _route(tmp_path, {1: [_msg(_text("neutral"), cwd=None), message]})
    assert hit in ("text_marker", "tool_input")


def test_the_view_reads_the_whole_session_not_the_day(tmp_path) -> None:
    _, hub = _route(tmp_path, {1: [_msg(_text("neutral"), cwd=None)]})
    assert hub.windows == [(1, None)]


# M8: path shapes without a verbatim marker.
@pytest.mark.parametrize("tool_input", [
    {"path": "/Users/ac/_sync/dev", "pattern": "_clientx/**/*.py"},
    {"command": "curl 'http://hub/v1/search?project=%2FUsers%2Fac%2F_clientx%2Frepo'"},
    {"file_path": "C:\\\\Users\\\\ac\\\\_clientx\\\\repo\\\\a.py"},
    {"file_path": "/w/" + "y" * 400 + "/_clientx/r.py"},
])
def test_tool_input_shapes_are_normalized_before_matching(tmp_path, tool_input) -> None:
    hit, _ = _route(tmp_path, {1: [_msg({"type": "tool_use", "name": "Grep", "input": tool_input}, cwd=None)]})
    assert hit == "tool_input"


def test_a_term_only_in_the_cut_middle_protects(tmp_path) -> None:
    long = "a" * 50_000 + " BuildThing " + "b" * 50_000
    hit, _ = _route(tmp_path, {1: [_msg(_text(long), cwd=None)]})
    assert hit == "text_term"


# M9, C7: fail closed, and the next group goes on.
def test_one_unreadable_session_of_two_protects(tmp_path) -> None:
    hit, _ = _route(tmp_path, {1: [_msg(_text("neutral"), cwd=None)], 2: []}, fail={2})
    assert hit == "unreadable"


def test_a_malformed_content_item_protects(tmp_path) -> None:
    hit, _ = _route(tmp_path, {1: [_msg(content=[42, "stray"], cwd=None)]})
    assert hit == "unreadable"


def test_every_capped_part_is_inside_the_view(tmp_path) -> None:
    content = [
        _text("t" * 40), {"type": "thinking", "thinking": "k" * 900},
        {"type": "tool_use", "name": "Bash", "input": {"command": "c" * 700}},
        {"type": "tool_result", "content": [{"type": "text", "text": "r" * 900}]},
        {"type": "tool_result", "content": "s" * 900},
    ]
    f = d.flatten(content)
    assert f.capped and all(any(c in full for full in f.full) for c in f.capped)
    assert d._content_text(content) == "\n".join(f.capped)
