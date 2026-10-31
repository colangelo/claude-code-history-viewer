#!/usr/bin/env -S uv run --quiet --with requests python
"""Release probe for the journal protection tag (openspec/changes/journal-read-protection-tag, task 4.1).

Routes each journal-read shape from the hub-produced fixture through a distiller's
`route_group` against the LIVE routing file (and its floor record) and prints the
class per shape, never text. Exit 0 only when every protected shape routes
`text_term` and every general twin routes general.

    scripts/journal-tag-probe.py [path/to/cchv-distill]   # default: the installed copy
"""
import importlib.machinery
import importlib.util
import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
target = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else pathlib.Path.home() / ".local/bin/cchv-distill")
loader = importlib.machinery.SourceFileLoader("cchv_distill_probe", str(target))
spec = importlib.util.spec_from_loader(loader.name, loader)
assert spec is not None
d = importlib.util.module_from_spec(spec)
sys.modules[loader.name] = d
loader.exec_module(d)

shapes = json.loads((HERE / "fixtures/journal-read-shapes.json").read_text())
routing = d.load_routing(d.ROUTING_FILE, floor=d.ROUTING_FLOOR_FILE)
if routing.protect_all:
    print(f"routing: every group protected ({routing.reason}); the probe proves nothing")
    sys.exit(1)
if d.PROTECTION_TAG != shapes["tag"]:
    print("the distiller's tag differs from the hub's")
    sys.exit(1)


class Hub:
    def __init__(self, block):
        self.block = block

    def session_messages(self, session_id, window=None):
        return [{"role": "user", "is_sidechain": False, "cwd": None, "content": [self.block]}]


def block(rows):
    return {"type": "tool_result", "content": json.dumps(rows, indent=2)}


cases = []
for general in (False, True):
    pick = [r for r in shapes["browse"] if (r["route"] == "general") == general]
    hits = [r for r in shapes["search"]["journal"] if (r["route"] == "general") == general]
    threads = [t for t in shapes["report"]["threads"] if (t["origin"] == "general") == general]
    want = None if general else "text_term"
    cases += [(f"browse {'general' if general else 'protected'}", block(pick), want),
              (f"search {'general' if general else 'protected'}", block({"journal": hits}), want),
              (f"report {'general' if general else 'protected'}", block({"threads": threads}), want)]

ok = True
for name, b, want in cases:
    got = d.route_group(Hub(b), routing, "/probe/lab-project", [1])
    good = got == want
    ok &= good
    print(f"{'ok ' if good else 'BAD'} {name}: {got or 'general'}")
sys.exit(0 if ok else 1)
