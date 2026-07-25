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
-- Left NULL here and backfilled in batches; the supporting index is created
-- afterwards (concurrently, in its own migration) so the bulk UPDATE does not
-- pay per-row index maintenance, and so the index shape can be chosen against
-- a real EXPLAIN of the rollup queries.
--
-- NOT a GENERATED column on purpose: that would weld the schema to today's
-- `raw` format, which is already slated to become byte-exact original-line
-- passthrough. Under that change a generated column silently starts producing
-- NULLs archive-wide with no migration step to notice. Keeping the extraction
-- rule in Rust means the `raw`-format change has to confront it.

ALTER TABLE messages ADD COLUMN message_id TEXT;

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
    -- Ordinal of the invocation within its message. Together with message_ref
    -- this makes re-extraction idempotent: the extractor upserts on this key,
    -- so re-ingesting a message cannot accumulate duplicate invocation rows.
    seq          INTEGER     NOT NULL,
    tool_name    TEXT        NOT NULL,
    -- Populated only when tool_name is the Claude `Skill` tool, from
    -- `input.skill` — so a skill is reportable by name instead of collapsing
    -- into one aggregated `Skill` entry. (issue #321)
    skill_name   TEXT,
    -- Likewise for the Claude `Agent` tool, from `input.subagent_type`, which
    -- `ProjectStatsSummary.most_used_subagents` reports separately from tools
    -- and skills. Both stay NULL for providers with no such abstraction.
    subagent_type TEXT,
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
