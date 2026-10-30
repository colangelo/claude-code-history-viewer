## ADDED Requirements

### Requirement: A stored password hash is not a credential

A value that is wholly a bcrypt, argon2, or sha-crypt (`$5$`, `$6$`) hash, matched anchored at both ends, SHALL NOT produce an `assign`, `assign-strong` or `pair` finding. The same hash with any characters appended SHALL be judged as before.

#### Scenario: A harness file's bcrypt hash
- **WHEN** a transcript holds `PASSWORD = "<a 60-character bcrypt hash>"`
- **THEN** no finding is recorded and the text is stored unchanged

#### Scenario: A tampered hash still fires
- **WHEN** the same hash has extra characters appended
- **THEN** the assign rules judge it as any other value

### Requirement: Code after a closed string literal is not a secret

A quoted assignment capture whose first character is `)`, `]`, `}` or `,` SHALL be treated as code, and record no finding, only when the rest of the capture also holds whitespace or another bracket character.

#### Scenario: A credential-helper parse line
- **WHEN** a transcript holds `line.startswith("password=")), None)` followed by more code
- **THEN** no assign finding is recorded

#### Scenario: A secret that starts with a closing character
- **WHEN** a quoted value starts with `)` and holds no whitespace and no other bracket
- **THEN** it is judged as any other value and fires
