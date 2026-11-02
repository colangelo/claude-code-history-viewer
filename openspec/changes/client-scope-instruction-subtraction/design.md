## Context

The injected copies, measured on pg1 2026-11-02 (hashes and lengths only):

- The `instructions` attachment holds `attachment.files[]`, each `{path, content, type}`. For the
  global file, `content` is the file with trailing whitespace removed: 7137 bytes against 7138 on
  disk, and the sha256 of the trimmed file equals the stored one.
- Each file's `content` occurs verbatim in a text block of an `api-request-blob` row of the same
  session, after `Contents of <path> (<description>):` and a blank line, so at a line start.
- Six versions of the global file were injected since 2026-09-20. All six hash to versions committed
  in ac's dotfiles repo (`private_dot_config/claude/CLAUDE.md`), a source independent of the archive.

## Decisions

### D1. Registered by hash, located at word starts

An entry is `{sha256, bytes, head_sha256}`: the sha256 and length of the block, and the sha256 of its
first 64 bytes. In a string, a candidate start is offset 0 or any offset just after ASCII whitespace
(a word start).

Each file version is registered in two forms, both printed by `instruction-entry`: as injected, and
as `search_text` holds it. `search_text` (history-core `search_text`) joins the message's strings and
collapses every whitespace run to one space, so there the block follows a space, not a newline. The
first census (2026-11-02, infra-hlp) found the injected form alone lowered no `text_marker` session:
`search_text` still marked all 11 Exa sessions, and its collapsed form of the global file is present
verbatim in all 11.

At a candidate, the classifier hashes the next 64 bytes; only when that matches a registered head does it
hash the full length of each block with that head and compare. A match removes the span and the scan
resumes at its end.

The hub never holds the block's text, only hashes. The head keeps the cost to one 64-byte hash per
word of a string that already holds a marker; strings without a marker are not searched at all.

Rejected: skipping `attachment` or `api-request-blob` rows (security d457e4cf: an attachment can hold
a client file); skipping by attachment path (a path says nothing about the bytes).

### D2. What is left is matched piece by piece

The pieces between removed spans are normalized and matched separately. A marker inside a piece
marks as before, including text appended right after a block or a client file injected next to it.
A marker split across a block boundary (half inside the block, half outside) does not mark; the block
is ac's own text, so that needs a client path built from its last bytes.

### D3. Where it applies

Every string the message extractor reads: `content` (text, tool input, tool result, other block
types), `search_text`, `raw` and its keys. Not the session row, the project row or `cwd`, which are
paths and never instruction text. A registered block is ac's text wherever it appears, so a tool
result that prints the file verbatim is subtracted too; the Read tool's numbered lines never match.

### D4. Loading and acceptance

`[protected].instruction_blocks` is optional. A present key must be an array of tables with a 64-hex
`sha256`, a 64-hex `head_sha256` and an integer `bytes` ≥ 256; anything else makes the policy unknown
("instruction_blocks invalid"). 256 bytes keeps a marker-sized block, which would blind the classifier
to that marker, out of the registry.

The accepted set (`client_scope_policy.marker_hash`) gains one row per block, `instruction:<sha256>`.
`compare` reads the two kinds apart: a removed marker or an added block is a narrowing (policy unknown
until `accept-routing`); an added marker or a removed block is a widening (accepted on load). Running
`accept-routing` right after the files change, before the hub's refresh, leaves no unknown window.

### D5. Census

`hub client-scope census --routing FILE --floor FILE [--days N] [--ids]` loads the candidate files
through the same loader and re-classifies the sessions stamped client at the hub's current generation
whose last message is within N days (default 31). It prints counts by (stamped class → candidate
class) and, with `--ids`, the session ids whose candidate class is `none`. It opens its pool
read-only and stamps nothing.

## Risks

- A block registered by mistake hides a marker inside it in every session. The owner's floor record
  pins the routing bytes, the hub requires `accept-routing`, and the census shows the effect first.
- A Claude Code change in how the file is injected (another trim, an escape) stops the match: those
  sessions stay client, the safe direction.

## Security review (2026-11-02, ac/security af08b2f, U1-U7)

- **U1:** each entry names its source (`git_repo`, a 40-hex `git_commit`, `git_path`, `form`).
  `accept-routing` re-derives every block from it and refuses on any mismatch; the audit row lists
  each hash with form, commit and path. The accepted row pins sha256, length and head hash together,
  and two descriptors for one sha256 make the policy unknown.
- **U2:** an empty accepted set bootstraps markers only; a first set with blocks is unknown until
  `accept-routing`.
- **U3:** the early guard is "some needle occurs in the normalized string, boundaries ignored",
  which is conservative: a piece normalizes to a substring of the whole (U5 keeps escapes off the
  edges, and starts follow whitespace). A differential test checks it against subtraction run on
  every string.
- **U4:** every occurrence at every word start counts, overlaps included, and the union is removed:
  no dependence on entry order.
- **U5:** an occurrence whose edges, over the longest needle's length, hold a `%`, end with a
  needle's proper prefix or start with its proper suffix is not removed. `instruction-entry`
  refuses such a file against the live markers.
- **U6:** hashes are exactly 64 ASCII hex digits.
- **U7:** `cwd` is matched without subtraction.

