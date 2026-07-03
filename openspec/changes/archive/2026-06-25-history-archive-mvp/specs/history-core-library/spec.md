## ADDED Requirements

### Requirement: Tauri-free extraction library

The system SHALL provide a `history-core` Rust library crate that owns provider detection and the parse/normalize pipeline for all supported agents. The crate MUST NOT depend on `tauri`, `tauri-*`, or any GUI/webview dependency, so it can be linked into headless binaries.

#### Scenario: Crate builds without tauri in its dependency tree

- **WHEN** `cargo tree -p history-core` is inspected
- **THEN** no `tauri` or `tauri-*` crate appears in `history-core`'s dependency graph

#### Scenario: Crate builds standalone

- **WHEN** `cargo build -p history-core` is run from the workspace root with no other crate selected
- **THEN** the build succeeds without requiring the desktop app or any GUI feature

### Requirement: Stable headless extraction API

The library SHALL expose a stable, documented API that returns the normalized models (`ClaudeMessage`, `ClaudeSession`, `ClaudeProject`): provider detection, project scanning, session listing for a project, and message loading for a session. Each supported provider MUST be reachable through this unified API.

#### Scenario: Enumerate providers and load a session end to end

- **WHEN** a caller invokes detect → scan_projects → load_sessions → load_messages against a fixture history directory for any supported provider
- **THEN** the API returns normalized `ClaudeProject`, `ClaudeSession`, and `ClaudeMessage` values with provider, identifiers, timestamps, and content populated

#### Scenario: Unsupported or empty directory yields empty results, not errors

- **WHEN** scanning a directory that contains no recognizable agent history
- **THEN** the API returns empty collections and does not panic or return a hard error

### Requirement: Deterministic normalization preserved from current behavior

The normalized output produced by `history-core` for a given input file SHALL be equivalent to what the desktop application produced before the refactor, so the extraction is a behavior-neutral move. Normalization MUST be deterministic for identical input.

#### Scenario: Golden output matches pre-refactor parsing

- **WHEN** a per-provider fixture file is parsed via `history-core`
- **THEN** the resulting normalized records match the committed golden snapshot for that fixture

#### Scenario: Repeated parsing is stable

- **WHEN** the same fixture is parsed twice in the same process
- **THEN** the two normalized outputs are identical (same ordering, same field values)

### Requirement: Desktop application consumes the library without behavior change

The desktop application (`src-tauri`) SHALL depend on `history-core` for all extraction and retain its `#[tauri::command]` wrappers as thin adapters. The desktop app's existing validation MUST remain green.

#### Scenario: Desktop validation stays green after extraction

- **WHEN** the desktop workspace member is validated (`cargo test --test-threads=1`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`)
- **THEN** all checks pass with no new failures introduced by the refactor
