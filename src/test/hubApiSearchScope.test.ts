import { describe, it, expect, vi, afterEach } from "vitest";
import { hubApi, type HubConfig } from "../services/hubApi";

/**
 * `GET /v1/search` runs two independent legs (message FTS + journal FTS) and
 * `scope` selects which. Each caller reads only one of the two blocks, so
 * asking for both would make the hub pay for a result the client discards —
 * these tests pin the narrowing so a refactor can't silently reintroduce it.
 */

const config: HubConfig = { url: "https://hub.example", token: "t" };

function stubFetch(body: unknown) {
  const fetchMock = vi.fn(async () => ({
    ok: true,
    status: 200,
    json: async () => body,
  })) as unknown as typeof fetch;
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock as unknown as ReturnType<typeof vi.fn>;
}

function requestedUrl(fetchMock: ReturnType<typeof vi.fn>): URL {
  return new URL(fetchMock.mock.calls[0][0] as string);
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("hubApi search scoping", () => {
  it("asks for scope=messages and still unwraps the results block", async () => {
    const fetchMock = stubFetch({ results: [{ message_key: "m1" }] });

    const hits = await hubApi.search(config, "needle", { limit: 5 });

    const url = requestedUrl(fetchMock);
    expect(url.searchParams.get("scope")).toBe("messages");
    expect(url.searchParams.get("q")).toBe("needle");
    expect(url.searchParams.get("limit")).toBe("5");
    expect(hits).toHaveLength(1);
  });

  it("asks for scope=journal + mode=hybrid and reads the journal block", async () => {
    const fetchMock = stubFetch({ results: [], journal: [{ rank: 1 }] });

    const result = await hubApi.journalSearch(config, "needle");

    const url = requestedUrl(fetchMock);
    expect(url.searchParams.get("scope")).toBe("journal");
    expect(url.searchParams.get("mode")).toBe("hybrid");
    expect(result.hits).toHaveLength(1);
    expect(result.degraded).toBe(false);
  });

  it("surfaces the hub's journal_degraded flag", async () => {
    stubFetch({ results: [], journal: [{ rank: 1 }], journal_degraded: true });
    await expect(hubApi.journalSearch(config, "needle")).resolves.toMatchObject({
      degraded: true,
    });
  });

  it("yields no journal hits when the hub omits the block", async () => {
    stubFetch([{ message_key: "m1" }]);
    await expect(hubApi.journalSearch(config, "needle")).resolves.toEqual({
      hits: [],
      degraded: false,
    });
  });
});
