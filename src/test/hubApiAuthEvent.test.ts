import { describe, it, expect, vi, afterEach } from "vitest";
import { HUB_UNAUTHORIZED_EVENT, hubApi, type HubConfig } from "../services/hubApi";

/**
 * A read that ends the browser session brings the login form back (the gate
 * listens for HUB_UNAUTHORIZED_EVENT). A 429 from the hub's failed-credential
 * limit counts as such when it carries the auth-failure marker (security
 * fe3fd22 F3); any other 429 or error does not.
 */

const config: HubConfig = { url: "https://hub.example", token: "" };

function stubFetch(status: number, headers: Record<string, string> = {}) {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => ({
      ok: false,
      status,
      headers: new Headers(headers),
      json: async () => ({}),
    })) as unknown as typeof fetch
  );
}

async function eventFired(): Promise<boolean> {
  const listener = vi.fn();
  window.addEventListener(HUB_UNAUTHORIZED_EVENT, listener);
  await hubApi.listProjects(config, { limit: 1 }).catch(() => undefined);
  window.removeEventListener(HUB_UNAUTHORIZED_EVENT, listener);
  return listener.mock.calls.length > 0;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("hubApi session-ended signal", () => {
  it("fires on a 401", async () => {
    stubFetch(401);
    expect(await eventFired()).toBe(true);
  });

  it("fires on a 429 marked as a credential failure", async () => {
    stubFetch(429, { "x-cchv-auth-failed": "1", "Retry-After": "600" });
    expect(await eventFired()).toBe(true);
  });

  it("does not fire on an unmarked 429 or a 503", async () => {
    stubFetch(429);
    expect(await eventFired()).toBe(false);
    stubFetch(503);
    expect(await eventFired()).toBe(false);
  });
});
