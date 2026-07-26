/**
 * Browse's session list for the selected project group, with the request
 * generation that keeps a superseded project's slow response from landing in
 * the pane.
 *
 * `showWorktrees` is passed per call rather than held here: the toggle lives
 * with the projects surface, and threading it through as an argument keeps
 * this hook from re-fetching on a state change it does not own.
 */

import { useCallback, useRef, useState } from "react";
import type { ProjectGroup } from "./projectGrouping";
import {
  hubApi,
  identityProjectFilter,
  type HubConfig,
  type HubSession,
} from "../../services/hubApi";

export interface ArchiveSessions {
  sessions: HubSession[];
  isLoading: boolean;
  error: string | null;
  fetchSessionsFor: (group: ProjectGroup, showWorktrees: boolean) => void;
  /** Drop the list and invalidate any in-flight fetch (mobile drill-up). */
  reset: () => void;
}

export function useArchiveSessions(config: HubConfig): ArchiveSessions {
  const [sessions, setSessions] = useState<HubSession[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const generationRef = useRef(0);

  const fetchSessionsFor = useCallback(
    (group: ProjectGroup, showWorktrees: boolean) => {
      const generation = ++generationRef.current;
      setSessions([]);
      setError(null);
      setIsLoading(true);
      // Identity groups query the hub's identity scope (server-side expansion
      // to member + aliased paths); path groups keep the byte-exact path
      // filter. Neither pins machine/provider — a group spans them.
      hubApi
        .listSessions(config, {
          project: group.identityKey
            ? identityProjectFilter(group.identityKey)
            : group.paths[0] ?? "",
          include_worktrees:
            group.identityKey && !showWorktrees ? false : undefined,
        })
        .then((list) => {
          if (generationRef.current !== generation) return;
          setSessions(list);
        })
        .catch((err) => {
          if (generationRef.current !== generation) return;
          setError(String(err));
        })
        .finally(() => {
          if (generationRef.current !== generation) return;
          setIsLoading(false);
        });
    },
    [config]
  );

  // Deliberately leaves `isLoading` alone: the generation bump makes any
  // in-flight `.finally` skip, and with no group selected the pane shows the
  // "pick a project" prompt instead of the loading line, so a stranded `true`
  // is never rendered — and clearing it here would flash the empty state.
  const reset = useCallback(() => {
    ++generationRef.current;
    setSessions([]);
    setError(null);
  }, []);

  return { sessions, isLoading, error, fetchSessionsFor, reset };
}
