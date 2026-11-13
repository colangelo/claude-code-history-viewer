## ADDED Requirements

### Requirement: Credentials in URL userinfo

The detector SHALL record a `userinfo` finding for the password part of a URL of the form
`scheme://user:secret@host`, where the secret is 8 or more characters. The secret and the user part SHALL stop at
whitespace, quotes, angle brackets, braces and `/`, and the character after `@` SHALL open a host. A finding
SHALL carry no value, fragment or digest.

#### Scenario: A credential store line is flagged
- **WHEN** a message holds `https://ac:<40 hex>@gitea.example.test`
- **THEN** one `userinfo` finding is recorded, even though no key name, prefix or bearer word is present

#### Scenario: A database URL inside JSON is flagged
- **WHEN** a JSON string holds `postgres://app:<24 characters>@db.internal:5432/x`
- **THEN** one `userinfo` finding is recorded

### Requirement: Userinfo exemptions

A `userinfo` secret SHALL NOT fire when it is a placeholder or template (`${VAR}`, `{{ x }}`, `<password>`, `***…`,
an existing `[REDACTED:…]` marker), a same-length mask, a display mask that elides its middle (`..`, an
ellipsis, `*`), or one of the words `password`, `passwd`, `secret`, `changeme`, `yourpassword`, `mypassword`,
`redacted`, `example`.

#### Scenario: Negative controls are untouched
- **WHEN** the text holds `https://user:password@host`, `https://user:${TOKEN}@host`, `ssh://git@host:org/repo.git`, a masked `ab12...cd34`, or a URL with a port and no userinfo
- **THEN** no `userinfo` finding is recorded

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
