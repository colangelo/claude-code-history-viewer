/**
 * @fileoverview The hub session row leads with conversation items, not records (#41).
 * `message_count` counts every archived row, ~half of them content-less state records.
 */
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { SessionsPane } from "../components/ArchiveBrowser/SessionsPane";
import { sessionHeadlineCount, type HubSession } from "../services/hubApi";

vi.mock("react-i18next", async () => {
  const actual =
    await vi.importActual<typeof import("react-i18next")>("react-i18next");
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, opts?: { count?: number }) =>
        opts?.count != null ? `${key}|${opts.count}` : key,
    }),
  };
});

function session(over: Partial<HubSession>): HubSession {
  return {
    id: 1,
    provider: "claude",
    session_id: "s-1",
    summary: "a session",
    file_path: null,
    entrypoint: null,
    message_count: 15,
    first_message_time: null,
    last_message_time: null,
    has_tool_use: false,
    has_errors: false,
    project_name: null,
    project_path: null,
    machine_hostname: "host",
    ...over,
  };
}

function renderPane(sessions: HubSession[]) {
  return render(
    <SessionsPane
      sessions={sessions}
      hasSelection
      openSessionRef={null}
      isLoading={false}
      error={null}
      hidden={false}
      onBackToProjects={() => {}}
      onOpenSession={() => {}}
    />
  );
}

describe("SessionsPane count headline", () => {
  it("renders conversation_count, with records in the tooltip", () => {
    renderPane([session({ conversation_count: 7, message_count: 15 })]);
    const line = screen.getByText(/messageCountUnit/);
    expect(line.textContent).toContain("7");
    expect(line.textContent).not.toContain("15");
    expect(line.getAttribute("title")).toBe(
      "settings.archiveHub.browser.sessions.recordsTooltip|15"
    );
  });

  it("falls back to message_count against a hub that predates the field", () => {
    renderPane([session({ message_count: 15 })]);
    expect(screen.getByText(/messageCountUnit/).textContent).toContain("15");
  });
});

describe("sessionHeadlineCount", () => {
  it("prefers conversation_count, including a legitimate zero", () => {
    expect(
      sessionHeadlineCount({ message_count: 4, conversation_count: 0 })
    ).toBe(0);
    expect(sessionHeadlineCount({ message_count: 4 })).toBe(4);
  });
});
