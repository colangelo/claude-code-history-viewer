/**
 * Archive hub read API client (spec: openspec/specs/archive-search-api/spec.md).
 *
 * Talks to the cchv archive hub DIRECTLY from the frontend (Tauri webview and
 * WebUI alike) with plain `fetch` — deliberately NOT through `services/api.ts`:
 * the hub is a separate service (`crates/hub`, `/v1/*` GET endpoints, bearer
 * token) and adding per-command Tauri/axum proxies would re-open the
 * command/route parity bug class (#340/#355). CORS on the hub side makes the
 * direct call possible from browser contexts.
 *
 * All methods take the hub config explicitly so they are trivially testable;
 * callers read it from user settings (archiveHubUrl / archiveHubToken).
 */

import type { ClaudeMessage } from "../types";

export interface HubConfig {
  /** Base URL of the hub, e.g. `http://100.79.255.107:8787` (no trailing slash). */
  url: string;
  /** Bearer token for the hub's read API. */
  token: string;
}

/** Row shape of `GET /v1/projects` (crates/hub/src/browse.rs `ProjectRow`). */
export interface HubProject {
  id: number;
  provider: string;
  project_path: string;
  name: string | null;
  storage_type: string | null;
  session_count: number;
  message_count: number;
  last_modified: string | null;
  machine_id: string;
  machine_hostname: string;
}

/** Row shape of `GET /v1/sessions` (crates/hub/src/browse.rs `SessionRow`). */
export interface HubSession {
  id: number;
  provider: string;
  session_id: string;
  summary: string | null;
  file_path: string | null;
  entrypoint: string | null;
  message_count: number;
  first_message_time: string | null;
  last_message_time: string | null;
  has_tool_use: boolean;
  has_errors: boolean;
  project_name: string | null;
  project_path: string | null;
  machine_hostname: string;
}

/** Row shape of `GET /v1/sessions/{id}/messages` (crates/hub/src/browse.rs `MessageRow`). */
export interface HubMessage {
  id: number;
  message_key: string;
  uuid: string | null;
  parent_uuid: string | null;
  seq: number;
  timestamp: string | null;
  message_type: string | null;
  role: string | null;
  model: string | null;
  stop_reason: string | null;
  input_tokens: number | null;
  output_tokens: number | null;
  cost_usd: number | null;
  duration_ms: number | null;
  is_sidechain: boolean;
  content: unknown;
}

/** One page of a session's messages plus the session total from `X-Total-Count`. */
export interface HubMessagePage {
  messages: HubMessage[];
  totalCount: number;
}

/** Hit shape of `GET /v1/search` results (crates/hub/src/search.rs). */
export interface HubSearchHit {
  provider: string;
  session_id: string;
  session_summary: string | null;
  project_name: string | null;
  project_path: string | null;
  machine_hostname: string;
  timestamp: string | null;
  snippet: string;
  rank: number;
}

export interface HubListOptions {
  machine?: string;
  provider?: string;
  project?: string;
  limit?: number;
  offset?: number;
}

export interface HubSearchOptions extends HubListOptions {
  from?: string;
  to?: string;
}

export interface HubPageOptions {
  limit?: number;
  offset?: number;
}

const NOT_IMPLEMENTED = "hubApi: not implemented";

export const hubApi = {
  /** `GET /v1/healthz` — no auth; true iff the hub is reachable and healthy. */
  async healthz(_config: HubConfig): Promise<boolean> {
    void _config;
    throw new Error(NOT_IMPLEMENTED);
  },

  /** `GET /v1/projects` — archived projects across machines. */
  async listProjects(
    _config: HubConfig,
    _options?: HubListOptions
  ): Promise<HubProject[]> {
    void [_config, _options];
    throw new Error(NOT_IMPLEMENTED);
  },

  /** `GET /v1/sessions` — archived sessions, filterable by project/machine/provider. */
  async listSessions(
    _config: HubConfig,
    _options?: HubListOptions
  ): Promise<HubSession[]> {
    void [_config, _options];
    throw new Error(NOT_IMPLEMENTED);
  },

  /**
   * `GET /v1/sessions/{ref}/messages` — one page of a session's messages in
   * chronological order. `ref` is the numeric hub session id or the provider
   * session UUID. `totalCount` comes from the `X-Total-Count` response header.
   */
  async sessionMessages(
    _config: HubConfig,
    _ref: number | string,
    _options?: HubPageOptions
  ): Promise<HubMessagePage> {
    void [_config, _ref, _options];
    throw new Error(NOT_IMPLEMENTED);
  },

  /** `GET /v1/search` — Postgres websearch full-text query over the archive. */
  async search(
    _config: HubConfig,
    _query: string,
    _options?: HubSearchOptions
  ): Promise<HubSearchHit[]> {
    void [_config, _query, _options];
    throw new Error(NOT_IMPLEMENTED);
  },
};

/**
 * Map a hub message row to the viewer's `ClaudeMessage` union so archived
 * messages render through the existing message renderers.
 */
export function hubMessageToClaudeMessage(
  _row: HubMessage,
  _sessionId: string
): ClaudeMessage {
  void [_row, _sessionId];
  throw new Error(NOT_IMPLEMENTED);
}
