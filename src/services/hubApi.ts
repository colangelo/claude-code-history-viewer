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
  /** Git-fingerprint identity (null = not a git repo → path identity). */
  identity_key: string | null;
  /** Linked `git worktree` member of its identity. */
  git_worktree: boolean;
  /** For worktrees: the main checkout's path. */
  git_main_path: string | null;
}

/** One member path of an identity (`GET /v1/identities`). */
export interface HubIdentityMember {
  project_path: string;
  providers: string[];
  machines: string[];
  /** True when every row binding this path to the identity is a worktree. */
  worktree: boolean;
  main_path: string | null;
  last_active: string | null;
}

export interface HubIdentityAlias {
  id: number;
  project_path: string;
  created_by: string;
  created_at: string;
}

/** Advisory link suggestion — never acted on automatically. */
export interface HubIdentitySuggestion {
  kind: "orphan_path" | "related_identity" | string;
  project_path?: string;
  identity_key?: string;
}

/** One identity (`GET /v1/identities`): equivalence class of project rows. */
export interface HubIdentity {
  identity_key: string;
  display_name: string;
  members: HubIdentityMember[];
  aliases: HubIdentityAlias[];
  suggestions: HubIdentitySuggestion[];
}

/** Build the `identity:<key>` form of the `project` filter param. */
export function identityProjectFilter(identityKey: string): string {
  return `identity:${identityKey}`;
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
  /** Hub message pk of the hit (present since cchv-v0.10.1). */
  message_id?: number;
  uuid?: string | null;
  /** 0-based index of the hit in its session's browse ordering (since
   * cchv-v0.10.1) — lets the client open the page containing the match.
   * Absent on older hubs → the client falls back to page 1. */
  position?: number;
}

/**
 * Row shape of `GET /v1/journal/entries` (crates/hub/src/journal.rs
 * `JournalEntry`). One distilled `(entry_date, project_path)` entry.
 */
export interface JournalEntry {
  entry_date: string;
  project_path: string;
  status: string;
  headline: string | null;
  summary: string | null;
  topics: string[];
  open_questions: string[];
  session_ids: number[];
  model: string | null;
  generated_at: string;
}

/**
 * A journal hit inside the `journal` block of `GET /v1/search`
 * (crates/hub/src/journal.rs `JournalHit`) — a `JournalEntry` plus its
 * independent full-text rank.
 */
export interface JournalSearchHit extends Omit<JournalEntry, "status"> {
  rank: number;
}

/**
 * The journal block of a search plus the hub's degradation indicator:
 * `degraded` is `true` when a semantic/hybrid request fell back to
 * keyword-only results (embedder or embeddings unavailable hub-side).
 */
export interface JournalSearchResult {
  hits: JournalSearchHit[];
  degraded: boolean;
}

export interface HubJournalOptions {
  project?: string;
  /** In identity scope: `false` excludes worktree-only member paths. */
  include_worktrees?: boolean;
  from?: string;
  to?: string;
  limit?: number;
  offset?: number;
}

export interface HubListOptions {
  machine?: string;
  provider?: string;
  project?: string;
  /** In identity scope: `false` excludes worktree-only member paths. */
  include_worktrees?: boolean;
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
  params?: Record<string, string | number | boolean | undefined>
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

/** Non-GET request (alias create/delete). JSON body when provided. */
async function hubSend(
  method: "POST" | "DELETE",
  url: URL,
  config: HubConfig,
  body?: unknown
): Promise<Response> {
  const res = await fetch(url.toString(), {
    method,
    headers: {
      ...authHeaders(config),
      ...(body !== undefined ? { "Content-Type": "application/json" } : {}),
    },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) {
    throw new Error(`hub ${method} to ${url.pathname} failed: ${res.status}`);
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
      include_worktrees: options?.include_worktrees,
      limit: options?.limit,
      offset: options?.offset,
    });
    const res = await hubGet(url, config);
    return (await res.json()) as HubSession[];
  },

  /** `GET /v1/identities` — identity groups with members/aliases/suggestions. */
  async listIdentities(config: HubConfig): Promise<HubIdentity[]> {
    const url = hubUrl(config, "/v1/identities");
    const res = await hubGet(url, config);
    return (await res.json()) as HubIdentity[];
  },

