## Purpose

The locally built viewer binary: it exports a single session headless and serves the WebUI
over HTTP for browsing local AI-assistant history. It is built from source on demand
(for example by the `cchv-find` skill) and is not a shipped artifact.

## ADDED Requirements

### Requirement: Headless session export
The binary SHALL export one session given by id, unambiguous id prefix or absolute JSONL
path, as HTML or JSON, to a file or to stdout, then exit. It SHALL open no window and need
no display.

#### Scenario: Export by session id to a file
- **WHEN** the binary runs with `--export <session-id> --format html --output <file>`
- **THEN** it writes the rendered HTML to `<file>` and exits with status 0

#### Scenario: Export with no display available
- **WHEN** the binary runs with `--export <path.jsonl> --format json` in a session with no
  display server
- **THEN** it writes the JSON to stdout and exits with status 0

#### Scenario: Ambiguous prefix
- **WHEN** the id prefix matches more than one session
- **THEN** it exits non-zero with an error saying the id is ambiguous and how many sessions
  match, and writes no export

### Requirement: WebUI server
The binary SHALL serve the WebUI and its `/api/*` routes when started with `--serve`,
honouring `--port`, `--host`, `--token`/`--no-auth` and read-only mode exactly as today.

#### Scenario: Serve with token auth
- **WHEN** the binary runs with `--serve --port 3727 --token T`
- **THEN** `GET /` returns the WebUI and `POST /api/scan_projects` without the token is
  rejected with 401

### Requirement: Every frontend command is an HTTP route
Every command the WebUI frontend calls SHALL be served by an `/api/*` route. No call SHALL
depend on a desktop IPC channel.

#### Scenario: No method-not-allowed on load
- **WHEN** the WebUI loads and the user browses a project and a session
- **THEN** no `/api/*` request answers 404 or 405

### Requirement: Builds without a webview stack
The binary SHALL build and run on Linux and macOS without GTK, WebKit or any other
webview or desktop-GUI system library installed.

#### Scenario: Clean Linux build
- **WHEN** the binary is built on an Ubuntu image with no `libgtk-3-dev` or
  `libwebkit2gtk-4.1-dev`
- **THEN** the build succeeds and its test suite passes

#### Scenario: Dependency graph has no webview crates
- **WHEN** the dependency graph of the binary is listed for all targets
- **THEN** it contains no `tauri`, `wry`, `tao`, `webkit2gtk` or `gtk` crate
