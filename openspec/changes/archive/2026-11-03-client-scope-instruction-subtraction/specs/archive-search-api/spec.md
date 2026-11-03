## ADDED Requirements

### Requirement: Registered instruction text does not mark a session

The routing file MAY register instruction blocks under `[protected].instruction_blocks`, each by the
sha256 and byte length of the exact text and the sha256 of its first 64 bytes. Before matching path
markers in a message string, the hub SHALL remove every span that starts at the string's start or
right after ASCII whitespace and whose bytes hash to a registered block. It SHALL match the markers in each
remaining piece. It SHALL NOT skip any row, attachment or block by its type.

A registered block shorter than 256 bytes, an entry that is not a table with a 64-hex `sha256`, a
64-hex `head_sha256`, an integer `bytes`, a `git_repo`, a 40-hex `git_commit`, a `git_path` and a
`form` of `injected` or `search`, or two entries giving one sha256 different descriptors, SHALL make
the policy unknown. A routing generation that registers a block not yet accepted SHALL be a
narrowing, also when nothing is accepted yet: the policy stays unknown until
`client-scope accept-routing`, which SHALL re-derive every block from its git source and refuse on
any mismatch. An occurrence whose edges could carry part of a marker or a percent escape SHALL NOT be
removed. `cwd` SHALL be matched without subtraction.

#### Scenario: A registered block is subtracted
- **WHEN** a message's text holds a registered block that contains a path marker, and nothing else
  marks
- **THEN** the session is classified `none`

#### Scenario: One changed byte is still scanned
- **WHEN** the text differs from a registered block by one byte
- **THEN** the marker in it marks the session

#### Scenario: Client text appended after the block still marks
- **WHEN** a registered block is followed in the same string by text holding a path marker
- **THEN** the session is client

#### Scenario: An attachment holding a client file still raises
- **WHEN** an `instructions` attachment lists a registered block and a second file whose text holds a
  path marker
- **THEN** the session is client

#### Scenario: The search_text form is subtracted
- **WHEN** a block's whitespace-collapsed form is registered and `search_text` holds it after a space
- **THEN** it is removed there too, and a marker after it still marks

#### Scenario: Registering a block needs acceptance
- **WHEN** the routing file gains an instruction block the hub has not accepted
- **THEN** the policy is unknown until `client-scope accept-routing`, and every session counts as
  client meanwhile
