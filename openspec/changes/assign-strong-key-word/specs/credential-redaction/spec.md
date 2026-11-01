## ADDED Requirements

### Requirement: Three-class values under a word-final secret key are strong

The detector SHALL classify an `assign` hit of 32 to 63 characters with exactly three character classes as
`assign-strong` when its key name, compared case-insensitively, ends in `PASSWORD`, `PASSWD`, `TOKEN`, `SECRET`
or `KEY`. A key that contains one of these words only before its end SHALL keep the existing rule. A value
that holds a reference SHALL NOT become `assign-strong`.

#### Scenario: An alphanumeric password is redacted
- **WHEN** `assign-strong` is in redact mode and `NATS_PASSWORD` holds 32 random letters and digits
- **THEN** the value is replaced by `[REDACTED:assign-strong]` and the finding's rule is `assign-strong`

#### Scenario: A JSON key in camel case counts
- **WHEN** a JSON object's key `apiKey` holds 40 random letters and digits
- **THEN** the value is replaced by `[REDACTED:assign-strong]`

#### Scenario: The key word is not at the end
- **WHEN** `password_reset_url` holds a 40-character value with three classes
- **THEN** the text is unchanged and the finding stays `assign`

#### Scenario: A hex digest and code are untouched
- **WHEN** `NATS_PASSWORD` holds a 40-character hex digest, or `ACCESS_TOKEN` is assigned a call or a dotted path such as `loadAccessTokenFromKeychain()` or `config.secrets.signingPrivateKey`
- **THEN** the text is unchanged
