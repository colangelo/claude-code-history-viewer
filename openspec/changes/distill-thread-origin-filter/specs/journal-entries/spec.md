## ADDED Requirements

### Requirement: Entries record the route they were generated on

The write endpoint SHALL store an entry's optional `route` (`general` or `protected`), reject any
other value with `400`, and store none when it is absent. The distiller SHALL post the route it
actually generated on. The migration SHALL mark existing entries from a GPT model `general` and
leave the rest without a route.

#### Scenario: Protected entry
- **WHEN** the distiller writes an entry for a protected group
- **THEN** the stored row has route `protected`

#### Scenario: Gate-turned group
- **WHEN** a group found general by the routing view is turned protected at the send-time check
- **THEN** the stored row has route `protected`

#### Scenario: Older distiller
- **WHEN** an entry is posted without a route
- **THEN** it is stored with no route, and its open questions count as protected origin

#### Scenario: Invalid route
- **WHEN** an entry is posted with route `other`
- **THEN** the request is rejected with `400` and nothing is written

## MODIFIED Requirements

### Requirement: The distiller links and resolves threads
Before writing an entry, the distiller SHALL fetch the project's unresolved threads mentioned
in the 30 days before the entry date (at most 30), list them in the prompt, and ask for a
thread link per open question and the list of threads the day resolved. On the general route it
SHALL fetch only general-origin threads, and SHALL offer none when the hub does not confirm that
filter; on the protected route it SHALL fetch every thread. It SHALL drop ids it did not offer and
ids both continued and resolved, so a model slip yields a new thread and not a rejected entry. An
entry without links (older distiller, plain-string questions) SHALL remain valid.

#### Scenario: Model returns an id it was not given
- **WHEN** the model links an open question to an id that was not in the prompt
- **THEN** the distiller posts that question as a new thread

#### Scenario: A general day after a protected day
- **WHEN** a project has a protected entry with open questions on day 1 and its day-2 group routes general
- **THEN** the day-2 prompt offers none of the day-1 threads

#### Scenario: Two general days
- **WHEN** a project has a general entry with open questions on day 1 and its day-2 group routes general
- **THEN** the day-2 prompt offers the day-1 threads

#### Scenario: A hub without the filter
- **WHEN** the hub's report does not confirm the `origin=general` filter
- **THEN** a general call is offered no threads and the run logs a warning
