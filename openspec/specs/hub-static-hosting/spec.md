# hub-static-hosting Specification

## Purpose
TBD - created by archiving change static-archive-web. Update Purpose after archive.
## Requirements
### Requirement: Optional static directory serving

The hub SHALL accept an optional static directory setting — `static_dir` in
the `HUB_CONFIG` TOML file or the `HUB_STATIC_DIR` environment variable,
following the same source-precedence as existing hub settings (the hub has no
CLI flags) — and, when set, serve that directory's files at the HTTP root.

#### Scenario: Static files served when configured

- **WHEN** the hub runs with a static directory configured and a request hits
  `/` or an asset path that exists in the directory
- **THEN** the file is returned with an appropriate `Content-Type` (`/`
  resolves to `index.html`)

#### Scenario: Behavior unchanged when not configured

- **WHEN** the hub runs without a static directory setting
- **THEN** non-`/v1` paths return 404 exactly as before

### Requirement: API precedence over static content

Static serving MUST NOT shadow the API: every `/v1/*` route SHALL behave
identically whether or not a static directory is configured.

#### Scenario: API route wins over same-named file

- **WHEN** a static directory is configured (even one containing a `v1/`
  subdirectory) and a request hits `/v1/healthz`
- **THEN** the JSON API handler responds, not the static file server

#### Scenario: Unauthenticated static access

- **WHEN** a static directory is configured and a request without an
  `Authorization` header hits `/`
- **THEN** the static content is served (bearer auth applies to `/v1/*`
  endpoints per the existing archive-search-api requirements, not to static
  assets)

