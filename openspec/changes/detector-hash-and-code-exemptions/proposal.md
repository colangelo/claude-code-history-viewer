## Why

Two classes of false positive paged the findings check on 2026-10-29, and each owner
triaged them by hand:

- **Stored password hashes.** A committed rehearsal harness file holds bcrypt hashes under a
  `password:` key. A hash is what a system stores instead of the credential; it is not
  the credential.
- **Code after a closed string literal.** A Python line such as
  `line.startswith("password=")), None)` puts a quote right after `password=`. The assign
  rule reads that quote as the start of a quoted value and captures code up to the next
  quote: `)), None)…`.

security approved both conditions on 2026-10-30 (5211704d), with the conditions below.

## What Changes

- **Password hashes are not credentials.** A value that is wholly one of these, anchored at
  both ends, records no `assign`, `assign-strong` or `pair` finding:
  - bcrypt: `$2[abxy]?$NN$` plus 53 characters of `[./A-Za-z0-9]`;
  - argon2: `$argon2(id|i|d)$v=…$m=…,t=…,p=…$salt$hash`;
  - sha-crypt: `$5$` and `$6$`, with optional `rounds=N$`, a salt of 1–16 characters, and
    43 or 86 characters of hash.

  A hash with anything appended still fires. scrypt and PBKDF2 are left out until one is
  seen.
- **Code after a closed string literal is not a secret.** A *quoted* `assign` capture whose
  first character is `)`, `]`, `}` or `,` is code. This applies only when the rest also
  holds a second code signal: whitespace, or another of `( ) [ ] { }`. A leading closing
  character alone still fires, because a random secret can start with one.

## Impact

- `crates/hub` only (`redact.rs`, `pair.rs`). No migration, no `hub.toml` change.
- Fewer false positives on the findings check; no real-shaped secret stops firing (tests
  hold a must-fire control for each condition).
