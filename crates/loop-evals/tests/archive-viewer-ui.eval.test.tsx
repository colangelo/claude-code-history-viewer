/**
 * T1 evals for "Archive viewer UI: browse and search the hub archive from
 * the viewer" (Gitea #5).
 *
 * Backend-observable criteria (hub CORS, UserSettings hub fields) live in
 * `crates/loop-evals/tests/archive-viewer-ui_eval.rs`.
 *
 * Strategy: `hubApi` is imported for real (never `vi.mock`'d as a module) so
 * that AC4-AC7 exercise the actual `fetch`-based implementation against a
 * stubbed `global.fetch`, and AC7's `hubMessageToClaudeMessage` runs for
 * real. Component tests (AC8-AC12) override individual `hubApi` methods
 * with `vi.spyOn` — since `hubApi` is a plain exported object, spying on its
 * methods also redirects the calls `ArchiveBrowser` makes internally,
 * without needing to replace the whole module. `vi.restoreAllMocks()` after
 * each test returns `hubApi` to its real implementation.
 *
 * Every criterion here fails against the unmodified stubs: `hubApi` methods
 * throw "not implemented", `hubMessageToClaudeMessage` throws, and
 * `ArchiveBrowser`/`ArchiveHubSection` render only an empty
 * `data-testid` div with no data, inputs, or interactive controls.
 */

import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { hubApi, hubMessageToClaudeMessage } from "@/services/hubApi";
import type {
  HubConfig,
  HubProject,
  HubSession,
  HubMessage,
  HubSearchHit,
} from "@/services/hubApi";
import { ArchiveBrowser } from "@/components/ArchiveBrowser";
import { ArchiveHubSection } from "@/components/SettingsManager/sections/ArchiveHubSection";

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next"
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, fallback?: string) => fallback ?? key,
    }),
  };
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

const CFG: HubConfig = { url: "http://hub:8787", token: "tok" };

// ============================================================================
// fetch stub helpers
// ============================================================================

function fakeHeaders(map: Record<string, string>) {
  const lower = new Map(
    Object.entries(map).map(([k, v]) => [k.toLowerCase(), v])
  );
  return { get: (name: string) => lower.get(name.toLowerCase()) ?? null };
}

function jsonResponse(
  body: unknown,
  opts: { ok?: boolean; status?: number; headers?: Record<string, string> } = {}
) {
  return {
    ok: opts.ok ?? true,
    status: opts.status ?? 200,
    headers: fakeHeaders(opts.headers ?? {}),
    json: async () => body,
  };
}

function headerValue(
  init: (RequestInit & { headers?: unknown }) | undefined,
  name: string
): string | undefined {
  const h = init?.headers as
    | { get?: (name: string) => string | null }
    | Record<string, string>
    | undefined;
  if (!h) return undefined;
  if (typeof (h as { get?: unknown }).get === "function") {
    return (h as { get: (name: string) => string | null }).get(name) ?? undefined;
  }
  const record = h as Record<string, string>;
  const key = Object.keys(record).find(
    (k) => k.toLowerCase() === name.toLowerCase()
  );
  return key ? record[key] : undefined;
}

function requestUrl(call: unknown[]): URL {
  const target = call[0];
  return new URL(String(target));
}

// ============================================================================
// AC4-AC7: hubApi client (real implementation, stubbed fetch)
// ============================================================================

