## ADDED Requirements

### Requirement: Registered instruction text does not mark a session

The routing file MAY register instruction blocks under `[protected].instruction_blocks`, each by the
sha256 and byte length of the exact text and the sha256 of its first 64 bytes. Before matching path
markers in a message string, the hub SHALL remove every span that starts at the string's start or
right after a newline and whose bytes hash to a registered block. It SHALL match the markers in each
remaining piece. It SHALL NOT skip any row, attachment or block by its type.

A registered block shorter than 256 bytes, or an entry that is not a table with a 64-hex `sha256`, a
64-hex `head_sha256` and an integer `bytes`, SHALL make the policy unknown. A routing generation that
registers a block not yet accepted SHALL be a narrowing: the policy stays unknown until
`client-scope accept-routing`.

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

#### Scenario: Registering a block needs acceptance
- **WHEN** the routing file gains an instruction block the hub has not accepted
- **THEN** the policy is unknown until `client-scope accept-routing`, and every session counts as
  client meanwhile
