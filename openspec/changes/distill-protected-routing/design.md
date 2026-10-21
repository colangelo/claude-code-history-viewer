## Decisions

- **Substring markers, not prefixes.** The same tree sits under `/Users/ac/...` on the Macs and
  `/home/agent/...` on guests. A marker such as `/client-x/` matches on every host. A false
  positive only moves a call to Claude, which is the safe direction.
- **Content terms are matched on the whole prompt.** That covers the transcript plus the
  open-thread text from earlier days, which is exactly what would be sent.
- **Fail closed.** If routing cannot be read, nothing is known to be safe for GPT, so every group
  is protected. The failure is logged once per run with the reason.
- **The guard checks what served, not what was asked.** A name such as `claude-*` can be served
  by another vendor behind a proxy, so the protected route requires `modelUsage` to equal the
  configured id exactly; an empty or missing `modelUsage` is also refused. Stripping
  `ANTHROPIC_BASE_URL` and `ANTHROPIC_AUTH_TOKEN` from the child env is the first line; the
  guard is the second.
- **The alert is the existing one.** A refused group is not stored, so it stays pending, and
  `/v1/healthz/journal` (Gatus `cchv-journal`) goes stale after its grace. No hub change is
  needed. The log line `REFUSED <day> <path>: served=<models>` is the evidence.
