/**
 * Archive browser: browse and search the cross-machine hub archive via
 * `services/hubApi.ts`. Presents two views behind a tab switcher —
 * **Journal** (the default landing view: a distilled day-timeline feed) and
 * **Browse** (projects → sessions → messages) — with a global search bar above
 * both. Rendered as its own mode: archived history spans machines and outlives
 * local retention, so it is presented separately from the local provider tree,
 * with provenance (machine hostname) visible.
 */

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { GitBranch } from "lucide-react";
import { cn } from "@/lib/utils";
import { JournalView } from "./JournalView";
import { AnalyticsView } from "./AnalyticsView";
import { SearchBar } from "./SearchBar";
import { SearchResults } from "./SearchResults";
import { useArchiveSearch } from "./useArchiveSearch";
import { useArchiveSessions } from "./useArchiveSessions";
import { ProjectsPane } from "./ProjectsPane";
import { SessionsPane } from "./SessionsPane";
import { MessagesPane, type OpenSession } from "./MessagesPane";
import {
  aliasKeyByPath,
  groupProjects,
  type ProjectGroup,
} from "./projectGrouping";
import {
  loadShowWorktrees,
  storeShowWorktrees,
} from "./worktreeVisibilityStorage";
import {
  formatArchiveHash,
  parseArchiveHash,
  type ArchiveRoute,
} from "./archiveRoute";
import {
  hubApi,
  type HubConfig,
  type HubIdentity,
  type HubMessage,
  type HubProject,
  type HubSearchHit,
  type JournalSearchHit,
} from "../../services/hubApi";

export interface ArchiveBrowserProps {
  /** Hub connection; callers normally derive this from user settings. */
  config: HubConfig;
  /** Own `location.hash` as routable state (#/journal/…, #/browse/session/…).
   * Only the standalone webapp turns this on — embedded in the desktop/WebUI
   * the browser owns the URL and the hash must be left alone. */
  enableHashRoutes?: boolean;
}

/** Hub's max page size (`crates/hub/src/pagination.rs::MAX_LIMIT`). */
const PAGE_SIZE = 200;

type ArchiveView = "journal" | "browse" | "analytics";

/** Optional project context for a session opened from Journal or a search
 * hit — used to sync the Browse panes (select the project, load its
 * sessions) so the surrounding lists match what's open. */
export interface SessionOpenContext {
  project_path?: string | null;
  machine_hostname?: string | null;
  provider?: string | null;
}

/** Where to land inside a session opened from a search hit (issue #20):
 * open the page containing `position` and highlight `messageId`. */
export interface SessionOpenTarget {
  position: number;
  messageId?: number;
}

