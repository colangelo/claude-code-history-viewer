/**
 * The archive's search state: the query, both result sets, and the request
 * generation that keeps them honest.
 *
 * Message hits and journal hits are fetched in parallel from one `runSearch`
 * because they share a generation — a stale response from a superseded query
 * must not clobber either list, and giving them separate counters would let
 * the two halves disagree about which query they belong to. Journal hits stay
 * best-effort: a hub without the journal block simply yields no journal
 * section, which is why their failure path resets rather than sets `error`.
 *
 * Activation is deliberately NOT here — landing on a hit means switching view
 * and opening a session, which is the browser's business, not search's.
 */

import { useCallback, useRef, useState } from "react";
import {
  hubApi,
  type HubConfig,
  type HubSearchHit,
  type JournalSearchHit,
} from "../../services/hubApi";

export interface ArchiveSearch {
  query: string;
  setQuery: (query: string) => void;
  /** `null` means "no search has run" — distinct from an empty result set. */
  hits: HubSearchHit[] | null;
  journalHits: JournalSearchHit[];
  journalDegraded: boolean;
  isSearching: boolean;
  error: string | null;
  runSearch: (query: string) => void;
  /** Dismiss the results without clearing the query input. */
  clearSearch: () => void;
}

export function useArchiveSearch(
  config: HubConfig,
  initialQuery: string
): ArchiveSearch {
  const [query, setQuery] = useState(initialQuery);
  const [hits, setHits] = useState<HubSearchHit[] | null>(null);
  const [journalHits, setJournalHits] = useState<JournalSearchHit[]>([]);
  const [journalDegraded, setJournalDegraded] = useState(false);
  const [isSearching, setIsSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Monotonic request generation so a slow, stale response from a superseded
  // query can never clobber the results of whatever the user has since run.
  const generationRef = useRef(0);

  const runSearch = useCallback(
    (q: string) => {
      if (!q) return;
      const generation = ++generationRef.current;
      setIsSearching(true);
      setError(null);
      setJournalHits([]);
      setJournalDegraded(false);
      hubApi
        .search(config, q)
        .then((result) => {
          if (generationRef.current !== generation) return;
          setHits(result);
        })
        .catch((err) => {
          if (generationRef.current !== generation) return;
          setError(String(err));
        })
        .finally(() => {
          if (generationRef.current !== generation) return;
          setIsSearching(false);
        });
      // Journal hits are additive and best-effort: a hub without the journal
      // block (or an unreachable one) simply yields no journal section.
      hubApi
        .journalSearch(config, q)
        .then((result) => {
          if (generationRef.current !== generation) return;
          setJournalHits(result.hits);
          setJournalDegraded(result.degraded);
        })
        .catch(() => {
          if (generationRef.current !== generation) return;
          setJournalHits([]);
          setJournalDegraded(false);
        });
    },
    [config]
  );

  // Bumping the generation is what makes this a dismissal rather than a
  // cosmetic clear: an in-flight response must not repopulate the list the
  // user just dismissed.
  const clearSearch = useCallback(() => {
    ++generationRef.current;
    setHits(null);
    setJournalHits([]);
    setJournalDegraded(false);
    setError(null);
    setIsSearching(false);
  }, []);

  return {
    query,
    setQuery,
    hits,
    journalHits,
    journalDegraded,
    isSearching,
    error,
    runSearch,
    clearSearch,
  };
}
