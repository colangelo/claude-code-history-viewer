## ADDED Requirements

### Requirement: Entries record open-question thread links and resolutions
The journal write endpoint SHALL accept, with an entry, `open_question_threads` (one optional
thread id per open question, same order) and `resolved_threads`. The hub SHALL assign a new
thread id to every open question without one. A re-distilled entry SHALL replace its links and
resolutions with the rest of the row.

#### Scenario: New and continued questions
- **WHEN** an entry posts two open questions with thread links `[null, 7]` and 7 belongs to an
  earlier entry of the same project
- **THEN** the first question gets a new thread id and the second keeps 7

### Requirement: Invalid thread links are rejected
The hub SHALL reject with `400` and no write: an id that does not belong to an open question of
another entry of the same project, an id both continued and resolved in one entry, and a links
array whose length differs from `open_questions`.

#### Scenario: Foreign or unknown id
- **WHEN** an entry links a question to an id that belongs only to another project, or to none
- **THEN** the request is rejected with `400` and nothing is written

### Requirement: The distiller links and resolves threads
Before writing an entry, the distiller SHALL fetch the project's unresolved threads mentioned
in the 30 days before the entry date (at most 30), list them in the prompt, and ask for a
thread link per open question and the list of threads the day resolved. It SHALL drop ids it
did not offer and ids both continued and resolved, so a model slip yields a new thread and not
a rejected entry. An entry without links (older distiller, plain-string questions) SHALL
remain valid.

#### Scenario: Model returns an id it was not given
- **WHEN** the model links an open question to an id that was not in the prompt
- **THEN** the distiller posts that question as a new thread
