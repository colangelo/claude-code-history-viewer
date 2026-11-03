## REMOVED Requirements

### Requirement: Authentication and pagination

**Reason**: its identity-header path let any process with ac's tailnet identity, or any local process on m4m, read every row with no credential (security P2, 2026-11-03). Replaced by "Read authentication and pagination", which keeps the bearer, pagination and parameter rules and drops the header path.

**Migration**: callers that relied on the `Tailscale-User-Login` header log in with a token: the webapp through `POST /v1/login`, any other caller with a bearer token (design D2 lists every caller).

## ADDED Requirements

### Requirement: Read authentication and pagination

All read endpoints SHALL require a credential: a valid bearer token, or the session cookie that `POST /v1/login` issues in exchange for one. A `Tailscale-User-Login` header SHALL grant nothing, whatever its value, and the hub SHALL refuse to start with a `trust_tailscale_identity` key in its config. The login cookie SHALL be `HttpOnly`, `Secure`, `SameSite=Strict`, with a lifetime, and failed credential checks SHALL be rate-limited globally without ever refusing a valid credential. The config SHALL refuse a token shorter than 32 characters, and a client-reader hash equal to the SHA256 of any plaintext token. `/v1/ingest` SHALL keep requiring a bearer token, since writes bind to a machine identity. All read endpoints SHALL support bounded pagination via limit and offset, returning a stable order so that paging does not drop or duplicate rows. Truncation MUST be detectable: the session messages endpoint SHALL report the session's total message count, and the projects and sessions lists their filtered totals, in an `X-Total-Count` response header, so a client that receives a default-limit page (50; max 200) can tell it is partial. Browse endpoints SHALL reject unknown query parameters with `400` instead of ignoring them: an unsupported filter that silently returns unfiltered rows reads exactly like a real answer.

#### Scenario: Unauthenticated read is rejected
- **WHEN** a client calls any read endpoint without a valid bearer token and without a live login cookie
- **THEN** the hub responds `401`

#### Scenario: An identity header grants nothing
- **WHEN** a read request carries a `Tailscale-User-Login` header with ac's login and no other credential, on any registered read route
- **THEN** the hub responds `401`

#### Scenario: The old trust key is refused
- **WHEN** the hub starts with `trust_tailscale_identity` in its config (even `[]`), or with `HUB_TRUST_TAILSCALE_IDENTITY` set
- **THEN** it exits with an error that names the removed key

#### Scenario: A client hash of a plaintext token is refused
- **WHEN** a `[[client_readers]]` hash equals the SHA256 of a `[[tokens]]` or `[[readers]]` token
- **THEN** the config is refused, so a restricted token can never gain client reads

#### Scenario: A short token is refused
- **WHEN** a configured plaintext token is shorter than 32 characters
- **THEN** the config is refused

#### Scenario: A browser reads after login
- **WHEN** a browser posts a valid read token to `/v1/login` and then calls a read route with the returned cookie
- **THEN** the request is served with that token's read scope

#### Scenario: Failed credentials are rate-limited, valid ones never
- **WHEN** failed credential checks across all routes exceed the global limit within its window
- **THEN** requests whose credential fails get `429` with `Retry-After`, while a request with a valid credential is still served

#### Scenario: Ingest ignores identity headers
- **WHEN** a request to `/v1/ingest` carries a `Tailscale-User-Login` header but no valid bearer token
- **THEN** the hub responds `401`

#### Scenario: Paging is stable
- **WHEN** a client pages through a large result set using limit and offset
- **THEN** each row appears in exactly one page and the overall order is consistent across pages

#### Scenario: List truncation is detectable
- **WHEN** a projects or sessions list request matches more rows than the (clamped) limit
- **THEN** `X-Total-Count` carries the filtered total, so the caller can tell a capped page from the whole result set

#### Scenario: Unknown query params are rejected
- **WHEN** a browse endpoint is queried with a parameter it does not support (e.g. a typo of a real filter)
- **THEN** the hub responds `400` naming the unknown parameter, instead of returning plausible-looking unfiltered rows

### Requirement: Browser sessions
`POST /v1/login` SHALL trade a reader token (a lab reader that is not `stats_only`, or a client reader) for an `HttpOnly` `__Host-` session cookie that reads with that token's scope and never writes. Sessions SHALL survive a hub restart, SHALL take their principal and scope from the configuration current at each request, and SHALL end after 7 days unused or 30 days in all. The hub SHALL store only a hash of the cookie value and a keyed fingerprint of the token.

#### Scenario: Only a reader logs in
- **WHEN** a machine token, an ingest-only token or a `stats_only` reader posts to `/v1/login`
- **THEN** no cookie is set and the answer is `401` (machine, ingest) or `403` (`stats_only`)

#### Scenario: A restart keeps the session
- **WHEN** the hub restarts with the same configuration and key file
- **THEN** a cookie minted before the restart still reads

#### Scenario: A removed token ends its sessions
- **WHEN** the token that opened a session is removed from the configuration and the hub restarts
- **THEN** that session's cookie gets `401`

#### Scenario: Re-login, cap, revoke and expiry
- **WHEN** a login carries an existing cookie, or a principal opens a sixth session, or the session is deleted (`DELETE /v1/login`, `hub sessions revoke`), or it has been unused 7 days or open 30 days
- **THEN** the replaced, oldest, deleted or expired cookie gets `401`, and the others keep reading