describe("hubApi", () => {
  it("AC4: listProjects issues one authenticated GET to /v1/projects", async () => {
    const fixture: HubProject[] = [
      {
        id: 1,
        provider: "claude",
        project_path: "/tmp/a",
        name: "alpha",
        storage_type: "jsonl",
        session_count: 2,
        message_count: 10,
        last_modified: null,
        machine_id: "m1",
        machine_hostname: "host-a",
      },
    ];
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse(fixture));
    vi.stubGlobal("fetch", fetchMock);

    const result = await hubApi.listProjects(CFG);

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const call = fetchMock.mock.calls[0];
    const url = requestUrl(call);
    expect(`${url.origin}${url.pathname}`).toBe("http://hub:8787/v1/projects");
    expect(headerValue(call[1], "Authorization")).toBe("Bearer tok");
    expect(result).toEqual(fixture);
  });

  it("AC5: sessionMessages targets /v1/sessions/{ref}/messages and reads X-Total-Count", async () => {
    const ref = "6741a288-41fb-4cce-8b2d-a027c391b4da";
    const fixtureMessages: HubMessage[] = [
      {
        id: 1,
        message_key: "k1",
        uuid: "u1",
        parent_uuid: null,
        seq: 0,
        timestamp: "2026-01-01T00:00:00Z",
        message_type: "user",
        role: "user",
        model: null,
        stop_reason: null,
        input_tokens: null,
        output_tokens: null,
        cost_usd: null,
        duration_ms: null,
        is_sidechain: false,
        content: "hi",
      },
    ];
    const fetchMock = vi.fn().mockResolvedValue(
      jsonResponse(fixtureMessages, { headers: { "X-Total-Count": "120" } })
    );
    vi.stubGlobal("fetch", fetchMock);

    const page = await hubApi.sessionMessages(CFG, ref, { limit: 100, offset: 0 });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const url = requestUrl(fetchMock.mock.calls[0]);
    expect(url.pathname).toBe(`/v1/sessions/${ref}/messages`);
    expect(url.searchParams.get("limit")).toBe("100");
    expect(url.searchParams.get("offset")).toBe("0");
    expect(page).toEqual({ messages: fixtureMessages, totalCount: 120 });
  });

  it("AC6: search targets /v1/search with q + filters, rejects on non-2xx", async () => {
    const fixtureHits: HubSearchHit[] = [
      {
        provider: "claude",
        session_id: "s1",
        session_summary: "a session",
        project_name: "alpha",
        project_path: "/tmp/a",
        machine_hostname: "host-a",
        timestamp: "2026-01-01T00:00:00Z",
        snippet: "the needle fix landed",
        rank: 0.5,
      },
    ];
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse(fixtureHits));
    vi.stubGlobal("fetch", fetchMock);

    const hits = await hubApi.search(CFG, "needle fix", { project: "alpha" });

    const url = requestUrl(fetchMock.mock.calls[0]);
    expect(url.pathname).toBe("/v1/search");
    expect(url.searchParams.get("q")).toBe("needle fix");
    expect(url.searchParams.get("project")).toBe("alpha");
    expect(hits).toEqual(fixtureHits);

    const failingFetch = vi.fn().mockResolvedValue(
      jsonResponse({ error: "boom" }, { ok: false, status: 500 })
    );
    vi.stubGlobal("fetch", failingFetch);
    await expect(hubApi.search(CFG, "needle")).rejects.toBeTruthy();
  });

  it("AC7: hubMessageToClaudeMessage maps a row and never throws on unknown type", () => {
    const assistantRow: HubMessage = {
      id: 1,
      message_key: "k1",
      uuid: "u1",
      parent_uuid: null,
      seq: 0,
      timestamp: "2026-01-01T00:00:00Z",
      message_type: "assistant",
      role: "assistant",
      model: "claude-opus",
      stop_reason: "end_turn",
      input_tokens: 10,
      output_tokens: 20,
      cost_usd: null,
      duration_ms: null,
      is_sidechain: false,
      content: [{ type: "text", text: "hello archive" }],
    };
    const mapped = hubMessageToClaudeMessage(assistantRow, "sess-1");
    expect(mapped.type).toBe("assistant");
    expect(JSON.stringify(mapped.content)).toContain("hello archive");

    const unknownTypeRow: HubMessage = {
      ...assistantRow,
      message_type: null,
      role: null,
    };
    let mappedUnknown: ReturnType<typeof hubMessageToClaudeMessage> | undefined;
    expect(() => {
      mappedUnknown = hubMessageToClaudeMessage(unknownTypeRow, "sess-1");
    }).not.toThrow();
    expect(mappedUnknown).toBeTruthy();
    expect(typeof mappedUnknown?.type).toBe("string");
    expect((mappedUnknown?.type ?? "").length).toBeGreaterThan(0);
  });
});

