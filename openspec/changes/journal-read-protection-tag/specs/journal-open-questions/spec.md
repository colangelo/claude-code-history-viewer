## ADDED Requirements

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
