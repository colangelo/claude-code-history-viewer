import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ConnectGate } from "../components/ArchiveBrowser/ConnectGate";
import {
  loadStoredHubConfig,
  clearStoredHubConfig,
} from "../components/ArchiveBrowser/hubConfigStorage";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, params?: Record<string, unknown>) =>
      params?.error != null
        ? `${key}:${String(params.error)}`
        : params?.seconds != null
          ? `${key}:${String(params.seconds)}`
          : key,
  }),
}));

const { mockHubApi, MockHubHttpError } = vi.hoisted(() => {
  class MockHubHttpError extends Error {
    constructor(
      readonly status: number,
      readonly path: string,
      readonly retryAfterSecs?: number
    ) {
      super(`hub request to ${path} failed: ${status}`);
    }
  }
  return {
    mockHubApi: {
      listProjects: vi.fn(),
      login: vi.fn(),
      logout: vi.fn(),
    },
    MockHubHttpError,
  };
});

vi.mock("../services/hubApi", () => ({
  hubApi: mockHubApi,
  HubHttpError: MockHubHttpError,
  HUB_UNAUTHORIZED_EVENT: "cchv:hub-unauthorized",
}));

vi.mock("../components/ArchiveBrowser/index", () => ({
  ArchiveBrowser: ({ config }: { config: { url: string } }) => (
    <div data-testid="archive-browser">{config.url}</div>
  ),
}));

// ThemeToggle needs a ThemeProvider; the gate's own behavior doesn't.
vi.mock("../components/ArchiveBrowser/ThemeToggle", () => ({
  ThemeToggle: () => null,
}));

const STORAGE_KEY = "cchv.archiveWeb.hubConfig";

function fillAndSubmit(url: string, token: string) {
  fireEvent.change(screen.getByLabelText("archive.web.urlLabel"), {
    target: { value: url },
  });
  fireEvent.change(screen.getByLabelText("archive.web.tokenLabel"), {
    target: { value: token },
  });
  fireEvent.click(screen.getByRole("button", { name: "archive.web.login" }));
}

beforeEach(() => {
  localStorage.clear();
  vi.clearAllMocks();
});

