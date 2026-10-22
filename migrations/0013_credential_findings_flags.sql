-- Reference and placeholder hits per finding row (ac/security policy 2026-10-22,
-- openspec/changes/assign-strong-reference-exemption). Of `hits`, how many values
-- held an unexpanded reference (`${VAR}`, `$(cmd)`, ...) and how many a placeholder
-- (`xxx`, `<token>`, ...), so the daily count can report them apart. Counts only,
-- like the rest of the table: never a value. Rows recorded before this read 0.
ALTER TABLE credential_findings
    ADD COLUMN reference_hits   INTEGER NOT NULL DEFAULT 0 CHECK (reference_hits >= 0),
    ADD COLUMN placeholder_hits INTEGER NOT NULL DEFAULT 0 CHECK (placeholder_hits >= 0);
