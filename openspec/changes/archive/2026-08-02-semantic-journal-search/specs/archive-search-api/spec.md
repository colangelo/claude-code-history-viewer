# archive-search-api Delta

## ADDED Requirements

### Requirement: Journal search mode parameter

`GET /v1/search` SHALL accept a `mode` parameter
(`keyword` | `semantic` | `hybrid`, default `keyword`) governing the
journal block's retrieval per the semantic-search capability. The
message-results leg is unaffected by `mode` in this phase. An unknown
`mode` value is a 400; the default keeps every existing request
byte-compatible.

#### Scenario: Hybrid journal block

- **WHEN** `/v1/search?q=…&scope=journal&mode=hybrid` is queried on a hub with embeddings available
- **THEN** the journal block is the reciprocal-rank fusion of keyword and semantic rankings, message results are absent (scope), and the shape is unchanged

#### Scenario: Old clients unaffected

- **WHEN** any pre-change request (no `mode`) is replayed
- **THEN** the response is byte-identical to pre-change behavior
