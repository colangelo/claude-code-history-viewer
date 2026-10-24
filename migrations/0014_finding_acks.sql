-- Acknowledged credential findings (Gitea #76, openspec/changes/findings-ack).
-- A finding that has been triaged stops paging without rewriting the finding:
-- `credential_findings` stays the record of what was detected, and this table
-- records who looked at it and what they decided. Like the findings table it
-- holds no detected value; `hub findings ack` refuses a note the detector flags.
CREATE TABLE IF NOT EXISTS finding_acks (
    finding_id     BIGINT      PRIMARY KEY REFERENCES credential_findings (id) ON DELETE CASCADE,
    classification TEXT        NOT NULL CHECK (classification IN ('false_positive', 'handled')),
    acked_by       TEXT        NOT NULL CHECK (acked_by <> ''),
    note           TEXT        NOT NULL DEFAULT '',
    acked_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
