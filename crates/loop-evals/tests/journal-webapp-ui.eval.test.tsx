/**
 * T1 evals for "Archive journal view + unified type scale (webapp UI for hub
 * journal entries)" — issue #16, change `journal-webapp-ui`.
 *
 * Acceptance authority: openspec/changes/journal-webapp-ui/specs/archive-journal-ui/spec.md
 *
 * STRATEGY — every criterion is frontend-observable (T1). The whole feature is
 * driven through the real `ArchiveBrowser` component against a **stubbed
 * `global.fetch`** (a URL router returning hub-shaped fixtures), exactly the way
 * the frozen archive-viewer-ui eval exercises the real `hubApi`. This is
 * deliberate:
 *   - It imports ONLY surfaces that exist today (`ArchiveBrowser`, `ToolUseCard`,
 *     `MessageContentDisplay`, the renderer `layout` tokens, locale JSON), so the
 *     file loads on the unmodified app.
 *   - The new journal endpoints (`GET /v1/journal/entries`, the `journal` block
 *     of `GET /v1/search`) are addressed purely by URL, so the tests never name a
 *     not-yet-existing `hubApi` method (which would be an import/`spyOn` error).
 *   - Missing behaviour fails at RUNTIME (a query/element is absent), not at
 *     import time — the required pre-implementation failure shape.
 *
 * The unmodified `ArchiveBrowser` renders only the three-pane Browse UI with no
 * tabs and never calls `/v1/journal/entries`, so all journal assertions below
 * fail today; the type-scale assertions fail because message text is 12px and
 * tool ids/headers carry the old tokens.
 *
 * TEST-ID CONTRACT (documented in .secondloop/runs/journal-webapp-ui/evals.md):
 *   archive-tab-journal / archive-tab-browse  — the two <button> tabs (mandated
 *     by the run spec; the frozen eval's Browse-tab click depends on them).
 *   journal-day-header      — one per day group (grouping + ordering).
 *   journal-entry-card      — one per rendered entry.
 *   journal-entry-toggle    — the expand affordance inside a card.
 *   journal-session-link    — one per session id inside an expanded card.
 *   journal-date-picker     — an <input type="date"> that jumps the feed.
 *   journal-project-filter  — a <select> narrowing the feed by project.
 *   journal-empty           — the not-yet-distilled empty notice.
 *   journal-error           — the failed-fetch error state.
 *   journal-search-section  — the journal-hits section in search results.
 *   journal-search-hit      — one clickable journal hit inside that section.
 */

import { describe, it, expect, vi, afterEach } from "vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  within,
  cleanup,
} from "@testing-library/react";
import { ArchiveBrowser } from "@/components/ArchiveBrowser";
import { ToolUseCard } from "@/components/contentRenderer/toolUseRenderers/ToolUseCard";
import { MessageContentDisplay } from "@/components/messageRenderer";
import { ExpandKeyProvider } from "@/contexts/CaptureExpandContext";
import type { HubConfig } from "@/services/hubApi";

import enSettings from "@/i18n/locales/en/settings.json";
import koSettings from "@/i18n/locales/ko/settings.json";
import jaSettings from "@/i18n/locales/ja/settings.json";
import zhCNSettings from "@/i18n/locales/zh-CN/settings.json";
import zhTWSettings from "@/i18n/locales/zh-TW/settings.json";

