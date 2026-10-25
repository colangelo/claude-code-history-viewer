-- Where in a message each finding sat (Gitea #82): the content-block kinds of
-- its hits (`text`, `tool_result`, `tool_input`, `tool_use`, `other`), so a
-- fetched page in a tool result can be told from text a person typed without
-- opening the transcript. Empty for `search_text` and for rows written before.
ALTER TABLE credential_findings ADD COLUMN IF NOT EXISTS origins TEXT[] NOT NULL DEFAULT '{}';
