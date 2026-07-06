# archive-search-api — delta

## MODIFIED Requirements

### Requirement: Authentication and pagination

All read endpoints SHALL require authentication — either a valid bearer
token, or, when the hub is configured with a non-empty
`trust_tailscale_identity` allow-list, a `Tailscale-User-Login` request
header whose value matches an allow-listed identity (as injected by
Tailscale serve for tailnet clients). The identity path grants READ scope
only: `/v1/ingest` SHALL keep requiring a bearer token, since writes bind to
a machine identity. With the allow-list unset or empty, behavior is
unchanged (bearer only). All read endpoints SHALL support bounded pagination
via limit and offset, returning a stable order so that paging does not drop
or duplicate rows. Truncation MUST be detectable: the session messages
endpoint SHALL report the session's total message count in an
`X-Total-Count` response header, so a client that receives a default-limit
page (50; max 200) can tell it is partial.

#### Scenario: Unauthenticated read is rejected

- **WHEN** a client calls any read endpoint without a valid bearer token and without a trusted identity header
- **THEN** the hub responds `401`

#### Scenario: Trusted Tailscale identity is accepted for reads

- **WHEN** the hub is configured with `trust_tailscale_identity` containing a login, and a read request carries a `Tailscale-User-Login` header with that login and no bearer token
- **THEN** the request is served

#### Scenario: Untrusted identity is rejected

- **WHEN** a read request carries a `Tailscale-User-Login` header whose value is not in the allow-list (or the allow-list is empty)
- **THEN** the hub responds `401`

#### Scenario: Ingest ignores identity headers

- **WHEN** a request to `/v1/ingest` carries a trusted `Tailscale-User-Login` header but no valid bearer token
- **THEN** the hub responds `401`

#### Scenario: Paging is stable

- **WHEN** a client pages through a large result set using limit and offset
- **THEN** each row appears in exactly one page and the overall order is consistent across pages
