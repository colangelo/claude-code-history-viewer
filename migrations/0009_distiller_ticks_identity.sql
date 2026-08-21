-- Distiller identity on tick records (change: build-identity-surfaces, #40).
--
-- The distiller runs as an installed COPY of scripts/cchv-distill.py, and a
-- copy that says nothing about itself let a Jul-24 build run for hours against
-- an Aug-21 `main` while every checkbox said the fix was live (2026-08-21).
-- From here on a tick carries which copy posted it: the release version its
-- script was cut at, and the git blob id of the file that actually ran
-- (`git hash-object scripts/cchv-distill.py`), so the log, this table and
-- `/v1/healthz/journal` can all be compared against `git rev-parse
-- <rev>:scripts/cchv-distill.py` without an ssh.
--
-- Both nullable, no default, no backfill: a distiller that predates these
-- fields posts neither, and the resulting NULL is itself the reading "an old
-- distiller is ticking" — which is exactly the condition this exists to show.
-- Rows written before this migration have no identity to recover.
ALTER TABLE distiller_ticks
    ADD COLUMN IF NOT EXISTS distiller_version text,
    ADD COLUMN IF NOT EXISTS distiller_blob    text;
