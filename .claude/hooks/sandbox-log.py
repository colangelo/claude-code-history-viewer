#!/usr/bin/env python3
"""PostToolUse / PostToolUseFailure hook for the sandbox pilot: log blocked Bash calls.

Claude Code keeps no log of sandbox denials, so this records one line per
blocked call: UTC time, the command's first word, and which marker matched.
Never the command's arguments or output, which can carry values.
Log: ~/.config/cchv/sandbox-log/<session id>.log
"""
import json, os, sys, time

MARKERS = {
    "Operation not permitted": "fs-denied",
    "not on the allow list": "net-denied",
    "CONNECT tunnel failed": "net-denied",
    "Connection blocked by network allowlist": "net-denied",
    "<sandbox_violations>": "sandbox",
    "could not translate host name": "dns-denied",
    "nodename nor servname": "dns-denied",
    "no such host": "dns-denied",
}

def main():
    try:
        event = json.load(sys.stdin)
    except ValueError:
        return
    if event.get("tool_name") != "Bash":
        return
    # A call that exits non-zero arrives as PostToolUseFailure, with the output
    # in "error" and no tool_response.
    text = json.dumps(event.get("tool_response")) + str(event.get("error") or "")
    hits = sorted({tag for marker, tag in MARKERS.items() if marker in text})
    if not hits:
        return
    command = (event.get("tool_input") or {}).get("command", "")
    first = os.path.basename(command.split()[0]) if command.split() else "?"
    session = "".join(c for c in event.get("session_id", "unknown") if c.isalnum() or c == "-")
    d = os.path.expanduser("~/.config/cchv/sandbox-log")
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, f"{session}.log"), "a") as f:
        f.write(f"{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}\t{first}\t{','.join(hits)}\n")

if __name__ == "__main__":
    main()
