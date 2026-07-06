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
  /** Base URL of the hub, e.g. `https://m4m.cat-bluegill.ts.net:8788` (no trailing slash). */
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

/** Strip a trailing slash so `${base}${path}` never doubles one up. */
function baseUrl(config: HubConfig): string {
  return config.url.replace(/\/+$/, "");
}

/** Build a hub URL, only setting query params that are actually provided. */
function hubUrl(
  config: HubConfig,
  path: string,
  params?: Record<string, string | number | undefined>
): URL {
  const url = new URL(`${baseUrl(config)}${path}`);
  if (params) {
    for (const [key, value] of Object.entries(params)) {
      if (value !== undefined) {
        url.searchParams.set(key, String(value));
      }
    }
  }
  return url;
}

function authHeaders(config: HubConfig): HeadersInit {
  // Empty token = rely on host-side auth (e.g. Tailscale serve identity
  // headers on a same-origin hub) — sending "Bearer " would only force a
  // pointless CORS preflight and a guaranteed 401 on the bearer path.
  return config.token ? { Authorization: `Bearer ${config.token}` } : {};
}

async function hubGet(url: URL, config: HubConfig): Promise<Response> {
  const res = await fetch(url.toString(), { headers: authHeaders(config) });
  if (!res.ok) {
    throw new Error(`hub request to ${url.pathname} failed: ${res.status}`);
  }
  return res;
}

export const hubApi = {
  /** `GET /v1/healthz` — no auth; true iff the hub is reachable and healthy. */
  async healthz(config: HubConfig): Promise<boolean> {
    try {
      const res = await fetch(`${baseUrl(config)}/v1/healthz`);
      return res.ok;
    } catch {
      return false;
    }
  },

  /** `GET /v1/projects` — archived projects across machines. */
  async listProjects(
    config: HubConfig,
    options?: HubListOptions
  ): Promise<HubProject[]> {
    const url = hubUrl(config, "/v1/projects", {
      machine: options?.machine,
      provider: options?.provider,
      limit: options?.limit,
      offset: options?.offset,
    });
    const res = await hubGet(url, config);
    return (await res.json()) as HubProject[];
  },

  /** `GET /v1/sessions` — archived sessions, filterable by project/machine/provider. */
  async listSessions(
    config: HubConfig,
    options?: HubListOptions
  ): Promise<HubSession[]> {
    const url = hubUrl(config, "/v1/sessions", {
      machine: options?.machine,
      provider: options?.provider,
      project: options?.project,
      limit: options?.limit,
      offset: options?.offset,
    });
    const res = await hubGet(url, config);
    return (await res.json()) as HubSession[];
  },

  /**
   * `GET /v1/sessions/{ref}/messages` — one page of a session's messages in
   * chronological order. `ref` is the numeric hub session id or the provider
   * session UUID. `totalCount` comes from the `X-Total-Count` response header.
   */
  async sessionMessages(
    config: HubConfig,
    ref: number | string,
    options?: HubPageOptions
  ): Promise<HubMessagePage> {
    const url = hubUrl(config, `/v1/sessions/${encodeURIComponent(String(ref))}/messages`, {
      limit: options?.limit,
      offset: options?.offset,
    });
    const res = await hubGet(url, config);
    const messages = (await res.json()) as HubMessage[];
    const totalCountHeader = res.headers.get("x-total-count");
    const totalCount = totalCountHeader !== null ? Number(totalCountHeader) : messages.length;
    return { messages, totalCount };
  },

  /** `GET /v1/search` — Postgres websearch full-text query over the archive. */
  async search(
    config: HubConfig,
    query: string,
    options?: HubSearchOptions
  ): Promise<HubSearchHit[]> {
    const url = hubUrl(config, "/v1/search", {
      q: query,
      machine: options?.machine,
      provider: options?.provider,
      project: options?.project,
      from: options?.from,
      to: options?.to,
      limit: options?.limit,
      offset: options?.offset,
    });
    const res = await hubGet(url, config);
    const data = (await res.json()) as HubSearchHit[] | { results: HubSearchHit[] };
    return Array.isArray(data) ? data : data.results;
  },
};

/**
 * Map a hub message row to the viewer's `ClaudeMessage` union so archived
 * messages render through the existing message renderers. Unknown/missing
 * `message_type` values degrade to a renderable "system" message rather than
 * throwing, since the archive can carry provider message shapes this viewer
 * doesn't otherwise recognize.
 */
export function hubMessageToClaudeMessage(
  row: HubMessage,
  sessionId: string
): ClaudeMessage {
  const uuid = row.uuid ?? row.message_key;
  const timestamp = row.timestamp ?? "";
  const content = (row.content ?? "") as ClaudeMessage["content"];
  const isSidechain = row.is_sidechain;
  const kind = row.message_type ?? row.role;

  if (kind === "user") {
    return {
      type: "user",
      role: "user",
      uuid,
      sessionId,
      timestamp,
      isSidechain,
      content,
    };
  }

  if (kind === "assistant") {
    return {
      type: "assistant",
      role: "assistant",
      uuid,
      sessionId,
      timestamp,
      isSidechain,
      content,
      model: row.model ?? undefined,
      stop_reason: (row.stop_reason ?? undefined) as
        | "tool_use"
        | "end_turn"
        | "max_tokens"
        | "stop_sequence"
        | "pause_turn"
        | "refusal"
        | undefined,
      usage: {
        input_tokens: row.input_tokens ?? undefined,
        output_tokens: row.output_tokens ?? undefined,
      },
      costUSD: row.cost_usd ?? undefined,
      durationMs: row.duration_ms ?? undefined,
    };
  }

  return {
    type: "system",
    subtype: row.message_type ?? "unknown",
    uuid,
    sessionId,
    timestamp,
    isSidechain,
    content,
  };
}
