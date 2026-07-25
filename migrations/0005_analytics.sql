-- analytics :: provider message id + extracted tool invocations
--
-- Additive migration for hub-side analytics (openspec change `hub-analytics`,
-- Deliverable 1 of the web-only pivot). One new nullable column on `messages`
-- and one new side table; no existing column is rewritten and no data is
-- destroyed. Rollback is "deploy the previous binary": the column sits NULL and
-- the table sits inert, since nothing else reads either.
--
--
-- WHY `messages.message_id` MUST BE A COLUMN
--
-- Token/cost rollups have to count each provider response's `usage` block
-- exactly once: one assistant response occupies several stored rows carrying an
-- identical `usage`, so a plain SUM over `messages` OVER-REPORTS. The dedup key
-- is (session_id, message_id) — falling back to uuid, then to the row id — which
-- cannot be expressed while the id is buried in JSONB.
--
-- The value is extracted hub-side from `raw->>'messageId'`. Note the path:
-- `raw` is the NORMALIZED, FLAT `ClaudeMessage` (sync-daemon convert.rs sets
-- `raw = to_value(m)`), not the original JSONL line — so the nested
-- `raw->'message'->>'id'` does NOT exist and would yield NULL for every row.
-- The `ClaudeMessage` field carries `#[serde(rename = "messageId")]`, hence the
-- camelCase key.
--
-- That field is deliberately DUAL-PURPOSE: `TryFrom<RawLogEntry>` computes
-- `msg.id.clone().or(log_entry.message_id)`, so the assistant `msg_…` id wins
-- and the file-history-snapshot `messageId` is the fallback. The retired desktop
-- implementation dedups on exactly this precedence, and it is the verification
-- oracle for these rollups — so the precedence must be REPRODUCED, not "cleaned
-- up" by splitting the two meanings apart. Snapshot rows carry no `usage`, so
-- they cannot perturb token totals.
--
-- Left NULL here and backfilled in batches (`hub backfill-analytics`).
--
-- The index is PARTIAL and created here rather than concurrently afterwards.
-- The original plan was "backfill, then CREATE INDEX CONCURRENTLY in its own
-- migration", to avoid per-row index maintenance during the bulk UPDATE. That
-- plan does not survive contact with the migrator: sqlx applies all pending
-- migrations at startup, so a later migration would run immediately after this
-- one and long before any backfill — the ordering it depends on cannot happen.
--
-- Creating it here is cheap anyway: measured on pg1, only 10.6% of rows carry a
-- messageId, and at creation time the column is entirely NULL, so the partial
-- index starts EMPTY and the build is a single scan indexing nothing. The
-- backfill then maintains ~280k entries incrementally, which is the cost the
-- original plan was avoiding — accepted, because it buys a far simpler
-- deployment with no ordering constraint between migration and backfill.
--
-- NOT a GENERATED column on purpose: that would weld the schema to today's
-- `raw` format, which is already slated to become byte-exact original-line
-- passthrough. Under that change a generated column silently starts producing
-- NULLs archive-wide with no migration step to notice. Keeping the extraction
-- rule in Rust means the `raw`-format change has to confront it.

ALTER TABLE messages ADD COLUMN message_id TEXT;

-- Partial: ~89% of rows have no provider message id (only assistant responses
-- carry one), and no query looks for rows lacking it.
CREATE INDEX messages_message_id_idx
    ON messages (message_id) WHERE message_id IS NOT NULL;

COMMENT ON COLUMN messages.message_id IS
    'Provider response id (Anthropic msg_…), else the file-history-snapshot '
    'messageId; extracted from raw->>''messageId''. Dedup key for usage '
    'rollups. Distinct from message_key (content-derived row-dedup key) and '
    'from the surrogate id.';