export function ArchiveBrowser({
  config,
  enableHashRoutes = false,
}: ArchiveBrowserProps) {
  const { t } = useTranslation();

  // Deep link (#/journal/<date>, #/browse/session/<ref>, #/search/<q>)
  // parsed once before first render; state initializers consume it and a
  // mount effect triggers the fetches it implies.
  const initialRouteRef = useRef(
    enableHashRoutes ? parseArchiveHash(window.location.hash) : null
  );

  const [view, setView] = useState<ArchiveView>(
    initialRouteRef.current?.kind === "browse" ? "browse" : "journal"
  );
  // Anchor requested from a journal search hit or an inbound route; `nonce`
  // re-triggers a jump even when the date is unchanged. "" clears the filter.
  const [anchorDate, setAnchorDate] = useState<string | null>(
    initialRouteRef.current?.kind === "journal"
      ? initialRouteRef.current.date
      : null
  );
  const [anchorNonce, setAnchorNonce] = useState(0);
  // Mirror of JournalView's date filter — only feeds the hash writer.
  const [journalDate, setJournalDate] = useState<string>(
    initialRouteRef.current?.kind === "journal"
      ? (initialRouteRef.current.date ?? "")
      : ""
  );

  const [projects, setProjects] = useState<HubProject[]>([]);
  const [identities, setIdentities] = useState<HubIdentity[]>([]);
  const [isLoadingProjects, setIsLoadingProjects] = useState(false);
  const [projectsError, setProjectsError] = useState<string | null>(null);
  const [selectedGroup, setSelectedGroup] = useState<ProjectGroup | null>(null);
  const [showWorktrees, setShowWorktrees] = useState<boolean>(loadShowWorktrees);
  const [aliasError, setAliasError] = useState<string | null>(null);

  // Identity grouping of the sidebar/dropdown (spec: archive-journal-ui
  // "Identity-grouped project surfaces"). Aliased dead paths fold into their
  // identity's group so a moved repo appears once.
  const projectGroups = useMemo(
    () =>
      groupProjects(projects, {
        aliases: aliasKeyByPath(identities),
        showWorktrees,
      }),
    [projects, identities, showWorktrees]
  );
  // The selected group's CURRENT incarnation (grouping recomputes on alias or
  // toggle changes; selection is stable by key).
  const activeGroup =
    projectGroups.find((g) => g.key === selectedGroup?.key) ?? selectedGroup;

  const {
    sessions,
    isLoading: isLoadingSessions,
    error: sessionsError,
    fetchSessionsFor,
    reset: resetSessions,
  } = useArchiveSessions(config);

  const [openSession, setOpenSession] = useState<OpenSession | null>(null);
  const [messages, setMessages] = useState<HubMessage[]>([]);
  const [totalCount, setTotalCount] = useState<number | null>(null);
  // Offset of messages[0] in the session (issue #20): a search hit opens the
  // page CONTAINING the match, so the loaded window need not start at 0.
  const [windowStart, setWindowStart] = useState(0);
  const [highlightMessageId, setHighlightMessageId] = useState<number | null>(null);
  // Scroll-once flag: consumed by the effect that centers the matched message.
  const pendingScrollRef = useRef<number | null>(null);
  const [isLoadingMessages, setIsLoadingMessages] = useState(false);
  // Distinct from `isLoadingMessages` so only the multi-page walk offers a
  // stop: a 59k-message session takes minutes, and measured, it had no escape.
  const [isLoadingAll, setIsLoadingAll] = useState(false);
  const abortLoadAllRef = useRef(false);
  const [messagesError, setMessagesError] = useState<string | null>(null);

  const {
    query: searchQuery,
    setQuery: setSearchQuery,
    hits: searchHits,
    journalHits,
    journalDegraded,
    isSearching,
    error: searchError,
    runSearch,
    clearSearch: handleClearSearch,
  } = useArchiveSearch(
    config,
    initialRouteRef.current?.kind === "search"
      ? initialRouteRef.current.query
      : ""
  );

  // Monotonic request generation so a slow, stale response from a superseded
  // session selection can never clobber the messages of whatever the user has
  // since opened. The sessions and search counters live in their own hooks,
  // each beside the fetch it guards.
  const messagesGenerationRef = useRef(0);

  // Hash writes we initiated — the hashchange listener must ignore them or
  // every state-driven write would bounce back as a route application.
  const selfHashRef = useRef<string | null>(null);
  const writeHash = useCallback(
    (hash: string) => {
      if (!enableHashRoutes) return;
      if (window.location.hash === hash) return;
      selfHashRef.current = hash;
      window.location.hash = hash;
    },
    [enableHashRoutes]
  );

  useEffect(() => {
    let cancelled = false;
    setIsLoadingProjects(true);
    setProjectsError(null);
    hubApi
      .listProjects(config)
      .then((list) => {
        if (!cancelled) setProjects(list);
      })
      .catch((err) => {
        if (!cancelled) setProjectsError(String(err));
      })
      .finally(() => {
        if (!cancelled) setIsLoadingProjects(false);
      });
    return () => {
      cancelled = true;
    };
  }, [config]);

  // Identity metadata (aliases + suggestions) is additive and best-effort: a
  // hub without /v1/identities simply yields ungrouped-by-alias behavior.
  const refreshIdentities = useCallback(() => {
    hubApi
      .listIdentities(config)
      .then(setIdentities)
      .catch(() => {
        setIdentities([]);
      });
  }, [config]);
  useEffect(() => {
    refreshIdentities();
  }, [refreshIdentities]);

  // Select a group WITHOUT clearing the open session — pane sync for a
  // session opened from Journal or a search hit.
  const syncProjectSelection = useCallback(
    (group: ProjectGroup) => {
      setSelectedGroup(group);
      fetchSessionsFor(group, showWorktrees);
    },
    [fetchSessionsFor, showWorktrees]
  );

  const openSessionRef = useCallback(
    (
      ref: number | string,
      label: string,
      context?: SessionOpenContext,
      target?: SessionOpenTarget
    ) => {
      const generation = ++messagesGenerationRef.current;
      // Land on the page containing the target (older hubs send no position
      // → page 1 exactly as before).
      const start = target
        ? Math.floor(target.position / PAGE_SIZE) * PAGE_SIZE
        : 0;
      setOpenSession({ ref, label });
      setMessages([]);
      setTotalCount(null);
      setWindowStart(start);
      setHighlightMessageId(target?.messageId ?? null);
      pendingScrollRef.current = target?.messageId ?? null;
      setMessagesError(null);
      setIsLoadingMessages(true);
      hubApi
        .sessionMessages(config, ref, { limit: PAGE_SIZE, offset: start })
        .then((page) => {
          if (messagesGenerationRef.current !== generation) return;
          setMessages(page.messages);
          setTotalCount(page.totalCount);
        })
        .catch((err) => {
          if (messagesGenerationRef.current !== generation) return;
          setMessagesError(String(err));
        })
        .finally(() => {
          if (messagesGenerationRef.current !== generation) return;
          setIsLoadingMessages(false);
        });
      // Pane sync: only when the context pins exactly one group — identity
      // grouping makes this the common case (same path on several machines
      // now folds into one group instead of being ambiguous).
      if (context?.project_path) {
        const matches = projectGroups.filter((g) =>
          g.rows.some(
            (p) =>
              p.project_path === context.project_path &&
              (context.machine_hostname == null ||
                p.machine_hostname === context.machine_hostname) &&
              (context.provider == null || p.provider === context.provider)
          )
        );
        if (matches.length === 1 && matches[0]!.key !== selectedGroup?.key) {
          syncProjectSelection(matches[0]!);
        }
      }
    },
    [config, projectGroups, selectedGroup?.key, syncProjectSelection]
  );

  // Open a session from the Journal view: switch to Browse and load messages
  // through the existing message-fetch path.
  const handleOpenSessionFromJournal = useCallback(
    (sessionId: number, label: string, context?: SessionOpenContext) => {
      setView("browse");
      openSessionRef(sessionId, label, context);
    },
    [openSessionRef]
  );

  const handleSelectGroup = useCallback(
    (group: ProjectGroup) => {
      // Invalidate any in-flight message fetch for the previously open
      // session — it belongs to a project we're navigating away from.
      ++messagesGenerationRef.current;
      setSelectedGroup(group);
      setAliasError(null);
      setOpenSession(null);
      setMessages([]);
      setTotalCount(null);
      setMessagesError(null);
      fetchSessionsFor(group, showWorktrees);
    },
    [fetchSessionsFor, showWorktrees]
  );

  const handleToggleWorktrees = useCallback(() => {
    const next = !showWorktrees;
    setShowWorktrees(next);
    storeShowWorktrees(next);
    // Identity-scoped sessions change with the toggle; refetch explicitly
    // (path groups are unaffected by the param).
    if (selectedGroup?.identityKey) {
      fetchSessionsFor(selectedGroup, next);
    }
  }, [showWorktrees, selectedGroup, fetchSessionsFor]);

  // Alias management: explicit user actions with visible error feedback.
  const handleLinkAlias = useCallback(
    (projectPath: string, identityKey: string) => {
      setAliasError(null);
      hubApi
        .createAlias(config, projectPath, identityKey)
        .then(() => {
          refreshIdentities();
          // The linked path's history joins the selected group's scope.
          if (selectedGroup?.identityKey === identityKey) {
            fetchSessionsFor(selectedGroup, showWorktrees);
          }
        })
        .catch((err) => {
          setAliasError(String(err));
        });
    },
    [config, refreshIdentities, selectedGroup, fetchSessionsFor, showWorktrees]
  );

  const handleUnlinkAlias = useCallback(
    (aliasId: number) => {
      setAliasError(null);
      hubApi
        .deleteAlias(config, aliasId)
        .then(() => {
          refreshIdentities();
          if (selectedGroup?.identityKey) {
            fetchSessionsFor(selectedGroup, showWorktrees);
          }
        })
        .catch((err) => {
          setAliasError(String(err));
        });
    },
    [config, refreshIdentities, selectedGroup, fetchSessionsFor, showWorktrees]
  );

  const handleLoadMore = useCallback(() => {
    // Guard against double-submit: a second click can land before React
    // applies the button's disabled state, duplicating a page.
    if (!openSession || isLoadingMessages) return;
    const generation = messagesGenerationRef.current;
    setIsLoadingMessages(true);
    hubApi
      .sessionMessages(config, openSession.ref, {
        limit: PAGE_SIZE,
        offset: windowStart + messages.length,
      })
      .then((page) => {
        if (messagesGenerationRef.current !== generation) return;
        setMessages((prev) => [...prev, ...page.messages]);
        setTotalCount(page.totalCount);
      })
      .catch((err) => {
        if (messagesGenerationRef.current !== generation) return;
        setMessagesError(String(err));
      })
      .finally(() => {
        if (messagesGenerationRef.current !== generation) return;
        setIsLoadingMessages(false);
      });
  }, [config, openSession, messages.length, isLoadingMessages, windowStart]);

  // Extend the window upward when it doesn't start at the session's beginning
  // (a search hit landed mid-session).
  const handleLoadEarlier = useCallback(() => {
    if (!openSession || isLoadingMessages || windowStart === 0) return;
    const generation = messagesGenerationRef.current;
    const fetchOffset = Math.max(0, windowStart - PAGE_SIZE);
    const fetchLimit = windowStart - fetchOffset;
    setIsLoadingMessages(true);
    hubApi
      .sessionMessages(config, openSession.ref, {
        limit: fetchLimit,
        offset: fetchOffset,
      })
      .then((page) => {
        if (messagesGenerationRef.current !== generation) return;
        setMessages((prev) => [...page.messages, ...prev]);
        setWindowStart(fetchOffset);
        setTotalCount(page.totalCount);
      })
      .catch((err) => {
        if (messagesGenerationRef.current !== generation) return;
        setMessagesError(String(err));
      })
      .finally(() => {
        if (messagesGenerationRef.current !== generation) return;
        setIsLoadingMessages(false);
      });
  }, [config, openSession, isLoadingMessages, windowStart]);

  // Issue #28: a long session took a dozen "Load more" clicks to walk. Pages
  // are still fetched PAGE_SIZE at a time and appended as each lands, so the
  // list visibly fills rather than hanging on one long request — and the
  // generation check both stops the loop and leaves the loading flag alone
  // when the user navigates away mid-walk (whoever superseded us owns it).
  const handleLoadAll = useCallback(async () => {
    if (!openSession || isLoadingMessages) return;
    const generation = messagesGenerationRef.current;
    abortLoadAllRef.current = false;
    setIsLoadingAll(true);
    setIsLoadingMessages(true);
    // Tracked locally: `messages.length` is captured by this closure and would
    // stay at its first-render value for every iteration after the first.
    let offset = windowStart + messages.length;
    try {
      for (;;) {
        const page = await hubApi.sessionMessages(config, openSession.ref, {
          limit: PAGE_SIZE,
          offset,
        });
        if (messagesGenerationRef.current !== generation) return;
        setTotalCount(page.totalCount);
        if (page.messages.length === 0) return;
        setMessages((prev) => [...prev, ...page.messages]);
        offset += page.messages.length;
        if (offset >= page.totalCount) return;
        // Checked after the append so a stop always keeps the page in hand.
        if (abortLoadAllRef.current) return;
      }
    } catch (err) {
      if (messagesGenerationRef.current !== generation) return;
      setMessagesError(String(err));
    } finally {
      if (messagesGenerationRef.current === generation) {
        setIsLoadingMessages(false);
        setIsLoadingAll(false);
      }
    }
  }, [config, openSession, messages.length, isLoadingMessages, windowStart]);

  // The pane's one button toggles between starting and stopping the walk, so
  // it gets both verbs. Starting swallows the promise here rather than in the
  // click handler; stopping only raises the flag `handleLoadAll` reads.
  const handleLoadAllClick = useCallback(() => {
    void handleLoadAll();
  }, [handleLoadAll]);

  const handleStopLoadAll = useCallback(() => {
    abortLoadAllRef.current = true;
  }, []);

  // Center the matched message once its page has rendered.
  useEffect(() => {
    const id = pendingScrollRef.current;
    if (id == null || !messages.some((m) => m.id === id)) return;
    pendingScrollRef.current = null;
    requestAnimationFrame(() => {
      document
        .querySelector(`[data-msg-id="${id}"]`)
        ?.scrollIntoView({ block: "center" });
    });
  }, [messages]);

  const handleSearchSubmit = useCallback(
    (e: FormEvent) => {
      e.preventDefault();
      const query = searchQuery.trim();
      if (!query) return;
      runSearch(query);
      writeHash(formatArchiveHash({ kind: "search", query }));
    },
    [searchQuery, runSearch, writeHash]
  );

  const handleActivateHit = useCallback(
    (hit: HubSearchHit) => {
      handleClearSearch();
      // Land the user ON the session — a hit activated from the Journal view
      // used to open it invisibly behind the feed.
      setView("browse");
      openSessionRef(
        hit.session_id,
        hit.session_summary ?? hit.session_id,
        {
          project_path: hit.project_path,
          machine_hostname: hit.machine_hostname,
          provider: hit.provider,
        },
        // Land on the matched message when the hub says where it is
        // (cchv-v0.10.1+); older hubs → page 1 as before.
        hit.position != null
          ? { position: hit.position, messageId: hit.message_id }
          : undefined
      );
    },
    [openSessionRef, handleClearSearch]
  );

  const handleActivateJournalHit = useCallback(
    (hit: JournalSearchHit) => {
      handleClearSearch();
      setView("journal");
      setAnchorDate(hit.entry_date);
      setAnchorNonce((n) => n + 1);
    },
    [handleClearSearch]
  );

  // `/` focuses the search input from anywhere non-editable (issue #21).
  // Analytics renders no search surface, so there `/` first switches to
  // Journal — the view where search lives — and focuses once it exists.
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const pendingSearchFocusRef = useRef(false);
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "/" || e.metaKey || e.ctrlKey || e.altKey) return;
      const el = document.activeElement;
      const editable =
        el instanceof HTMLElement &&
        (el.tagName === "INPUT" ||
          el.tagName === "TEXTAREA" ||
          el.tagName === "SELECT" ||
          el.isContentEditable);
      if (editable) return;
      e.preventDefault();
      if (searchInputRef.current) {
        searchInputRef.current.focus();
      } else {
        pendingSearchFocusRef.current = true;
        setView("journal");
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
  useEffect(() => {
    if (!pendingSearchFocusRef.current) return;
    pendingSearchFocusRef.current = false;
    searchInputRef.current?.focus();
  }, [view]);

  // Arrow-key navigation on the Journal|Browse tablist (issue #21).
  const handleTablistKeyDown = useCallback(
    (e: ReactKeyboardEvent<HTMLDivElement>) => {
      if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
      e.preventDefault();
      const tabs = Array.from(
        e.currentTarget.querySelectorAll<HTMLButtonElement>('[role="tab"]')
      );
      if (tabs.length === 0) return;
      const current = tabs.findIndex((tab) => tab === document.activeElement);
      const base =
        current >= 0
          ? current
          : view === "journal"
            ? 0
            : view === "browse"
              ? 1
              : 2;
      const delta = e.key === "ArrowRight" ? 1 : -1;
      const next = tabs[(base + delta + tabs.length) % tabs.length]!;
      next.focus();
      next.click();
    },
    [view]
  );

  // Mobile drill-up: the stacked (<md) Browse shows one level at a time.
  const handleBackToProjects = useCallback(() => {
    ++messagesGenerationRef.current;
    resetSessions();
    setSelectedGroup(null);
    setOpenSession(null);
    setMessages([]);
    setTotalCount(null);
    setWindowStart(0);
    setHighlightMessageId(null);
    setMessagesError(null);
  }, [resetSessions]);

  const handleBackFromMessages = useCallback(() => {
    ++messagesGenerationRef.current;
    setOpenSession(null);
    setMessages([]);
    setTotalCount(null);
    setWindowStart(0);
    setHighlightMessageId(null);
    setMessagesError(null);
    setIsLoadingMessages(false);
  }, []);

  // --- Hash routing: state → URL. Search hashes are written on submit; the
  // view/date/session state is reflected here. Skip the very first write when
  // the page loaded on a search deep link so it isn't clobbered before the
  // search state settles.
  const skipFirstWriteRef = useRef(initialRouteRef.current?.kind === "search");
  useEffect(() => {
    if (skipFirstWriteRef.current) {
      skipFirstWriteRef.current = false;
      return;
    }
    const route: ArchiveRoute =
      view === "journal"
        ? { kind: "journal", date: journalDate || null }
        : { kind: "browse", sessionRef: openSession?.ref ?? null };
    writeHash(formatArchiveHash(route));
  }, [view, journalDate, openSession, writeHash]);

  // --- Hash routing: URL → state (back/forward, hand-edited hashes). A
  // latest-callback ref lets the singleton listener see fresh state without
  // re-subscribing every render.
  const applyRouteRef = useRef<(route: ArchiveRoute) => void>(() => {});
  applyRouteRef.current = (route: ArchiveRoute) => {
    if (route.kind === "journal") {
      setView("journal");
      setAnchorDate(route.date ?? "");
      setAnchorNonce((n) => n + 1);
    } else if (route.kind === "browse") {
      setView("browse");
      if (route.sessionRef == null) {
        if (openSession) handleBackFromMessages();
      } else if (openSession?.ref !== route.sessionRef) {
        openSessionRef(route.sessionRef, String(route.sessionRef));
      }
    } else {
      setSearchQuery(route.query);
      runSearch(route.query);
    }
  };

  useEffect(() => {
    if (!enableHashRoutes) return;
    const onHashChange = () => {
      const hash = window.location.hash;
      if (selfHashRef.current === hash) {
        selfHashRef.current = null;
        return;
      }
      const route = parseArchiveHash(hash);
      if (route) applyRouteRef.current(route);
    };
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, [enableHashRoutes]);

  // Deep-link fetches on mount — the state initializers only set the shape;
  // the session/search the route names still has to load.
  useEffect(() => {
    const route = initialRouteRef.current;
    if (route?.kind === "browse" && route.sessionRef != null) {
      openSessionRef(route.sessionRef, String(route.sessionRef));
    } else if (route?.kind === "search") {
      runSearch(route.query);
    }
    // Mount-only by design: the route was captured before first render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const hasMoreMessages =
    totalCount !== null && windowStart + messages.length < totalCount;
  const remainingMessages =
    totalCount === null ? 0 : totalCount - (windowStart + messages.length);

  return (
    <div
      data-testid="archive-browser"
      className="flex flex-col h-full gap-3 overflow-hidden"
    >
      {/* Search lives with content: the bar and its results render in
          Journal and Browse only. Analytics keeps its own toolbar; result
          state survives the round-trip because only rendering is gated. */}
      {view !== "analytics" && (
        <>
          <SearchBar
            inputRef={searchInputRef}
            query={searchQuery}
            onQueryChange={setSearchQuery}
            onSubmit={handleSearchSubmit}
          />
          {/* Search results (global, above both views) */}
          <SearchResults
            isSearching={isSearching}
            error={searchError}
            hits={searchHits}
            journalHits={journalHits}
            journalDegraded={journalDegraded}
            onClear={handleClearSearch}
            onActivateHit={handleActivateHit}
            onActivateJournalHit={handleActivateJournalHit}
          />
        </>
      )}

      {/* View switcher + worktree visibility toggle */}
      <div className="flex items-center gap-1 shrink-0 border-b border-border/50">
        <div
          role="tablist"
          aria-label={t("settings.archiveHub.journal.tabsLabel")}
          className="flex items-center gap-1"
          onKeyDown={handleTablistKeyDown}
        >
          <button
            type="button"
            role="tab"
            data-testid="archive-tab-journal"
            aria-selected={view === "journal"}
            onClick={() => setView("journal")}
            className={cn(
              "px-3 py-2 text-px14 border-b-2 -mb-px",
              view === "journal"
                ? "border-accent text-foreground font-medium"
                : "border-transparent text-muted-foreground hover:text-foreground"
            )}
          >
            {t("settings.archiveHub.journal.tab.journal")}
          </button>
          <button
            type="button"
            role="tab"
            data-testid="archive-tab-browse"
            aria-selected={view === "browse"}
            onClick={() => setView("browse")}
            className={cn(
              "px-3 py-2 text-px14 border-b-2 -mb-px",
              view === "browse"
                ? "border-accent text-foreground font-medium"
                : "border-transparent text-muted-foreground hover:text-foreground"
            )}
          >
            {t("settings.archiveHub.journal.tab.browse")}
          </button>
          <button
            type="button"
            role="tab"
            data-testid="archive-tab-analytics"
            aria-selected={view === "analytics"}
            onClick={() => setView("analytics")}
            className={cn(
              "px-3 py-2 text-px14 border-b-2 -mb-px",
              view === "analytics"
                ? "border-accent text-foreground font-medium"
                : "border-transparent text-muted-foreground hover:text-foreground"
            )}
          >
            {t("settings.archiveHub.journal.tab.analytics")}
          </button>
        </div>
        <button
          type="button"
          role="switch"
          data-testid="worktree-toggle"
          aria-checked={showWorktrees}
          aria-label={t("settings.archiveHub.identity.showWorktrees")}
          title={t("settings.archiveHub.identity.showWorktrees")}
          onClick={handleToggleWorktrees}
          className={cn(
            "ml-auto flex items-center gap-1 rounded-md border px-2 py-1 text-px12 transition-colors",
            // On/off used to differ only by a strikethrough. The accent tint
            // makes the live state legible at a glance, in the same language
            // the tabs and the Link buttons already speak.
            showWorktrees
              ? "border-accent/30 bg-accent/10 text-accent font-medium"
              : "border-border/50 text-muted-foreground line-through hover:bg-muted"
          )}
        >
          <GitBranch className="w-3 h-3" aria-hidden="true" />
          {t("settings.archiveHub.identity.showWorktrees")}
        </button>
      </div>

      {view === "journal" ? (
        <JournalView
          config={config}
          anchorDate={anchorDate}
          anchorNonce={anchorNonce}
          projectGroups={projectGroups}
          showWorktrees={showWorktrees}
          onOpenSession={handleOpenSessionFromJournal}
          onDateChange={setJournalDate}
        />
      ) : view === "analytics" ? (
        <div className="flex-1 min-h-0 overflow-y-auto">
          <AnalyticsView config={config} identities={identities} />
        </div>
      ) : (
        <div className="flex flex-1 min-h-0 gap-3">
          {/* Below `md` the three panes stack: exactly one level is visible,
              and the back buttons walk back up. Which level that is depends
              on the drill-down state, so the panes take `hidden` rather than
              deriving it. */}
          <ProjectsPane
            groups={projectGroups}
            activeGroup={activeGroup}
            identities={identities}
            isLoading={isLoadingProjects}
            error={projectsError}
            aliasError={aliasError}
            hidden={Boolean(selectedGroup || openSession)}
            onSelectGroup={handleSelectGroup}
            onLinkAlias={handleLinkAlias}
            onUnlinkAlias={handleUnlinkAlias}
          />
          <SessionsPane
            sessions={sessions}
            hasSelection={selectedGroup != null}
            openSessionRef={openSession?.ref ?? null}
            isLoading={isLoadingSessions}
            error={sessionsError}
            hidden={!selectedGroup || Boolean(openSession)}
            onBackToProjects={handleBackToProjects}
            onOpenSession={openSessionRef}
          />
          <MessagesPane
            messages={messages}
            openSession={openSession}
            totalCount={totalCount}
            windowStart={windowStart}
            highlightMessageId={highlightMessageId}
            isLoadingMessages={isLoadingMessages}
            isLoadingAll={isLoadingAll}
            error={messagesError}
            hasMore={hasMoreMessages}
            remaining={remainingMessages}
            hasSelectedGroup={selectedGroup != null}
            hidden={!openSession}
            onBack={handleBackFromMessages}
            onLoadEarlier={handleLoadEarlier}
            onLoadMore={handleLoadMore}
            onLoadAll={handleLoadAllClick}
            onStopLoadAll={handleStopLoadAll}
          />
        </div>
      )}
    </div>
  );
}
