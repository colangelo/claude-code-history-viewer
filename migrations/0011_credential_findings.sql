-- Credential-shape findings recorded at ingest (Gitea #34,
-- openspec/changes/ingest-credential-redaction). One row per message, stored
-- field and rule, per detector version: a re-sent batch adds nothing, and a new
-- detector version may record again.
--
-- NOTHING here holds a detected value, a fragment of one, or a digest of one:
-- the rule, the hit count and the secret-NAMED keys (`NATS_PASSWORD`, never its
-- value) are what a rotation needs, and all a leak of this table can expose.
CREATE TABLE credential_findings (
    id               BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    message_ref      BIGINT      NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    field            TEXT        NOT NULL CHECK (field IN ('raw', 'content', 'search_text')),
    rule             TEXT        NOT NULL,
    hits             INTEGER     NOT NULL CHECK (hits > 0),
    key_names        TEXT[]      NOT NULL DEFAULT '{}',
    -- Whether the rule was in redact mode, i.e. the stored field holds the
    -- marker instead of the value.
    redacted         BOOLEAN     NOT NULL,
    detector_version INTEGER     NOT NULL,
    detected_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (message_ref, field, rule, detector_version)
);

-- The findings summary reads "what was found since T".
CREATE INDEX credential_findings_detected_at ON credential_findings (detected_at);
