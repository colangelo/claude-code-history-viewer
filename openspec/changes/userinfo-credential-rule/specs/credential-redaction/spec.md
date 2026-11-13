## ADDED Requirements

### Requirement: Credentials in URL userinfo

The detector SHALL record a `userinfo` finding for the secret in `scheme://userinfo@host`. The authority SHALL
end at `/`, `?`, `#`, a quote, an angle bracket, a brace, a backslash, whitespace or 512 bytes, and its last `@`
SHALL end the userinfo. The secret is everything after the first `:`, 8 or more characters. A finding SHALL carry
no value, fragment or digest.

#### Scenario: A credential store line is flagged
- **WHEN** a message holds `https://ac:<40 hex>@gitea.example.test`
- **THEN** one `userinfo` finding is recorded, even though no key name, prefix or bearer word is present

#### Scenario: A raw at-sign in the password is redacted whole
- **WHEN** a message holds `https://u:Ab12Cd34@Ef56Gh78@host`
- **THEN** the redacted text is `https://u:[REDACTED:userinfo]@host`

#### Scenario: A query or fragment ends the authority
- **WHEN** a message holds `https://docs.example:8443?contact=help@example.test/`
- **THEN** no `userinfo` finding is recorded

### Requirement: Token in the user position

A userinfo with no `:` SHALL be a secret only when it is a token: 40 or 64 hex characters, or 20 or more
characters of at least three classes with no `.`. The user part before a `:` MAY be empty.

#### Scenario: A token in the user position is flagged
- **WHEN** a message holds `https://:<40 hex>@host` or `https://<40 hex>@host`
- **THEN** one `userinfo` finding is recorded

### Requirement: Userinfo repeats

Exact repeats of a detected `userinfo` secret SHALL be replaced with it in the same text and, in redact mode, in
every string leaf of the same message's `raw`, `content` and `search_text`, and SHALL be counted once. Object keys
and other messages are not covered.

#### Scenario: A bare repeat of the token goes too
- **WHEN** a message holds the credential URL in one leaf and the bare token in another leaf and field
- **THEN** in redact mode no copy of the token remains in `raw`, `content` or `search_text`, and one finding per URL is recorded

### Requirement: Userinfo exemptions

A `userinfo` secret SHALL NOT fire when it is an existing `[REDACTED:` marker, a shell variable (starts with `$`),
a run of `*` that is the whole value, or a display mask: `..` or an ellipsis between two ends of 1 to 6
characters. A word such as `changeme` or `password`, a value starting `xxx`, or a value holding `*` among other
characters SHALL be a finding.

#### Scenario: Negative controls are untouched
- **WHEN** the text holds `https://user:${TOKEN}@host`, `https://user:********@host`, `ssh://git@host:org/repo.git`, `https://first.last@host`, a masked `ab12...cd34`, or a URL with a port and no userinfo
- **THEN** no `userinfo` finding is recorded

#### Scenario: Real-looking secrets are findings
- **WHEN** the text holds `https://user:changeme@host` or `https://user:R7k*Q2m9Z4v8@host`
- **THEN** one `userinfo` finding is recorded and, in redact mode, the secret is replaced

### Requirement: Userinfo redaction keeps the URL readable

In `userinfo` redact mode only the secret SHALL be replaced by `[REDACTED:userinfo]`; the scheme, user and host
SHALL stay, in `raw`, `content` and `search_text`. Without `userinfo` in `[redaction] redact` the text SHALL be
stored unchanged and the finding still recorded.

#### Scenario: A credential store line is redacted
- **WHEN** `userinfo` is in redact mode and a message holds `https://ac:<40 hex>@gitea.example.test`
- **THEN** all three fields hold `https://ac:[REDACTED:userinfo]@gitea.example.test` and the findings row is marked redacted

#### Scenario: The rule is flag-only unless selected
- **WHEN** `userinfo` is not in `[redaction] redact`
- **THEN** the text is stored unchanged and a `userinfo` finding is still recorded