// ============================================================================
// AC8-AC11: ArchiveBrowser
// ============================================================================

function project(overrides: Partial<HubProject>): HubProject {
  return {
    id: 1,
    provider: "claude",
    project_path: "/tmp/proj",
    name: "proj",
    storage_type: "jsonl",
    session_count: 1,
    message_count: 1,
    last_modified: null,
    machine_id: "m1",
    machine_hostname: "host-default",
    ...overrides,
  };
}

function session(overrides: Partial<HubSession>): HubSession {
  return {
    id: 1,
    provider: "claude",
    session_id: "s1",
    summary: "a session",
    file_path: null,
    entrypoint: null,
    message_count: 1,
    first_message_time: null,
    last_message_time: null,
    has_tool_use: false,
    has_errors: false,
    project_name: "proj",
    project_path: "/tmp/proj",
    machine_hostname: "host-default",
    ...overrides,
  };
}

function messageRow(overrides: Partial<HubMessage>): HubMessage {
  return {
    id: 1,
    message_key: "k1",
    uuid: "u1",
    parent_uuid: null,
    seq: 0,
    timestamp: "2026-01-01T00:00:00Z",
    message_type: "user",
    role: "user",
    model: null,
    stop_reason: null,
    input_tokens: null,
    output_tokens: null,
    cost_usd: null,
    duration_ms: null,
    is_sidechain: false,
    content: "message text",
    ...overrides,
  };
}

/**
 * Activate the Browse view if the archive browser presents Journal/Browse tabs.
 * The journal-webapp-ui change makes Journal the default landing view; these
 * AC8-AC11 assertions target the Browse panes, so they click into Browse first.
 * Guarded so it is a no-op on the pre-tabs layout — this eval must pass both
 * before that change (no tab → Browse is already the only view) and after it.
 */
function showBrowse() {
  const browseTab = screen.queryByTestId("archive-tab-browse");
  if (browseTab) fireEvent.click(browseTab);
}