// A translation stub that returns the key, but folds interpolation VALUES into
// the output so counts/dates rendered via `t(key, { count })` remain observable
// (and honours a plain-string defaultValue). Keys stay visible so we can assert
// that user-facing strings resolve through i18n rather than raw literals.
vi.mock("react-i18next", () => {
  const t = (key: string, opts?: unknown): string => {
    if (typeof opts === "string") return opts;
    if (opts && typeof opts === "object") {
      const o = opts as Record<string, unknown>;
      const keys = Object.keys(o);
      if ("defaultValue" in o && keys.length === 1) return String(o.defaultValue);
      const vals = keys
        .filter((k) => k !== "defaultValue")
        .map((k) => String(o[k]))
        .join(" ");
      return vals ? `${key} ${vals}` : key;
    }
    return key;
  };
  return {
    useTranslation: () => ({
      t,
      i18n: { language: "en", changeLanguage: () => Promise.resolve() },
    }),
    initReactI18next: { type: "3rdParty", init: () => {} },
    Trans: ({ children }: { children?: unknown }) => children ?? null,
  };
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

const CFG: HubConfig = { url: "http://hub:8787", token: "tok" };

// ============================================================================
// fetch-router scaffold
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

interface JournalEntry {
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

function makeEntry(overrides: Partial<JournalEntry>): JournalEntry {
  return {
    entry_date: "2026-07-01",
    project_path: "/work/alpha-project",
    status: "entry",
    headline: "A day of work",
    summary: "Did some things.",
    topics: ["one", "two"],
    open_questions: [],
    session_ids: [1],
    model: "claude-opus",
    generated_at: "2026-07-01T12:00:00Z",
    ...overrides,
  };
}

interface RouterConfig {
  journalEntries?: (p: URLSearchParams) => JournalEntry[];
  journalEntriesResponse?: {
    ok?: boolean;
    status?: number;
    body?: unknown;
  };
  projects?: unknown[];
  sessions?: (p: URLSearchParams) => unknown[];
  messages?: { rows: unknown[]; totalCount: number };
  search?: (p: URLSearchParams) => unknown;
}

interface RouterCalls {
  journalEntries: URLSearchParams[];
  listProjects: URLSearchParams[];
  listSessions: URLSearchParams[];
  sessionMessages: { pathname: string; params: URLSearchParams }[];
  search: URLSearchParams[];
}

function installRouter(cfg: RouterConfig) {
  const calls: RouterCalls = {
    journalEntries: [],
    listProjects: [],
    listSessions: [],
    sessionMessages: [],
    search: [],
  };
  const fn = vi.fn(async (input: unknown) => {
    const url = new URL(String(input));
    const p = url.pathname;
    const params = url.searchParams;

    if (p === "/v1/journal/entries") {
      calls.journalEntries.push(params);
      if (cfg.journalEntriesResponse) {
        const r = cfg.journalEntriesResponse;
        return jsonResponse(r.body ?? [], { ok: r.ok, status: r.status });
      }
      return jsonResponse(cfg.journalEntries ? cfg.journalEntries(params) : []);
    }
    if (p === "/v1/projects") {
      calls.listProjects.push(params);
      return jsonResponse(cfg.projects ?? []);
    }
    if (/^\/v1\/sessions\/[^/]+\/messages$/.test(p)) {
      calls.sessionMessages.push({ pathname: p, params });
      const m = cfg.messages ?? { rows: [], totalCount: 0 };
      return jsonResponse(m.rows, {
        headers: { "X-Total-Count": String(m.totalCount) },
      });
    }
    if (p === "/v1/sessions") {
      calls.listSessions.push(params);
      return jsonResponse(cfg.sessions ? cfg.sessions(params) : []);
    }
    if (p === "/v1/search") {
      calls.search.push(params);
      return jsonResponse(
        cfg.search ? cfg.search(params) : { results: [], limit: 20, offset: 0 }
      );
    }
    if (p === "/v1/healthz") return jsonResponse(true);
    return jsonResponse([]);
  });
  vi.stubGlobal("fetch", fn);
  return calls;
}

// Resolve a font-size (in px) from a single class name. Recognizes the app's
// `text-pxNN` scale plus arbitrary `text-[13px]` / `text-[0.8125rem]` values —
// the latter matter because tailwind-merge STRIPS a bare `text-pxNN` when a
// `text-color` class follows it (which is exactly why tool-card headers inherit
// today), so a working fix commonly uses an arbitrary size that survives.
function classFontSizePx(cls: string): number | null {
  for (const c of cls.split(/\s+/)) {
    let m = /^text-px(\d+)$/.exec(c);
    if (m) return Number(m[1]);
    m = /^text-\[(\d+(?:\.\d+)?)px\]$/.exec(c);
    if (m) return Math.round(Number(m[1]));
    m = /^text-\[(\d+(?:\.\d+)?)rem\]$/.exec(c);
    if (m) return Math.round(Number(m[1]) * 16);
  }
  return null;
}

// Effective font size: the nearest own-or-ancestor font-size class (CSS
// font-size inherits), so the assertion is robust to whether the size lands on
// the text node's element or a wrapper.
function effectiveFontSizePx(el: Element | null): number | null {
  let node: Element | null = el;
  let hops = 0;
  while (node && hops < 8) {
    const px = classFontSizePx(node.getAttribute("class") ?? "");
    if (px !== null) return px;
    node = node.parentElement;
    hops++;
  }
  return null;
}

// Whether an element or any ancestor (up to a small bound) carries a class.
function hasClassUp(el: Element | null, cls: string): boolean {
  let node: Element | null = el;
  let hops = 0;
  while (node && hops < 8) {
    if ((node.getAttribute("class") ?? "").split(/\s+/).includes(cls)) return true;
    node = node.parentElement;
    hops++;
  }
  return false;
}

// Local YYYY-MM-DD for a Date.
function isoDate(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(
    d.getDate()
  ).padStart(2, "0")}`;
}

// ============================================================================
// AC1: journal-default tabs; Browse shows the existing three panes
// ============================================================================

describe("AC1 — Journal/Browse tabs, journal default", () => {
  it("renders Journal by default and Browse reveals the three panes", async () => {
    installRouter({
      journalEntries: () => [
        makeEntry({ headline: "Landing headline", entry_date: "2026-07-10" }),
      ],
      projects: [
        {
          id: 1,
          provider: "claude",
          project_path: "/work/alpha-project",
          name: "alpha-project",
          storage_type: "jsonl",
          session_count: 1,
          message_count: 1,
          last_modified: null,
          machine_id: "m1",
          machine_hostname: "host-alpha",
        },
      ],
    });

    render(<ArchiveBrowser config={CFG} />);

    // Both tabs exist and are real, focusable buttons.
    const journalTab = await screen.findByTestId("archive-tab-journal");
    const browseTab = await screen.findByTestId("archive-tab-browse");
    expect(journalTab.tagName).toBe("BUTTON");
    expect(browseTab.tagName).toBe("BUTTON");

    // Journal is the default landing view: a journal entry renders with no click.
    await screen.findByText("Landing headline");

    // The global search bar stays visible across both views.
    expect(screen.getByTestId("archive-search-input")).toBeInTheDocument();

    // Activating Browse shows the existing three panes with their current copy.
    fireEvent.click(browseTab);
    await screen.findByText("settings.archiveHub.browser.projects.title");
    expect(
      screen.getByText("settings.archiveHub.browser.sessions.title")
    ).toBeInTheDocument();
    // Projects loaded through the unchanged Browse path.
    await screen.findByText("alpha-project");
  });
});

// ============================================================================
// AC2: day-timeline grouping, newest-day-first, relative recent-day label
// ============================================================================

describe("AC2 — day grouping, newest-first, relative label", () => {
  it("groups entries by entry_date under humanized, newest-first day headers", async () => {
    const now = new Date();
    const yest = new Date(now);
    yest.setDate(now.getDate() - 1);
    const old = new Date(now);
    old.setDate(now.getDate() - 10);
    const Y = isoDate(yest);
    const OLD = isoDate(old);

    installRouter({
      // Newest-first, as the hub browse endpoint returns them.
      journalEntries: () => [
        makeEntry({
          entry_date: Y,
          project_path: "/work/alpha-project",
          headline: "Alpha work",
        }),
        makeEntry({
          entry_date: Y,
          project_path: "/work/beta",
          headline: "Beta work",
        }),
        makeEntry({
          entry_date: OLD,
          project_path: "/work/alpha-project",
          headline: "Old work",
        }),
      ],
    });

    render(<ArchiveBrowser config={CFG} />);

    await screen.findByText("Alpha work");
    const headers = screen.getAllByTestId("journal-day-header");
    const cards = screen.getAllByTestId("journal-entry-card");

    // Two distinct entry_dates → two day groups; three entries → three cards
    // (one card per project worked on that day).
    expect(headers).toHaveLength(2);
    expect(cards).toHaveLength(3);

    // Newest day first: the recent-day header precedes the older-day header.
    expect(
      headers[0].compareDocumentPosition(headers[1]) &
        Node.DOCUMENT_POSITION_FOLLOWING
    ).toBeTruthy();

    // Same-day entries precede the older day's entry in the feed.
    const alpha = screen.getByText("Alpha work");
    const oldWork = screen.getByText("Old work");
    expect(
      alpha.compareDocumentPosition(oldWork) & Node.DOCUMENT_POSITION_FOLLOWING
    ).toBeTruthy();

    // The most-recent CLOSED day carries a humanized relative label — not the
    // raw entry_date string — and it differs from the older day's absolute label.
    const recentHeader = (headers[0].textContent ?? "").trim();
    const olderHeader = (headers[1].textContent ?? "").trim();
    expect(recentHeader).not.toContain(Y);
    expect(recentHeader).not.toBe(olderHeader);
    expect(recentHeader.length).toBeGreaterThan(0);
  });
});

// ============================================================================
// AC3: date picker jump + project filter refetch/narrow
// ============================================================================

describe("AC3 — date picker and project filter", () => {
  it("date picker refetches with from/to and project filter narrows the feed", async () => {
    // --- date jump ---------------------------------------------------------
    const calls = installRouter({
      journalEntries: (p) => {
        if (p.get("from") === "2026-06-15") {
          return [
            makeEntry({
              entry_date: "2026-06-15",
              headline: "June entry",
              project_path: "/work/june",
            }),
          ];
        }
        return [
          makeEntry({
            entry_date: "2026-07-10",
            headline: "Default entry",
            project_path: "/work/alpha-project",
          }),
        ];
      },
    });

    render(<ArchiveBrowser config={CFG} />);
    await screen.findByText("Default entry");

    const picker = await screen.findByTestId("journal-date-picker");
    fireEvent.change(picker, { target: { value: "2026-06-15" } });

    await waitFor(() => {
      const last = calls.journalEntries.at(-1);
      expect(last?.get("from")).toBe("2026-06-15");
      expect(last?.get("to")).toBe("2026-06-15");
      // A jump resets pagination.
      expect(last?.get("offset") ?? "0").toBe("0");
    });
    await screen.findByText("June entry");

    // --- project filter ----------------------------------------------------
    cleanup();
    const calls2 = installRouter({
      journalEntries: (p) => {
        const proj = p.get("project");
        const all = [
          makeEntry({
            entry_date: "2026-07-10",
            headline: "Alpha entry",
            project_path: "/work/alpha-project",
          }),
          makeEntry({
            entry_date: "2026-07-10",
            headline: "Beta entry",
            project_path: "/work/beta",
          }),
        ];
        return proj ? all.filter((e) => e.project_path === proj) : all;
      },
    });

    render(<ArchiveBrowser config={CFG} />);
    await screen.findByText("Alpha entry");
    await screen.findByText("Beta entry");

    const filter = (await screen.findByTestId(
      "journal-project-filter"
    )) as HTMLSelectElement;
    const option = Array.from(filter.querySelectorAll("option")).find(
      (o) => o.value.includes("/work/alpha-project") || o.textContent?.includes("alpha")
    );
    expect(option).toBeTruthy();
    fireEvent.change(filter, { target: { value: option!.value } });

    await waitFor(() => {
      const last = calls2.journalEntries.at(-1);
      expect(last?.get("project")).toBe(option!.value);
    });
    await screen.findByText("Alpha entry");
    await waitFor(() =>
      expect(screen.queryByText("Beta entry")).not.toBeInTheDocument()
    );
  });
});

// ============================================================================
// AC4: rich card at rest, expand reveals details, lazy session resolution
// ============================================================================

describe("AC4 — entry card at rest / expanded, lazy session labels", () => {
  it("shows meta at rest and resolves sessions only on first expand (cached)", async () => {
    const listSessionsFor = (p: URLSearchParams) => {
      // Sessions for the requested project; lazily resolved labels.
      void p;
      return [
        {
          id: 201,
          provider: "claude",
          session_id: "s-201",
          summary: "Session Alpha",
          file_path: null,
          entrypoint: null,
          message_count: 12,
          first_message_time: null,
          last_message_time: null,
          has_tool_use: false,
          has_errors: false,
          project_name: "alpha",
          project_path: "/work/alpha-project",
          machine_hostname: "host-a",
        },
        {
          id: 202,
          provider: "claude",
          session_id: "s-202",
          summary: "Session Beta",
          file_path: null,
          entrypoint: null,
          message_count: 5,
          first_message_time: null,
          last_message_time: null,
          has_tool_use: false,
          has_errors: false,
          project_name: "alpha",
          project_path: "/work/alpha-project",
          machine_hostname: "host-a",
        },
      ];
    };

    const calls = installRouter({
      journalEntries: () => [
        makeEntry({
          entry_date: "2026-07-10",
          project_path: "/work/alpha-project",
          headline: "Refactor auth layer",
          summary: "Reworked the token cache and simplified the guard.",
          topics: ["auth", "cache"],
          open_questions: ["Should we expire tokens sooner?"],
          session_ids: [201, 202],
          model: "claude-opus-model",
        }),
        // A second card for the SAME project, different day, to prove caching.
        makeEntry({
          entry_date: "2026-07-09",
          project_path: "/work/alpha-project",
          headline: "Earlier work",
          summary: "Groundwork.",
          topics: ["setup"],
          open_questions: [],
          session_ids: [201],
          model: "claude-opus-model",
        }),
      ],
      sessions: listSessionsFor,
    });

    render(<ArchiveBrowser config={CFG} />);

    await screen.findByText("Refactor auth layer");
    const cards = screen.getAllByTestId("journal-entry-card");
    const card = within(cards[0]);

    // At rest: project (basename, full path on hover), model, headline,
    // summary, topic chips are visible (webapp-ux-readability change).
    expect(card.getByText("alpha-project")).toBeInTheDocument();
    expect(card.getByText("alpha-project")).toHaveAttribute(
      "title",
      "/work/alpha-project"
    );
    expect(card.getByText("claude-opus-model")).toBeInTheDocument();
    expect(card.getByText(/Reworked the token cache/)).toBeInTheDocument();
    expect(card.getByText("auth")).toBeInTheDocument();
    expect(card.getByText("cache")).toBeInTheDocument();
    // Session count (2) shown at rest.
    expect(cards[0].textContent).toMatch(/\b2\b/);

    // No lazy sessions request at feed render, and no drill-down UI at rest.
    expect(calls.listSessions).toHaveLength(0);
    expect(card.queryByText("Session Alpha")).not.toBeInTheDocument();
    expect(card.queryAllByTestId("journal-session-link")).toHaveLength(0);
    expect(
      card.queryByText(/Should we expire tokens sooner/)
    ).not.toBeInTheDocument();

    // Expand the first card: open questions + one link per session id appear,
    // and exactly one sessions request fires.
    fireEvent.click(card.getByTestId("journal-entry-toggle"));

    await card.findByText(/Should we expire tokens sooner/);
    await card.findByText("Session Alpha");
    expect(card.getByText("Session Beta")).toBeInTheDocument();
    expect(card.getAllByTestId("journal-session-link")).toHaveLength(2);
    await waitFor(() => expect(calls.listSessions).toHaveLength(1));

    // Expand the second card (same project) — labels come from cache, no new
    // sessions request.
    fireEvent.click(within(cards[1]).getByTestId("journal-entry-toggle"));
    await within(cards[1]).findAllByTestId("journal-session-link");
    // Give any (erroneous) refetch a chance to land, then assert still-one.
    await Promise.resolve();
    expect(calls.listSessions).toHaveLength(1);
  });
});

// ============================================================================
// AC5: session link switches to Browse and loads that session's messages
// ============================================================================

describe("AC5 — session link drills into Browse", () => {
  it("activating a session link opens Browse and fetches that session's messages", async () => {
    const calls = installRouter({
      journalEntries: () => [
        makeEntry({
          entry_date: "2026-07-10",
          project_path: "/work/alpha-project",
          headline: "Drill target",
          session_ids: [101],
        }),
      ],
      sessions: () => [
        {
          id: 101,
          provider: "claude",
          session_id: "s-101",
          summary: "Linked session",
          file_path: null,
          entrypoint: null,
          message_count: 3,
          first_message_time: null,
          last_message_time: null,
          has_tool_use: false,
          has_errors: false,
          project_name: "alpha",
          project_path: "/work/alpha-project",
          machine_hostname: "host-a",
        },
      ],
      messages: { rows: [], totalCount: 0 },
    });

    render(<ArchiveBrowser config={CFG} />);

    await screen.findByText("Drill target");
    const card = within(screen.getAllByTestId("journal-entry-card")[0]);
    fireEvent.click(card.getByTestId("journal-entry-toggle"));

    const link = (await card.findAllByTestId("journal-session-link"))[0];
    fireEvent.click(link);

    // The existing message-fetch path is used: GET /v1/sessions/101/messages.
    await waitFor(() => {
      expect(
        calls.sessionMessages.some((c) => c.pathname === "/v1/sessions/101/messages")
      ).toBe(true);
    });

    // The view switched to Browse (its three panes are now on screen).
    await screen.findByText("settings.archiveHub.browser.projects.title");
    expect(
      screen.getByText("settings.archiveHub.browser.sessions.title")
    ).toBeInTheDocument();
  });
});

// ============================================================================
// AC6: search journal section above message hits; anchors journal on activate;
//      tolerates a response with no journal block
// ============================================================================

describe("AC6 — journal hits in search results", () => {
  it("renders a journal section above message hits and anchors on activate", async () => {
    const messageHit = {
      message_id: 1,
      message_key: "mk1",
      uuid: "u1",
      provider: "claude",
      session_pk: 9,
      session_id: "sess-msg-1",
      session_summary: "a session",
      project_name: "alpha",
      project_path: "/work/alpha-project",
      machine_hostname: "host-a",
      timestamp: "2026-07-10T00:00:00Z",
      message_type: "assistant",
      role: "assistant",
      model: "claude-opus",
      snippet: "the needle in the haystack",
      rank: 0.9,
    };
    const journalHit = {
      entry_date: "2026-07-08",
      project_path: "/work/alpha-project",
      headline: "Distilled journal answer",
      summary: "Everything about the needle.",
      topics: ["needle"],
      open_questions: [],
      session_ids: [1],
      model: "claude-opus",
      generated_at: "2026-07-08T12:00:00Z",
      rank: 0.99,
    };

    const calls = installRouter({
      journalEntries: (p) => {
        if (p.get("from") === "2026-07-08") {
          return [
            makeEntry({
              entry_date: "2026-07-08",
              headline: "Anchored day entry",
              project_path: "/work/alpha-project",
            }),
          ];
        }
        return [
          makeEntry({ entry_date: "2026-07-10", headline: "Feed entry" }),
        ];
      },
      search: () => ({
        results: [messageHit],
        limit: 20,
        offset: 0,
        journal: [journalHit],
      }),
    });

    render(<ArchiveBrowser config={CFG} />);
    await screen.findByText("Feed entry");

    const input = screen.getByTestId("archive-search-input");
    fireEvent.change(input, { target: { value: "needle" } });
    fireEvent.submit(input.closest("form") ?? input);

    // Journal hit renders (headline, date, project) inside its own section...
    const section = await screen.findByTestId("journal-search-section");
    expect(within(section).getByText("Distilled journal answer")).toBeInTheDocument();
    expect(section.textContent).toContain("/work/alpha-project");
    expect(section.textContent).toContain("2026-07-08");

    // ...above the message hits.
    const msgHitEl = await screen.findByText("the needle in the haystack");
    expect(
      section.compareDocumentPosition(msgHitEl) &
        Node.DOCUMENT_POSITION_FOLLOWING
    ).toBeTruthy();

    // Activating the journal hit opens the Journal view anchored at its date.
    const hit = within(section).getByTestId("journal-search-hit");
    fireEvent.click(hit);

    await waitFor(() => {
      const anchored = calls.journalEntries.find(
        (p) => p.get("from") === "2026-07-08" && p.get("to") === "2026-07-08"
      );
      expect(anchored).toBeTruthy();
    });
    await screen.findByText("Anchored day entry");

    // A search response WITHOUT a journal block still renders message hits and
    // shows no journal section.
    cleanup();
    installRouter({
      journalEntries: () => [makeEntry({ headline: "Feed entry 2" })],
      search: () => ({ results: [messageHit], limit: 20, offset: 0 }),
    });
    render(<ArchiveBrowser config={CFG} />);
    await screen.findByText("Feed entry 2");
    const input2 = screen.getByTestId("archive-search-input");
    fireEvent.change(input2, { target: { value: "needle" } });
    fireEvent.submit(input2.closest("form") ?? input2);

    await screen.findByText("the needle in the haystack");
    expect(screen.queryByTestId("journal-search-section")).not.toBeInTheDocument();
  });
});

// ============================================================================
// AC7: empty range notice + failing fetch error state (tabs survive)
// ============================================================================

describe("AC7 — empty and error states", () => {
  it("shows the not-yet-distilled notice on empty and an error state on failure", async () => {
    // Empty range.
    installRouter({ journalEntries: () => [] });
    render(<ArchiveBrowser config={CFG} />);

    const empty = await screen.findByTestId("journal-empty");
    // The notice is a localized string (resolves through an i18n key).
    expect(empty.textContent ?? "").toMatch(/settings\.archiveHub\.journal\./);
    expect(screen.queryAllByTestId("journal-entry-card")).toHaveLength(0);

    // Failing journal fetch → error state, and the tab switcher still works.
    cleanup();
    installRouter({
      journalEntriesResponse: { ok: false, status: 500, body: { error: "boom" } },
      projects: [],
    });
    render(<ArchiveBrowser config={CFG} />);

    await screen.findByTestId("journal-error");
    const browseTab = screen.getByTestId("archive-tab-browse");
    fireEvent.click(browseTab);
    await screen.findByText("settings.archiveHub.browser.projects.title");
  });
});

// ============================================================================
// AC8: unified type scale — content 15 > tool header 14 > tool id 12 (mono)
// (sizes re-tuned by the webapp-ux-readability change)
// ============================================================================

describe("AC8 — unified archive type scale", () => {
  it("message text is 15px, tool-card headers 14px, tool ids 12px mono", () => {
    // Tool card: header title + tool id badge.
    render(
      <ExpandKeyProvider value="ac8-tool">
        <ToolUseCard
          title="Bash"
          icon={<span data-testid="ac8-icon" />}
          variant="code"
          toolId="toolu_abc"
        >
          <span>body</span>
        </ToolUseCard>
      </ExpandKeyProvider>
    );

    // Tool-card header carries an EXPLICIT 14px size (no size-class-less header
    // that inherits the 16px default).
    const headerTitle = screen.getByText(/^Bash/);
    const headerPx = effectiveFontSizePx(headerTitle);
    expect(headerPx).toBe(14);

    // Tool id: 12px monospace.
    const toolId = screen.getByText(/toolu_abc/);
    const idPx = effectiveFontSizePx(toolId);
    expect(idPx).toBe(12);
    expect(hasClassUp(toolId, "font-mono")).toBe(true);

    cleanup();

    // Conversation message text renders at the 15px scale.
    render(
      <ExpandKeyProvider value="ac8-msg">
        <MessageContentDisplay content="hello archive" messageType="user" />
      </ExpandKeyProvider>
    );
    const contentPx = effectiveFontSizePx(screen.getByText("hello archive"));
    expect(contentPx).toBe(15);

    // Content outranks chrome: 15 > 14 > 12.
    expect(contentPx as number).toBeGreaterThan(headerPx as number);
    expect(headerPx as number).toBeGreaterThan(idPx as number);
  });
});

// ============================================================================
// AC9: widened Browse panes; humanized timestamps + locale-formatted counts
// ============================================================================

describe("AC9 — widened Browse panes and humanized list rows", () => {
  it("uses 240/320px panes and humanized/locale-formatted session rows", async () => {
    const rawIso = "2020-01-02T03:04:05Z";
    const count = 12345;
    installRouter({
      journalEntries: () => [makeEntry({ headline: "Feed" })],
      projects: [
        {
          id: 1,
          provider: "claude",
          project_path: "/work/alpha-project",
          name: "alpha-project",
          storage_type: "jsonl",
          session_count: 1,
          message_count: count,
          last_modified: null,
          machine_id: "m1",
          machine_hostname: "host-a",
        },
      ],
      sessions: () => [
        {
          id: 10,
          provider: "claude",
          session_id: "s-10",
          summary: "widths session",
          file_path: null,
          entrypoint: null,
          message_count: count,
          first_message_time: null,
          last_message_time: rawIso,
          has_tool_use: false,
          has_errors: false,
          project_name: "alpha-project",
          project_path: "/work/alpha-project",
          machine_hostname: "host-a",
        },
      ],
    });

    render(<ArchiveBrowser config={CFG} />);
    fireEvent.click(await screen.findByTestId("archive-tab-browse"));

    // Projects pane widened to 240px (md:w-60), sessions pane to 320px
    // (md:w-80); below `md` the panes stack full-width (webapp-ux-readability).
    const projectsPane = screen
      .getByText("settings.archiveHub.browser.projects.title")
      .closest(".overflow-y-auto") as HTMLElement;
    const sessionsPane = screen
      .getByText("settings.archiveHub.browser.sessions.title")
      .closest(".overflow-y-auto") as HTMLElement;
    expect((projectsPane.getAttribute("class") ?? "").split(/\s+/)).toContain("md:w-60");
    expect((sessionsPane.getAttribute("class") ?? "").split(/\s+/)).toContain("md:w-80");

    // Open a project so its sessions render.
    fireEvent.click(await screen.findByText("alpha-project"));
    const sessionRow = await screen.findByText("widths session");
    const row = sessionRow.closest("button") as HTMLElement;

    // Locale-formatted count (matches the environment's default Intl grouping).
    const formatted = new Intl.NumberFormat().format(count);
    expect(row.textContent ?? "").toContain(formatted);
    if (formatted !== String(count)) {
      expect(row.textContent ?? "").not.toContain(String(count));
    }
    // Timestamp humanized — the raw ISO string is gone.
    expect(row.textContent ?? "").not.toContain(rawIso);
    expect(row.textContent ?? "").not.toContain("T03:04:05");
  });
});

// ============================================================================
// AC10: every new journal string localized in all five locales
// ============================================================================

describe("AC10 — journal strings localized in all five locales", () => {
  it("settings.archiveHub.journal.* keys exist and match across en/ko/ja/zh-CN/zh-TW", async () => {
    const PREFIX = "settings.archiveHub.journal.";
    const journalKeys = (o: Record<string, unknown>) =>
      Object.keys(o)
        .filter((k) => k.startsWith(PREFIX))
        .sort();

    const en = journalKeys(enSettings as Record<string, unknown>);
    const ko = journalKeys(koSettings as Record<string, unknown>);
    const ja = journalKeys(jaSettings as Record<string, unknown>);
    const zhCN = journalKeys(zhCNSettings as Record<string, unknown>);
    const zhTW = journalKeys(zhTWSettings as Record<string, unknown>);

    // A non-trivial set of new journal keys exists...
    expect(en.length).toBeGreaterThanOrEqual(5);
    // ...and every locale carries exactly the same journal keys (no drift).
    expect(ko).toEqual(en);
    expect(ja).toEqual(en);
    expect(zhCN).toEqual(en);
    expect(zhTW).toEqual(en);

    // And a new component actually routes a user-facing string through those
    // keys (the empty notice is an i18n key, not a raw literal).
    installRouter({ journalEntries: () => [] });
    render(<ArchiveBrowser config={CFG} />);
    const empty = await screen.findByTestId("journal-empty");
    expect(empty.textContent ?? "").toMatch(/settings\.archiveHub\.journal\./);
  });
});
