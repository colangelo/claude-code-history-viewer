## Purpose

Read back the open questions journal entries record, grouped into threads per project, so
that unresolved work resurfaces, without claiming more about a thread's fate than the
archive can know.

## ADDED Requirements

### Requirement: Questions are grouped into threads per project
The hub SHALL group a project's open questions within a requested window into threads, merging
questions that restate the same thread across days, and SHALL NOT merge questions from
different projects.

#### Scenario: Restated thread merges
- **WHEN** a project's entries on three different days each contain a rewording of the same
  open question
- **THEN** the report lists one thread for them with an occurrence count of 3 and the three
  entry ids

#### Scenario: Different projects stay apart
- **WHEN** two projects each contain the same open-question text
- **THEN** each project's report lists its own thread

### Requirement: Thread state claims only what the data supports
Each thread SHALL carry exactly one state: `recurring` (restated on two or more days),
`quiet` (not restated within the project's most recent N active days) or `new` (seen once,
within them). The report MUST NOT label any thread resolved.

#### Scenario: Thread goes quiet
- **WHEN** a thread was last restated before the project's three most recent active days
- **THEN** its state is `quiet`

#### Scenario: Calendar gaps do not age threads
- **WHEN** a project has no entries for two weeks and then one new entry
- **THEN** threads restated in the entry before the gap are not `quiet` on that basis alone

### Requirement: Open-questions report endpoint
The hub SHALL serve the threads for one project and window, newest last-seen first. Each
thread SHALL include its most recent wording, first and last seen dates, occurrence count,
state and entry ids. Without a project it SHALL return per-project counts by state only.

#### Scenario: Project report
- **WHEN** an authorized client requests the report for a project and a 30-day window
- **THEN** the response lists that project's threads in the window with all thread fields

#### Scenario: Unauthorized request
- **WHEN** a client without read authorization requests the report
- **THEN** the request is rejected as other journal reads are