  /**
   * `POST /v1/identities/aliases` — attach a (typically moved-away) path to an
   * identity. Reversible via {@link deleteAlias}; never rewrites archived rows.
   */
  async createAlias(
    config: HubConfig,
    projectPath: string,
    identityKey: string
  ): Promise<HubIdentityAlias> {
    const url = hubUrl(config, "/v1/identities/aliases");
    const res = await hubSend("POST", url, config, {
      project_path: projectPath,
      identity_key: identityKey,
    });
    return (await res.json()) as HubIdentityAlias;
  },

  /** `DELETE /v1/identities/aliases/{id}` — undo a link. */
  async deleteAlias(config: HubConfig, aliasId: number): Promise<void> {
    const url = hubUrl(config, `/v1/identities/aliases/${aliasId}`);
    await hubSend("DELETE", url, config);
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

  /**
   * `GET /v1/journal/entries` — distilled per-day journal entries, newest-first,
   * filterable by project and inclusive `from`/`to` date bounds, paginated.
   */
  async journalEntries(
    config: HubConfig,
    options?: HubJournalOptions
  ): Promise<JournalEntry[]> {
    const url = hubUrl(config, "/v1/journal/entries", {
      project: options?.project,
      include_worktrees: options?.include_worktrees,
      from: options?.from,
      to: options?.to,
      limit: options?.limit,
      offset: options?.offset,
    });
    const res = await hubGet(url, config);
    const data = (await res.json()) as
      | JournalEntry[]
      | { entries?: JournalEntry[]; results?: JournalEntry[] };
    if (Array.isArray(data)) return data;
    return data.entries ?? data.results ?? [];
  },

  /**
   * The `journal` block of `GET /v1/search`.
   * Kept separate from {@link search} so that method's array return stays
   * byte-compatible with existing callers; absence of the block yields `[]`.
   *
   * `scope=journal` because this method reads ONLY the journal block. The
   * default `scope=all` also runs the message leg — a GIN probe of
   * `messages_text_search_idx` plus a correlated `position` subquery per hit —
   * whose results are then thrown away. `ArchiveBrowser.runSearch` fires this
   * alongside {@link search}, so at the default every user search paid for the
   * message leg twice. Scoping each call to the block it reads halves that.
   * A pre-journal hub ignores the unknown param and returns the bare array,
   * which still degrades to an empty result below.
   *
   * `mode=hybrid` fuses keyword FTS with semantic (embedding) ranking on
   * hubs that support it (cchv-v0.12.0+); older hubs ignore the param.
   * `degraded: true` means the hub fell back to keyword-only because its
   * embedder or embeddings were unavailable — recall quality dropped, the
   * results are still valid.
   */
  async journalSearch(
    config: HubConfig,
    query: string,
    options?: HubSearchOptions
  ): Promise<JournalSearchResult> {
    const url = hubUrl(config, "/v1/search", {
      q: query,
      scope: "journal",
      mode: "hybrid",
      machine: options?.machine,
      provider: options?.provider,
      project: options?.project,
      include_worktrees: options?.include_worktrees,
      from: options?.from,
      to: options?.to,
      limit: options?.limit,
      offset: options?.offset,
    });
    const res = await hubGet(url, config);
    const data = (await res.json()) as
      | HubSearchHit[]
      | { journal?: JournalSearchHit[]; journal_degraded?: boolean };
    if (Array.isArray(data)) return { hits: [], degraded: false };
    return { hits: data.journal ?? [], degraded: data.journal_degraded === true };
  },

  /**
   * `GET /v1/search` — Postgres websearch full-text query over the archive.
   * `scope=messages` for the same reason {@link journalSearch} uses
   * `scope=journal`: this method reads only `results`, so the journal leg is
   * pure waste here. The response is byte-compatible with the pre-journal
   * shape at this scope.
   */
  async search(
    config: HubConfig,
    query: string,
    options?: HubSearchOptions
  ): Promise<HubSearchHit[]> {
    const url = hubUrl(config, "/v1/search", {
      q: query,
      scope: "messages",
      machine: options?.machine,
      provider: options?.provider,
      project: options?.project,
      include_worktrees: options?.include_worktrees,
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
