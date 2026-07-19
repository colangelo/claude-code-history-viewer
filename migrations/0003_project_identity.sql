-- project-identity :: git-fingerprint identity for projects (openspec change
-- project-identity)
--
-- Additive migration: nullable fact columns on `projects` + one new table.
-- Every existing row stays valid with NULL fingerprints and every
-- pre-identity query behaves unchanged; rollback is "deploy the previous
-- binary" (the columns/table are inert).
--
-- A project's *identity* is the equivalence class of rows sharing
-- `identity_key`, derived hub-side from the daemon-captured git facts
-- (archive_protocol::identity::derive_identity_key):
--   g:<root_commit>|<remote>  both facts present
--   g:<root_commit>           no usable remote
--   r:<remote>                shallow clone (root unknowable)
--   NULL                      not a git repo → path identity, as before
-- Root commit + remote are BOTH in the key so a fork (same root, different
-- remote) is a distinct identity, never auto-grouped.

ALTER TABLE projects
    -- Full 40-hex root commit (lexicographically smallest of the repo's roots).
    ADD COLUMN git_root_commit TEXT,
    -- Normalized origin URL (`host/path`, credentials stripped by the daemon,
    -- re-normalized defensively by the hub).
    ADD COLUMN git_remote_url  TEXT,
    -- Derived identity key (see forms above). Recomputed on every upsert from
    -- the effective (COALESCEd) facts, so absent facts in one batch never
    -- flap a project out of its group.
    ADD COLUMN identity_key    TEXT,
    -- Linked `git worktree` member: grouped under the identity but labeled,
    -- and excludable via include_worktrees=false.
    ADD COLUMN git_worktree    BOOLEAN NOT NULL DEFAULT false,
    -- For worktrees: the main checkout's path (labeling only, not identity).
    ADD COLUMN git_main_path   TEXT;

CREATE INDEX projects_identity_key_idx
    ON projects (identity_key)
    WHERE identity_key IS NOT NULL;

-- Manual alias layer: attaches a (typically moved-away, unfingerprint-able)
-- path to an identity. View-level only — no ingested row is ever rewritten;
-- deleting an alias restores the split exactly. A path belongs to at most one
-- identity. `created_by` records the authenticated principal (machine token's
-- machine_id or trusted Tailscale login) for audit.
CREATE TABLE project_identity_aliases (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    project_path  TEXT        NOT NULL UNIQUE,
    identity_key  TEXT        NOT NULL,
    created_by    TEXT        NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX project_identity_aliases_key_idx
    ON project_identity_aliases (identity_key);
