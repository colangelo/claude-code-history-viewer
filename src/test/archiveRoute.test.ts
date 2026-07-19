import { describe, expect, it } from "vitest";
import {
  formatArchiveHash,
  parseArchiveHash,
  type ArchiveRoute,
} from "../components/ArchiveBrowser/archiveRoute";

describe("parseArchiveHash", () => {
  it("returns null for empty or unknown hashes", () => {
    expect(parseArchiveHash("")).toBeNull();
    expect(parseArchiveHash("#")).toBeNull();
    expect(parseArchiveHash("#/")).toBeNull();
    expect(parseArchiveHash("#/nope")).toBeNull();
    expect(parseArchiveHash("#/search")).toBeNull();
  });

  it("parses journal routes with and without a date", () => {
    expect(parseArchiveHash("#/journal")).toEqual({
      kind: "journal",
      date: null,
    });
    expect(parseArchiveHash("#/journal/2026-07-17")).toEqual({
      kind: "journal",
      date: "2026-07-17",
    });
    // Malformed dates degrade to the plain journal route.
    expect(parseArchiveHash("#/journal/tomorrow")).toEqual({
      kind: "journal",
      date: null,
    });
  });

  it("parses browse routes with numeric and string session refs", () => {
    expect(parseArchiveHash("#/browse")).toEqual({
      kind: "browse",
      sessionRef: null,
    });
    expect(parseArchiveHash("#/browse/session/5081")).toEqual({
      kind: "browse",
      sessionRef: 5081,
    });
    expect(parseArchiveHash("#/browse/session/abc-123")).toEqual({
      kind: "browse",
      sessionRef: "abc-123",
    });
  });

  it("parses search routes including encoded slashes and spaces", () => {
    expect(parseArchiveHash("#/search/hello%20world")).toEqual({
      kind: "search",
      query: "hello world",
    });
    expect(parseArchiveHash("#/search/a%2Fb")).toEqual({
      kind: "search",
      query: "a/b",
    });
  });

  it("round-trips through formatArchiveHash", () => {
    const routes: ArchiveRoute[] = [
      { kind: "journal", date: null },
      { kind: "journal", date: "2026-01-02" },
      { kind: "browse", sessionRef: null },
      { kind: "browse", sessionRef: 42 },
      { kind: "browse", sessionRef: "sess-uuid" },
      { kind: "search", query: "type scale 15px" },
    ];
    for (const route of routes) {
      expect(parseArchiveHash(formatArchiveHash(route))).toEqual(route);
    }
  });
});
