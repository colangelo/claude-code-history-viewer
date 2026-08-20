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


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(pytest.main([__file__, "-q"]))
