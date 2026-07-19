/**
 * Hash routes for the archive browser (issue #18 phase 4): refresh-safe,
 * shareable URL state with back/forward support.
 *
 *   #/journal                → journal feed, no date filter
 *   #/journal/2026-07-17     → journal feed anchored to a date
 *   #/browse                 → browse view, nothing open
 *   #/browse/session/<ref>   → browse view with a session open (numeric hub
 *                              pk or provider session-id string)
 *   #/search/<query>         → run a search for <query> (URI-encoded)
 *
 * Pure parse/format helpers — the ArchiveBrowser owns the wiring (state →
 * hash writes, hashchange → state application).
 */

export type ArchiveRoute =
  | { kind: "journal"; date: string | null }
  | { kind: "browse"; sessionRef: number | string | null }
  | { kind: "search"; query: string };

const DATE_RE = /^\d{4}-\d{2}-\d{2}$/;

export function parseArchiveHash(hash: string): ArchiveRoute | null {
  const parts = hash.replace(/^#/, "").split("/").filter(Boolean);
  if (parts.length === 0) return null;
  switch (parts[0]) {
    case "journal": {
      const date = parts[1] != null && DATE_RE.test(parts[1]) ? parts[1] : null;
      return { kind: "journal", date };
    }
    case "browse": {
      if (parts[1] === "session" && parts[2] != null) {
        const raw = decodeURIComponent(parts[2]);
        return {
          kind: "browse",
          sessionRef: /^\d+$/.test(raw) ? Number(raw) : raw,
        };
      }
      return { kind: "browse", sessionRef: null };
    }
    case "search": {
      if (parts.length < 2) return null;
      // A query may itself contain encoded slashes; rejoin the tail.
      return {
        kind: "search",
        query: decodeURIComponent(parts.slice(1).join("/")),
      };
    }
    default:
      return null;
  }
}

export function formatArchiveHash(route: ArchiveRoute): string {
  switch (route.kind) {
    case "journal":
      return route.date ? `#/journal/${route.date}` : "#/journal";
    case "browse":
      return route.sessionRef != null
        ? `#/browse/session/${encodeURIComponent(String(route.sessionRef))}`
        : "#/browse";
    case "search":
      return `#/search/${encodeURIComponent(route.query)}`;
  }
}
