## ADDED Requirements

### Requirement: Threads carry their origin
Each thread SHALL carry an `origin`: `general` when every entry that mentions it has route
`general`, otherwise `protected`. A mention by an entry with route `protected` or with no route SHALL
make the thread protected origin, whichever entry holds the latest wording, as SHALL a mention by an
entry sharing a session with a protected entry of the project. Resolutions SHALL NOT change origin.

#### Scenario: Only general mentions
- **WHEN** thread 7 is mentioned by two entries, both with route `general`
- **THEN** its origin is `general`

#### Scenario: One protected mention
- **WHEN** thread 7 is mentioned first by a protected entry and later by a general one
- **THEN** its origin is `protected`

#### Scenario: A session shared with a protected day
- **WHEN** thread 7 is mentioned only by a general entry whose session also has a protected entry in the project
- **THEN** its origin is `protected`

#### Scenario: A mention with no route
- **WHEN** thread 7 is mentioned by an entry that has no route
- **THEN** its origin is `protected`

## MODIFIED Requirements

### Requirement: Open-questions report endpoint
The hub SHALL serve `GET /v1/journal/open-questions` for one project and window, newest last
mention first, each thread with id, latest wording, first and last mention dates, number of
days mentioned, entry dates, state, resolved date and origin; it SHALL filter by state when asked.
With `origin=general` it SHALL return only general-origin threads and echo `"origin": "general"` in
the response; any other `origin` value SHALL be rejected with `400`. Without a project it SHALL
return per-project counts by state only.

#### Scenario: Project report
- **WHEN** an authorized client requests a project's report for a 30-day window
- **THEN** the response lists that project's threads in the window with all thread fields

#### Scenario: Unauthorized request
- **WHEN** a client without read authorization requests the report
- **THEN** the request is rejected as other journal reads are

#### Scenario: General-origin filter
- **WHEN** a client requests a project's report with `origin=general`
- **THEN** the response lists only general-origin threads and carries `"origin": "general"`

#### Scenario: Unknown origin value
- **WHEN** a client requests the report with `origin=protected`
- **THEN** the request is rejected with `400`