describe("ConnectGate", () => {
  it("auto-connects same-origin when the host authenticates the probe", async () => {
    mockHubApi.listProjects.mockResolvedValueOnce([]);
    render(<ConnectGate />);
    await waitFor(() =>
      expect(screen.getByTestId("archive-browser")).toHaveTextContent(
        window.location.origin
      )
    );
    expect(mockHubApi.listProjects).toHaveBeenCalledWith(
      { url: window.location.origin, token: "" },
      { limit: 1 }
    );
    // Auto-connect persists nothing — revoking host auth must take effect.
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it("shows the connect form when nothing is stored and the same-origin probe fails", async () => {
    mockHubApi.listProjects.mockRejectedValueOnce(new Error("401"));
    render(<ConnectGate />);
    await waitFor(() =>
      expect(screen.getByLabelText("archive.web.urlLabel")).toBeInTheDocument()
    );
    expect(screen.queryByTestId("archive-browser")).not.toBeInTheDocument();
  });

  it("on another origin holds the token in memory only and mounts the browser", async () => {
    mockHubApi.listProjects
      .mockRejectedValueOnce(new Error("401")) // same-origin auto-probe
      .mockResolvedValueOnce([]); // manual connect
    render(<ConnectGate />);
    await waitFor(() =>
      expect(screen.getByLabelText("archive.web.urlLabel")).toBeInTheDocument()
    );
    fillAndSubmit("http://hub.example:8787", "tok-1");

    await waitFor(() =>
      expect(screen.getByTestId("archive-browser")).toHaveTextContent(
        "http://hub.example:8787"
      )
    );
    expect(mockHubApi.listProjects).toHaveBeenLastCalledWith(
      { url: "http://hub.example:8787", token: "tok-1" },
      { limit: 1 }
    );
    expect(mockHubApi.login).not.toHaveBeenCalled();
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it("on the page's own hub trades the token for the session cookie and stores no token", async () => {
    mockHubApi.listProjects
      .mockRejectedValueOnce(new Error("401")) // same-origin auto-probe
      .mockResolvedValueOnce([]); // probe on the new cookie
    mockHubApi.login.mockResolvedValueOnce(undefined);
    render(<ConnectGate />);
    await waitFor(() =>
      expect(screen.getByLabelText("archive.web.urlLabel")).toBeInTheDocument()
    );
    expect(screen.getByLabelText("archive.web.urlLabel")).toHaveValue(window.location.origin);
    fillAndSubmit(window.location.origin, "reader-token");

    await waitFor(() => expect(screen.getByTestId("archive-browser")).toBeInTheDocument());
    expect(mockHubApi.login).toHaveBeenCalledWith(window.location.origin, "reader-token");
    expect(mockHubApi.listProjects).toHaveBeenLastCalledWith(
      { url: window.location.origin, token: "" },
      { limit: 1 }
    );
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!)).toEqual({
      v: 1,
      url: window.location.origin,
      token: "",
    });
    expect(JSON.stringify(localStorage)).not.toContain("reader-token");
  });

  it("offers the form to a password manager", async () => {
    mockHubApi.listProjects.mockRejectedValueOnce(new Error("401"));
    const { container } = render(<ConnectGate />);
    await waitFor(() =>
      expect(screen.getByLabelText("archive.web.tokenLabel")).toBeInTheDocument()
    );
    expect(screen.getByLabelText("archive.web.tokenLabel")).toHaveAttribute(
      "autocomplete",
      "current-password"
    );
    expect(container.querySelector('input[autocomplete="username"]')).toHaveValue("cchv-reader");
  });

  it("returns to the form, saying why, with the token field focused and described", async () => {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ v: 1, url: window.location.origin, token: "" })
    );
    render(<ConnectGate />);
    expect(screen.getByTestId("archive-browser")).toBeInTheDocument();
    act(() => {
      window.dispatchEvent(new Event("cchv:hub-unauthorized"));
    });
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("archive.web.sessionEnded")
    );
    expect(screen.queryByTestId("archive-browser")).not.toBeInTheDocument();
    const field = screen.getByLabelText("archive.web.tokenLabel");
    await waitFor(() => expect(field).toHaveFocus());
    expect(field).toHaveAttribute("aria-describedby", screen.getByRole("status").id);
  });

  it("a 429 from the failed-credential limit says how long to wait", async () => {
    mockHubApi.listProjects.mockRejectedValueOnce(new Error("401"));
    mockHubApi.login.mockRejectedValueOnce(new MockHubHttpError(429, "/v1/login", 42));
    render(<ConnectGate />);
    await waitFor(() =>
      expect(screen.getByLabelText("archive.web.urlLabel")).toBeInTheDocument()
    );
    fillAndSubmit(window.location.origin, "reader-token");
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("archive.web.tooManyTries:42")
    );
  });

  it("a login failure shows the error and stores nothing", async () => {
    mockHubApi.listProjects.mockRejectedValueOnce(new Error("401"));
    mockHubApi.login.mockRejectedValueOnce(new MockHubHttpError(401, "/v1/login"));
    render(<ConnectGate />);
    await waitFor(() =>
      expect(screen.getByLabelText("archive.web.urlLabel")).toBeInTheDocument()
    );
    fillAndSubmit(window.location.origin, "machine-token");
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("archive.web.tokenRejected")
    );
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it("deletes a token stored by an older build", () => {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ v: 1, url: "http://stored:8787", token: "old-token" })
    );
    expect(loadStoredHubConfig()).toBeNull();
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it("shows the error and persists nothing when the probe fails", async () => {
    mockHubApi.listProjects
      .mockRejectedValueOnce(new Error("401")) // same-origin auto-probe
      .mockRejectedValueOnce(new Error("hub request to /v1/projects failed: 401"));
    render(<ConnectGate />);
    await waitFor(() =>
      expect(screen.getByLabelText("archive.web.urlLabel")).toBeInTheDocument()
    );
    fillAndSubmit("http://hub.example:8787", "bad-token");

    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent(
        "archive.web.connectFailed:hub request to /v1/projects failed: 401"
      )
    );
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
    expect(screen.queryByTestId("archive-browser")).not.toBeInTheDocument();
  });

  it("skips the form on a returning visit with a stored hub URL", () => {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ v: 1, url: "http://stored:8787", token: "" })
    );
    render(<ConnectGate />);
    expect(screen.getByTestId("archive-browser")).toHaveTextContent(
      "http://stored:8787"
    );
    expect(mockHubApi.listProjects).not.toHaveBeenCalled();
  });

  it("disconnect ends the session, clears storage and returns to the form", async () => {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ v: 1, url: "http://stored:8787", token: "" })
    );
    mockHubApi.logout.mockResolvedValueOnce(undefined);
    render(<ConnectGate />);
    fireEvent.click(
      screen.getByRole("button", { name: "archive.web.logout" })
    );
    expect(mockHubApi.logout).toHaveBeenCalledWith("http://stored:8787");
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
    await waitFor(() =>
      expect(screen.getByLabelText("archive.web.urlLabel")).toBeInTheDocument()
    );
  });

  it("shows the build version in both the connected header and the form", async () => {
    // Connected state (stored config).
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ v: 1, url: "http://stored:8787", token: "" })
    );
    const { unmount } = render(<ConnectGate />);
    expect(screen.getByTestId("app-version").textContent).toMatch(
      /^\(v\d+\.\d+\.\d+/
    );
    unmount();

    // Disconnected form.
    localStorage.clear();
    mockHubApi.listProjects.mockRejectedValueOnce(new Error("401"));
    render(<ConnectGate />);
    await waitFor(() =>
      expect(screen.getByLabelText("archive.web.urlLabel")).toBeInTheDocument()
    );
    expect(screen.getByTestId("app-version").textContent).toMatch(
      /^\(v\d+\.\d+\.\d+/
    );
  });

  it("connects without a token on a live session", async () => {
    mockHubApi.listProjects
      .mockRejectedValueOnce(new Error("401")) // same-origin auto-probe
      .mockResolvedValueOnce([]); // manual tokenless connect
    render(<ConnectGate />);
    await waitFor(() =>
      expect(screen.getByLabelText("archive.web.urlLabel")).toBeInTheDocument()
    );
    fillAndSubmit("https://hub.tailnet:8788", "");

    await waitFor(() =>
      expect(screen.getByTestId("archive-browser")).toHaveTextContent(
        "https://hub.tailnet:8788"
      )
    );
    expect(mockHubApi.listProjects).toHaveBeenLastCalledWith(
      { url: "https://hub.tailnet:8788", token: "" },
      { limit: 1 }
    );
    // The URL persists with an empty token and round-trips.
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!)).toEqual({
      v: 1,
      url: "https://hub.tailnet:8788",
      token: "",
    });
    expect(loadStoredHubConfig()).toEqual({
      url: "https://hub.tailnet:8788",
      token: "",
    });
  });

  it("ignores malformed or wrong-version stored payloads", () => {
    localStorage.setItem(STORAGE_KEY, "not json");
    expect(loadStoredHubConfig()).toBeNull();
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ v: 2, url: "x", token: "y" }));
    expect(loadStoredHubConfig()).toBeNull();
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ v: 1, url: "", token: "y" }));
    expect(loadStoredHubConfig()).toBeNull();
    clearStoredHubConfig();
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
  });
});
