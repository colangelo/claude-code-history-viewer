## ADDED Requirements

### Requirement: Client sessions are readable only with client read

The hub SHALL classify every session as client or lab. The classification SHALL use the path
markers of the routing file named in `[client_scope] routing_file`, with the distiller's
normalisation and segment rule. The markers SHALL be matched against:

- the project path;
- each message's `cwd`;
- tool input values;
- message text.

Once a session is client, a later message SHALL NOT make it lab again. A read principal SHALL hold
lab read unless its token entry sets `client_read = true`. The trusted Tailscale identity SHALL
hold lab read only.

With `[client_scope] enforce = true`, a lab-read caller SHALL NOT receive any part of a client
session from any read endpoint. That covers:

- message text and session summaries;
- project paths;
- journal entries built from the session;
- per-project or per-session stats and identities.

A session the hub has not classified under the current routing file SHALL count as client. If the
routing file is missing, invalid, below its floor record or has a different hash from that record,
every session SHALL count as client.

#### Scenario: Lab reader cannot open a client session
- **WHEN** a lab-read caller requests `/v1/sessions/{id}/messages` for a client session
- **THEN** the hub answers 404, exactly as for an id that does not exist

#### Scenario: Lab reader search leaves client sessions out
- **WHEN** a lab-read caller searches for a string that occurs only in a client session
- **THEN** the result is empty and `X-Total-Count` is 0

#### Scenario: Client reader sees both
- **WHEN** a caller whose token sets `client_read = true` makes the same requests
- **THEN** the hub answers as it does today

#### Scenario: Tailscale identity is lab read
- **WHEN** a request carries only a trusted Tailscale identity
- **THEN** client sessions are absent from every response

#### Scenario: Unknown routing file fails closed
- **WHEN** the routing file is missing or no longer matches its floor record and enforce is on
- **THEN** every session counts as client for lab readers and `/v1/healthz/client-scope` answers 503

#### Scenario: Every read route is covered
- **WHEN** the route-walk test calls every registered GET route as a lab reader with a client
  canary seeded
- **THEN** the canary appears in no response
