## Context

`generate()` calls `routing.is_protected(project, prompt)` on the prompt built from
`build_transcript`, which has already cut tool inputs (300 chars), thinking (500) and each session
to a head and a tail. Messages from the hub carry no `cwd`; it exists only in `messages.raw`.

## Decisions

1. **Route on a routing view, built from the same hub reads, before any cut.** `build_transcript`
   returns `(transcript, view)`, where `view` holds: the set of `cwd` values, the full JSON text of
   every `tool_use` input, and the full flattened text of every message (no `_content_text` caps).
   Same messages, same day window, no extra hub calls.
2. **Markers match paths, terms match text.** A marker in the project path, in any `cwd`, or
   anywhere in a tool input's text protects. A term anywhere in the full text protects. The prompt
   check is kept as a fourth input, so the change only widens protection.
3. **Fail closed per group.** If a hub read for the group raises, the group is protected and the
   log says why; the run does not fall back to the prompt-only check.
4. **`cwd` from the hub, nullable.** `m.raw->>'cwd' AS cwd` in the messages query. Providers without
   a `cwd` return null. An older hub omits the field; the distiller treats that as "no cwd signal"
   and logs it once per run.
5. **Log only counts.** The routing log line names which input class matched (`path`, `cwd`,
   `tool_input`, `text`, `prompt`), never the marker, term or matched text.

## Alternatives considered

- Untruncated prompt: rejected, the prompt budget exists for the model call.
- Hub-side routing flag: rejected for now, the markers deliberately live outside this repo and off
  the hub; the distiller is the only reader of the routing file.

## Risks

- More groups protected (the point). Protected calls are slower; measured after release by the
  per-route counts the distiller already logs.
- A marker that is too short now matches inside tool inputs too; marker hygiene stays with the
  routing file's owner (#13 adds a floor).