-- Tool invocations extracted once, at ingest, so tool/skill statistics never
-- scan message JSONB at query time — these rollups run over the whole archive,
-- so this is the hot path, not an occasional lookup.
--
-- Extraction is hub-side from the normalized `content` rather than daemon-side
-- over the wire: one code path then serves both live ingest and the backfill of
-- the ~9 months already stored, and no daemon rollout is required.
--
-- Rows are DERIVED DATA — deleting any or all of them is always safe; the
-- backfill regenerates them from `content`.
CREATE TABLE message_tool_uses (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    message_ref  BIGINT      NOT NULL
        REFERENCES messages (id) ON DELETE CASCADE,
    -- Denormalized from the owning message so an invocation can be joined to
    -- its outcome by (session_id, tool_use_id). `tool_use_id` alone is NOT
    -- unique archive-wide — providers reuse the id space across sessions and
    -- machines — so joining on it alone FANS OUT and corrupts success rate.
    -- Caught by analytics_extract_test.
    session_id   BIGINT      NOT NULL
        REFERENCES sessions (id) ON DELETE CASCADE,
    -- Ordinal of the invocation within its message. Together with message_ref
    -- this makes re-extraction idempotent: the extractor upserts on this key,
    -- so re-ingesting a message cannot accumulate duplicate invocation rows.
    seq          INTEGER     NOT NULL,
    tool_name    TEXT        NOT NULL,
    -- The `tool_use` content item's own id, used to join this invocation to its
    -- result (see message_tool_results). NULL for the top-level `toolUse` shape,
    -- which carries its result on the SAME record and so needs no join.
    tool_use_id  TEXT,
    -- Populated only when tool_name is the Claude `Skill` tool, from
    -- `input.skill` — so a skill is reportable by name instead of collapsing
    -- into one aggregated `Skill` entry. (issue #321)
    skill_name   TEXT,
    -- Likewise for the Claude `Agent` tool, from `input.subagent_type`, which
    -- `ProjectStatsSummary.most_used_subagents` reports separately from tools
    -- and skills. Both stay NULL for providers with no such abstraction.
    subagent_type TEXT,
    -- Only meaningful for the top-level `toolUse` shape, whose result rides the
    -- same record. For content-array invocations the outcome lives in a LATER
    -- user message, so it is resolved through message_tool_results instead —
    -- see that table's note for why this column alone is not enough.
    is_error     BOOLEAN     NOT NULL DEFAULT false,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (message_ref, seq)
);

-- Usage rollups group by tool name across a scope. (message_ref lookups are
-- already served by the UNIQUE (message_ref, seq) btree, so no separate index.)
CREATE INDEX message_tool_uses_tool_name_idx ON message_tool_uses (tool_name);

-- Skill and subagent reporting only ever filter to rows that name one.
CREATE INDEX message_tool_uses_skill_name_idx
    ON message_tool_uses (skill_name) WHERE skill_name IS NOT NULL;

CREATE INDEX message_tool_uses_subagent_type_idx
    ON message_tool_uses (subagent_type) WHERE subagent_type IS NOT NULL;

-- Joining an invocation to its outcome. Scoped by session on purpose — see
-- the session_id column note.
CREATE INDEX message_tool_uses_session_tool_use_idx
    ON message_tool_uses (session_id, tool_use_id) WHERE tool_use_id IS NOT NULL;


-- Tool OUTCOMES, extracted from the `tool_result` content items that report
-- them.
--
-- Why this is a separate table rather than a flag on the invocation: a
-- `tool_use` content item does NOT carry `is_error`. The outcome arrives in a
-- LATER user message, as a `tool_result` item referencing the invocation by
-- `tool_use_id`. The retired desktop implementation missed this — it reads
-- `is_error` straight off the `tool_use` item (stats.rs:560), where the key
-- never exists, so `unwrap_or(false)` scores every content-array invocation as
-- a success and its reported success rate is ~100% by construction.
--
-- Storing outcomes separately and resolving them with a LEFT JOIN at query time
-- gives a real success rate. Both rows always live in the same session, so no
-- cross-batch reconciliation is needed: whichever message arrives second simply
-- finds the other already stored.
--
-- CONSEQUENCE FOR THE VERIFICATION GATE: success rate is the one metric that is
-- deliberately expected NOT to match the desktop oracle. Every other field must
-- still match exactly. See the change's design.md.
CREATE TABLE message_tool_results (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    message_ref  BIGINT      NOT NULL
        REFERENCES messages (id) ON DELETE CASCADE,
    -- See message_tool_uses.session_id: this is the other half of the scoped
    -- join key. An invocation and the result reporting on it ALWAYS share a
    -- session, which is what makes the pairing well-defined.
    session_id   BIGINT      NOT NULL
        REFERENCES sessions (id) ON DELETE CASCADE,
    seq          INTEGER     NOT NULL,
    tool_use_id  TEXT        NOT NULL,
    is_error     BOOLEAN     NOT NULL DEFAULT false,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (message_ref, seq)
);

-- The join key from message_tool_uses.
CREATE INDEX message_tool_results_session_tool_use_idx
    ON message_tool_results (session_id, tool_use_id);