describe("ArchiveBrowser", () => {
  it("AC8: renders archived projects with name and machine hostname", async () => {
    const projects = [
      project({ id: 1, name: "alpha-project", machine_hostname: "host-alpha" }),
      project({ id: 2, name: "beta-project", machine_hostname: "host-beta" }),
    ];
    vi.spyOn(hubApi, "listProjects").mockResolvedValue(projects);
    vi.spyOn(hubApi, "listSessions").mockResolvedValue([]);

    render(<ArchiveBrowser config={CFG} />);
    showBrowse();

    await screen.findByText("alpha-project");
    await screen.findByText("beta-project");
    await screen.findByText("host-alpha");
    await screen.findByText("host-beta");
  });

  it("AC9: selecting a project filters listSessions and renders session summaries", async () => {
    const target = project({
      id: 1,
      name: "alpha-project",
      project_path: "/tmp/alpha-project",
      machine_hostname: "host-alpha",
    });
    const listSessions = vi
      .spyOn(hubApi, "listSessions")
      .mockResolvedValue([
        session({ id: 10, summary: "first archived session" }),
        session({ id: 11, summary: "second archived session" }),
      ]);
    vi.spyOn(hubApi, "listProjects").mockResolvedValue([target]);

    render(<ArchiveBrowser config={CFG} />);
    showBrowse();
    await screen.findByText("alpha-project");
    fireEvent.click(screen.getByText("alpha-project"));

    await waitFor(() => expect(listSessions).toHaveBeenCalled());
    const [, options] = listSessions.mock.calls[listSessions.mock.calls.length - 1];
    expect([target.name, target.project_path]).toContain(options?.project);

    await screen.findByText("first archived session");
    await screen.findByText("second archived session");
  });

  it("AC10: sessions page with load-more, appends remaining messages driven by totalCount", async () => {
    const targetProject = project({ id: 1, name: "alpha-project" });
    const targetSession = session({ id: 10, summary: "paged session" });
    const firstPage = Array.from({ length: 200 }, (_, i) =>
      messageRow({ id: i + 1, message_key: `k${i}`, content: `page1-msg-${i}` })
    );
    const secondPage = Array.from({ length: 50 }, (_, i) =>
      messageRow({
        id: 201 + i,
        message_key: `k2-${i}`,
        content: `page2-msg-${i}`,
      })
    );
    const sessionMessages = vi
      .spyOn(hubApi, "sessionMessages")
      .mockResolvedValueOnce({ messages: firstPage, totalCount: 250 })
      .mockResolvedValueOnce({ messages: secondPage, totalCount: 250 });
    vi.spyOn(hubApi, "listProjects").mockResolvedValue([targetProject]);
    vi.spyOn(hubApi, "listSessions").mockResolvedValue([targetSession]);

    render(<ArchiveBrowser config={CFG} />);
    showBrowse();
    await screen.findByText("alpha-project");
    fireEvent.click(screen.getByText("alpha-project"));
    await screen.findByText("paged session");
    fireEvent.click(screen.getByText("paged session"));

    await screen.findByText("page1-msg-0");
    const loadMore = await screen.findByTestId("archive-load-more");

    fireEvent.click(loadMore);

    await screen.findByText("page2-msg-49");
    expect(sessionMessages).toHaveBeenCalledTimes(2);
    const secondCallOptions = sessionMessages.mock.calls[1][2];
    expect(secondCallOptions?.offset).toBe(200);

    await waitFor(() =>
      expect(screen.queryByTestId("archive-load-more")).not.toBeInTheDocument()
    );
  });

  it("AC11: search renders hit snippet/hostname and activating opens its session", async () => {
    const hit: HubSearchHit = {
      provider: "claude",
      session_id: "session-uuid-123",
      session_summary: "a hit session",
      project_name: "alpha-project",
      project_path: "/tmp/alpha-project",
      machine_hostname: "host-search",
      timestamp: "2026-01-01T00:00:00Z",
      snippet: "the quick brown fox jumped",
      rank: 0.75,
    };
    vi.spyOn(hubApi, "listProjects").mockResolvedValue([]);
    vi.spyOn(hubApi, "listSessions").mockResolvedValue([]);
    vi.spyOn(hubApi, "search").mockResolvedValue([hit]);
    const sessionMessages = vi
      .spyOn(hubApi, "sessionMessages")
      .mockResolvedValue({ messages: [], totalCount: 0 });

    render(<ArchiveBrowser config={CFG} />);

    const searchInput = await screen.findByTestId("archive-search-input");
    fireEvent.change(searchInput, { target: { value: "quick fox" } });
    fireEvent.submit(searchInput.closest("form") ?? searchInput);

    await screen.findByText("the quick brown fox jumped");
    await screen.findByText("host-search");

    fireEvent.click(screen.getByText("the quick brown fox jumped"));

    await waitFor(() =>
      expect(sessionMessages).toHaveBeenCalledWith(
        CFG,
        hit.session_id,
        expect.anything()
      )
    );
  });
});

// ============================================================================
// AC12: ArchiveHubSection
// ============================================================================

describe("ArchiveHubSection", () => {
  it("AC12: shows initial values in labelled inputs and saves edits", async () => {
    const onSave = vi.fn();
    render(
      <ArchiveHubSection
        initialUrl="http://hub:8787"
        initialToken="secret-token"
        onSave={onSave}
      />
    );

    const urlInput = (await screen.findByTestId(
      "archive-hub-url-input"
    )) as HTMLInputElement;
    const tokenInput = (await screen.findByTestId(
      "archive-hub-token-input"
    )) as HTMLInputElement;

    expect(urlInput.value).toBe("http://hub:8787");
    expect(tokenInput.value).toBe("secret-token");
    expect(urlInput.labels?.length ?? 0).toBeGreaterThan(0);
    expect(tokenInput.labels?.length ?? 0).toBeGreaterThan(0);

    fireEvent.change(urlInput, { target: { value: "http://new-hub:9999" } });
    fireEvent.change(tokenInput, { target: { value: "new-token" } });
    fireEvent.click(screen.getByTestId("archive-hub-save-button"));

    await waitFor(() =>
      expect(onSave).toHaveBeenCalledWith("http://new-hub:9999", "new-token")
    );
  });
});
