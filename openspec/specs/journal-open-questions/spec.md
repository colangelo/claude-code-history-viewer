# journal-open-questions Specification

## Purpose
Read back the open questions journal entries record as threads per project, so unresolved
work resurfaces, with a state that says no more than the distilled record does.

## Requirements

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

### Requirement: Protected thread wording is tagged in the report
In `GET /v1/journal/open-questions`, the question text of every thread whose origin is not `general`
SHALL start with the protection tag `[cchv:protected-journal] `; general-origin threads SHALL be
returned untagged.

#### Scenario: Protected-origin thread
- **WHEN** the report lists a thread of protected origin
- **THEN** its question starts with the tag

#### Scenario: General-origin thread
- **WHEN** the report lists a thread of general origin
- **THEN** its question does not carry the tag
