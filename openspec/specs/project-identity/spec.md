# project-identity Specification

## Purpose

Git-fingerprint-derived project identity for the archive: moved, cloned, and
worktree checkouts of one repository group into a single identity across
machines via the root commit plus normalized remote. A reversible, view-level
alias layer joins dead (no-longer-fingerprintable) paths to an identity, and
read endpoints accept identity-scoped filters that expand server-side to the
member paths. Shipped cchv-v0.10.0.

## Requirements

### Requirement: Git fingerprint semantics

A project's git fingerprint SHALL consist of up to three facts captured from
the project directory's git repository: the **root commit** (full 40-hex hash;
when `git rev-list --max-parents=0 HEAD` yields multiple roots, the
lexicographically smallest is chosen for determinism), the **normalized remote
URL** of `origin`, and the **worktree status** (whether the directory is a
linked `git worktree` of another checkout). Remote normalization MUST strip
credentials/userinfo, convert scp-like syntax (`git@host:path`) to
`host/path` form, lowercase the host, and strip a trailing `.git` and
trailing slashes, yielding `<host>/<path>`. A shallow repository (where the
true root commit is unreachable) MUST omit the root commit and fingerprint by
remote alone. A directory that is not a git repository has no fingerprint.

#### Scenario: Same repo, two homes

- **WHEN** a repo is moved from `~/dev/foo` to `~/projects/foo` and both paths' project rows carry fingerprints
- **THEN** both fingerprints have the same root commit and normalized remote, and resolve to the same identity

#### Scenario: Fork is not the same project

- **WHEN** two directories contain repos sharing a root commit but with different normalized remotes (a fork alongside its upstream)
- **THEN** their fingerprints resolve to different identities and are never auto-grouped

#### Scenario: Credentialed remote is sanitized

- **WHEN** `origin` is `https://user:token@github.com/acme/foo.git`
- **THEN** the normalized remote is `github.com/acme/foo` and no credential material is captured, transmitted, or stored

#### Scenario: Shallow clone

- **WHEN** a project directory is a shallow clone with `origin` set
- **THEN** the fingerprint carries only the normalized remote and no (graft-falsified) root commit

### Requirement: Identity key derivation

The hub SHALL derive an opaque, stable `identity_key` from fingerprint facts:
`g:<root_commit>|<normalized_remote>` when both are present,
`g:<root_commit>` when only the root commit is present, and
`r:<normalized_remote>` when only the remote is present (shallow). Projects
with no fingerprint have a NULL `identity_key` and fall back to path identity.
The derivation function SHALL live in the shared `protocol` crate and the hub
MUST re-normalize the remote defensively rather than trusting daemon input.
The literal prefix `identity:` is reserved in project filters and MUST NOT be
interpreted as a path.

#### Scenario: Deterministic across machines

- **WHEN** the same repo is cloned on two machines at different paths
- **THEN** both projects derive the identical `identity_key` and group together

#### Scenario: No fingerprint means path identity

- **WHEN** a project directory is not a git repository
- **THEN** its `identity_key` is NULL and every existing path-keyed behavior is unchanged

### Requirement: Worktrees are related, not flattened

A linked git worktree SHALL join its main repository's identity (it shares
root commit and remote) but MUST be flagged as a worktree member wherever
identity members are exposed, and identity-scoped queries MUST support
excluding worktree members via an `include_worktrees=false` parameter
(default: included).

#### Scenario: Worktree grouped and labeled

- **WHEN** `~/dev/foo.feature-x` is a linked worktree of `~/dev/foo`
- **THEN** both group under one identity and the worktree member is flagged as such in `/v1/projects` and `/v1/identities`

#### Scenario: Worktree excluded on request

- **WHEN** a client queries sessions with `project=identity:<key>&include_worktrees=false`
- **THEN** sessions belonging to worktree member paths are omitted

### Requirement: Manual alias layer

The hub SHALL maintain a `project_identity_aliases` table mapping a project
path (typically a moved-away path that can no longer be fingerprinted) to an
`identity_key`. Aliases are view-level only: creating or deleting an alias
MUST NOT modify any ingested project, session, message, or journal row. A
path SHALL belong to at most one identity (unique on path). Alias writes
require an authenticated principal (machine bearer token or trusted Tailscale
identity) and record it for audit.

#### Scenario: Dead path joined to identity

- **WHEN** an alias maps dead path `~/old/foo` to identity K
- **THEN** identity-scoped queries for K include history and journal entries recorded under `~/old/foo`

#### Scenario: Alias removal restores the split

- **WHEN** that alias is deleted
- **THEN** `~/old/foo` reverts to a standalone path-identified project with all its rows intact

### Requirement: Identity-scoped project filtering

Read endpoints that accept a project filter SHALL additionally accept
`identity:<key>`, which the hub expands server-side to the set of member
project paths: all projects whose `identity_key` matches, plus all aliased
paths. Plain (non-prefixed) project filters keep their existing byte-exact
semantics.

#### Scenario: Identity filter unions member paths

- **WHEN** `/v1/sessions?project=identity:<key>` is queried and the identity has two live paths and one aliased dead path
- **THEN** sessions from all three paths are returned

#### Scenario: Plain filters unchanged

- **WHEN** `/v1/sessions?project=/Users/ac/dev/foo` is queried
- **THEN** results are identical to pre-identity behavior

### Requirement: Identity and alias management endpoints

The hub SHALL expose: `GET /v1/identities` returning each identity
(`identity_key`, members with path/providers/machines/worktree-flag/
last-activity, alias paths, and link suggestions), `POST
/v1/identities/aliases` creating an alias (`{project_path, identity_key}`,
returns the alias id), and `DELETE /v1/identities/aliases/{id}`. Suggestions
SHALL include: fingerprint-less project paths whose basename matches an
identity member's basename, and identities sharing a root commit but
differing in key (fork or remote-drift, labeled "related"). Suggestions are
advisory only — the hub MUST NOT auto-create aliases.

#### Scenario: Orphan suggested by basename

- **WHEN** a fingerprint-less project `~/old/place/foo` exists and identity K has a member `~/dev/foo`
- **THEN** `GET /v1/identities` lists `~/old/place/foo` as a suggestion on K, and no alias exists until explicitly created

#### Scenario: Read endpoints stay read-only for identity data

- **WHEN** a client only ever calls GET endpoints
- **THEN** no identity grouping decision is persisted anywhere
