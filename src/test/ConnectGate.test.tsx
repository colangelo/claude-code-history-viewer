import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ConnectGate } from "../components/ArchiveBrowser/ConnectGate";
import {
  loadStoredHubConfig,
  clearStoredHubConfig,
} from "../components/ArchiveBrowser/hubConfigStorage";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, params?: Record<string, unknown>) =>
      params?.error != null ? `${key}:${String(params.error)}` : key,
  }),
}));

const { mockHubApi } = vi.hoisted(() => ({
  mockHubApi: {
    listProjects: vi.fn(),
  },
}));

vi.mock("../services/hubApi", () => ({
  hubApi: mockHubApi,
}));

vi.mock("../components/ArchiveBrowser/index", () => ({
  ArchiveBrowser: ({ config }: { config: { url: string } }) => (
    <div data-testid="archive-browser">{config.url}</div>
  ),
}));

const STORAGE_KEY = "cchv.archiveWeb.hubConfig";

function fillAndSubmit(url: string, token: string) {
  fireEvent.change(screen.getByLabelText("archive.web.urlLabel"), {
    target: { value: url },
  });
  fireEvent.change(screen.getByLabelText("archive.web.tokenLabel"), {
    target: { value: token },
  });
  fireEvent.click(screen.getByRole("button", { name: "archive.web.connect" }));
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

  it("persists the config and mounts the browser when the probe succeeds", async () => {
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
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!)).toEqual({
      v: 1,
      url: "http://hub.example:8787",
      token: "tok-1",
    });
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

  it("skips the form on a returning visit with a stored config", () => {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ v: 1, url: "http://stored:8787", token: "tok" })
    );
    render(<ConnectGate />);
    expect(screen.getByTestId("archive-browser")).toHaveTextContent(
      "http://stored:8787"
    );
    expect(mockHubApi.listProjects).not.toHaveBeenCalled();
  });

  it("disconnect clears storage and returns to the form", async () => {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ v: 1, url: "http://stored:8787", token: "tok" })
    );
    render(<ConnectGate />);
    fireEvent.click(
      screen.getByRole("button", { name: "archive.web.disconnect" })
    );
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
    await waitFor(() =>
      expect(screen.getByLabelText("archive.web.urlLabel")).toBeInTheDocument()
    );
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
