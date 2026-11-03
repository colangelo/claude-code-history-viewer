-- Browser sessions minted by `POST /v1/login`, kept across hub restarts
-- (openspec hub-read-needs-credential, D3; security eff460e1).
--
-- `id_hash` is sha256 of the cookie value: the value itself is never stored.
-- `token_fp` is HMAC-SHA256 under a key only the hub holds (a file next to its
-- config, never in this database) over sha256 of the token that logged in. The
-- session's principal and scope are taken from the CURRENT config at every
-- lookup through this fingerprint, so a token removed from the config ends its
-- sessions at the next start. `principal` is kept for revocation by name only.
-- No route reads this table.
CREATE TABLE browser_sessions (
    id_hash     BYTEA       PRIMARY KEY,
    principal   TEXT        NOT NULL,
    token_fp    TEXT        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen   TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at  TIMESTAMPTZ NOT NULL
);
CREATE INDEX browser_sessions_principal_idx ON browser_sessions (principal, created_at);
