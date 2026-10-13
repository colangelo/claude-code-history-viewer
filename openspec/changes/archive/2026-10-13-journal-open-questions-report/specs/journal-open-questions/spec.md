## Purpose

Read back the open questions journal entries record as threads per project, so unresolved
work resurfaces, with a state that says no more than the distilled record does.

## ADDED Requirements

### Requirement: Threads are identified by thread ids, per project
A thread SHALL be the set of open questions that carry the same thread id. Thread ids SHALL be
scoped to one `project_path`: a question MUST NOT join a thread of another project.

#### Scenario: Restated thread
- **WHEN** a project's entries on three different days each carry an open question with thread id 7
- **THEN** the report lists one thread 7 mentioned on 3 days, with the three entry dates and
  the most recent wording

#### Scenario: Existing questions start as their own threads
- **WHEN** the migration runs over entries written before this change
- **THEN** every existing open question gets its own new thread id

### Requirement: Thread state follows the record
Each thread SHALL carry exactly one state: `resolved` when an entry of the project lists it as
resolved and is dated after its last mention; otherwise `quiet` when its last mention is
before the project's N most recent active days (N = 3, active day = a day with an `entry`
row); otherwise `open`.

#### Scenario: Resolved
- **WHEN** thread 7 was last mentioned on day 1 and the day-2 entry lists 7 as resolved
- **THEN** its state is `resolved` with resolved date day 2

#### Scenario: Mentioned again after a resolution
- **WHEN** a day-3 entry carries an open question continuing thread 7
- **THEN** its state is no longer `resolved`

#### Scenario: Calendar gaps do not age threads
- **WHEN** a project has no entries for two weeks and then one new entry
- **THEN** threads mentioned in the entry before the gap are not `quiet` on that basis alone

### Requirement: Open-questions report endpoint
The hub SHALL serve `GET /v1/journal/open-questions` for one project and window, newest last
mention first, each thread with id, latest wording, first and last mention dates, number of
days mentioned, entry dates, state and resolved date; it SHALL filter by state when asked.
Without a project it SHALL return per-project counts by state only.

#### Scenario: Project report
- **WHEN** an authorized client requests a project's report for a 30-day window
- **THEN** the response lists that project's threads in the window with all thread fields

#### Scenario: Unauthorized request
- **WHEN** a client without read authorization requests the report
- **THEN** the request is rejected as other journal reads are
