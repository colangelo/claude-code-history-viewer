# archive-analytics — delta for analytics-ux-costs

## MODIFIED Requirements

### Requirement: Per-project statistics endpoint

The hub SHALL expose `GET /v1/stats/projects/{identity_key}` returning a
`ProjectStatsSummary` for one project identity: session and message counts,
token totals, average and total session duration, most active hour, most used
tools, and a per-model usage distribution (`model_distribution`) with the same
shape and dedup semantics as the global endpoint's. Statistics MUST be folded
across every path and machine belonging to that identity, so a repository that
was moved, cloned, or checked out as a worktree reports as one project.

#### Scenario: Statistics fold across machines and paths

- **WHEN** a project identity spans several `project_path` values on more than one machine
- **THEN** the response aggregates sessions and messages from all of them under the single identity

#### Scenario: Unknown identity returns not found

- **WHEN** an authenticated client requests statistics for an identity key that does not exist
- **THEN** the hub responds `404`

#### Scenario: Per-model distribution at project scope

- **WHEN** an authenticated client requests statistics for a known identity whose messages carry model names
- **THEN** the response includes `model_distribution` entries with per-model message and token counts (input, output, cache-creation, cache-read) covering only that identity's messages, within any requested date window

## ADDED Requirements

### Requirement: Cost is a first-class analytics metric

The Analytics view SHALL present estimated cost as the first headline metric at
both the whole-archive and single-project scopes, derived by pricing the
scope's `model_distribution` with the client-side pricing table, and SHALL
display the pricing-coverage percentage alongside it. Where a figure's grain
has no per-model split (top projects, provider distribution, daily trend), the
view MUST NOT display an invented cost for it.

#### Scenario: Project scope shows cost

- **WHEN** a user selects a single project identity in the Analytics scope control
- **THEN** an estimated cost figure for that project is displayed, priced from the project's `model_distribution`

#### Scenario: Unpriceable grains carry no cost

- **WHEN** the view renders rows whose source data has no per-model split
- **THEN** those rows show token totals without a cost figure

### Requirement: Analytics range control offers presets and custom windows

The Analytics view SHALL offer relative date-window presets of 7, 14, 30, 60,
90, 180, and 365 days plus all time, and SHALL accept a custom "last N days"
positive integer. The selected window MUST apply to both scopes.

#### Scenario: Preset selection narrows the window

- **WHEN** a user picks the 7-day preset
- **THEN** the statistics request covers the last 7 days

#### Scenario: Custom day count

- **WHEN** a user enters 45 as a custom day count
- **THEN** the statistics request covers the last 45 days

### Requirement: Activity heatmap precedes tool usage

In both the whole-archive and single-project analytics layouts, the activity
heatmap SHALL appear in the first chart row, before tool-usage charts.

#### Scenario: Heatmap in the first chart row

- **WHEN** the Analytics view renders either scope with activity data present
- **THEN** the activity heatmap is in the first row of chart sections and the most-used-tools chart appears after it
