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
import { ChevronLeft, GitBranch, Loader2, X } from "lucide-react";
import { ExpandKeyProvider } from "@/contexts/CaptureExpandContext";
import { MessageContentDisplay } from "@/components/messageRenderer";
import { ClaudeContentArrayRenderer } from "@/components/contentRenderer";
import { cn } from "@/lib/utils";
import { formatCount, humanizeTimestamp } from "@/utils/journalFormat";
import { renderSnippet } from "@/utils/searchSnippet";
import { getProviderLabel } from "@/utils/providers";
import { ArchiveRenderContext } from "@/contexts/ArchiveRenderContext";
import { JournalView } from "./JournalView";
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
  hubMessageToClaudeMessage,
  identityProjectFilter,
  type HubConfig,
  type HubIdentity,
  type HubMessage,
  type HubProject,
  type HubSearchHit,
  type HubSession,
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

type ArchiveView = "journal" | "browse";

interface OpenSession {
  ref: number | string;
  label: string;
}

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

/** Roles that get a turn-boundary gutter; record types (`attachment`, `mode`,
 * …) neither render one nor reset the turn. */
const GUTTER_ROLES = new Set(["user", "assistant", "system", "summary"]);

/** The previous conversation-role before `index`, skipping record rows. */
function lastConversationRole(
  messages: HubMessage[],
  index: number
): string | null {
  for (let i = index - 1; i >= 0; i--) {
    const role = messages[i]!.role ?? messages[i]!.message_type;
    if (role != null && GUTTER_ROLES.has(role)) return role;
  }
  return null;
}

/** Localized role label for the message gutter; unknown roles pass through. */
function roleLabel(role: string, t: (key: string) => string): string {
  switch (role) {
    case "user":
      return t("navigator.role.user");
    case "assistant":
      return t("navigator.role.assistant");
    case "system":
      return t("navigator.role.system");
    case "summary":
      return t("navigator.role.summary");
    default:
      return role;
  }
}

/** Renders one archived message via the existing content renderers, keeping
 * structured content (tool use/results/thinking, etc.) intact rather than
 * collapsing it to a text preview. */
function ArchivedMessage({ row, sessionId }: { row: HubMessage; sessionId: string }) {
  const claudeMessage = hubMessageToClaudeMessage(row, sessionId);
  const content = claudeMessage.content;

  return (
    <ExpandKeyProvider value={claudeMessage.uuid}>
      {Array.isArray(content) ? (
        <ClaudeContentArrayRenderer content={content} />
      ) : (
        <MessageContentDisplay
          content={typeof content === "string" ? content : null}
          messageType={claudeMessage.type}
        />
      )}
    </ExpandKeyProvider>
  );
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

  const [sessions, setSessions] = useState<HubSession[]>([]);
  const [isLoadingSessions, setIsLoadingSessions] = useState(false);
  const [sessionsError, setSessionsError] = useState<string | null>(null);

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
  const [messagesError, setMessagesError] = useState<string | null>(null);

  const [searchQuery, setSearchQuery] = useState(
    initialRouteRef.current?.kind === "search"
      ? initialRouteRef.current.query
      : ""
  );
  const [searchHits, setSearchHits] = useState<HubSearchHit[] | null>(null);
  const [journalHits, setJournalHits] = useState<JournalSearchHit[]>([]);
  const [isSearching, setIsSearching] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);

  // Monotonic request generations so a slow, stale response from a
  // superseded project/session/search selection can never clobber the state
  // of whatever the user has since selected.
  const sessionsGenerationRef = useRef(0);
  const messagesGenerationRef = useRef(0);
  const searchGenerationRef = useRef(0);

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

  const fetchSessionsFor = useCallback(
    (group: ProjectGroup, showWt: boolean) => {
      const generation = ++sessionsGenerationRef.current;
      setSessions([]);
      setSessionsError(null);
      setIsLoadingSessions(true);
      // Identity groups query the hub's identity scope (server-side expansion
      // to member + aliased paths); path groups keep the byte-exact path
      // filter. Neither pins machine/provider — a group spans them.
      hubApi
        .listSessions(config, {
          project: group.identityKey
            ? identityProjectFilter(group.identityKey)
            : group.paths[0] ?? "",
          include_worktrees:
            group.identityKey && !showWt ? false : undefined,
        })
        .then((list) => {
          if (sessionsGenerationRef.current !== generation) return;
          setSessions(list);
        })
        .catch((err) => {
          if (sessionsGenerationRef.current !== generation) return;
          setSessionsError(String(err));
        })
        .finally(() => {
          if (sessionsGenerationRef.current !== generation) return;
          setIsLoadingSessions(false);
        });
    },
    [config]
  );

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

  const runSearch = useCallback(
    (query: string) => {
      if (!query) return;
      const generation = ++searchGenerationRef.current;
      setIsSearching(true);
      setSearchError(null);
      setJournalHits([]);
      hubApi
        .search(config, query)
        .then((hits) => {
          if (searchGenerationRef.current !== generation) return;
          setSearchHits(hits);
        })
        .catch((err) => {
          if (searchGenerationRef.current !== generation) return;
          setSearchError(String(err));
        })
        .finally(() => {
          if (searchGenerationRef.current !== generation) return;
          setIsSearching(false);
        });
      // Journal hits are additive and best-effort: a hub without the journal
      // block (or an unreachable one) simply yields no journal section.
      hubApi
        .journalSearch(config, query)
        .then((hits) => {
          if (searchGenerationRef.current !== generation) return;
          setJournalHits(hits);
        })
        .catch(() => {
          if (searchGenerationRef.current !== generation) return;
          setJournalHits([]);
        });
    },
    [config]
  );

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
    [openSessionRef]
  );

  const handleActivateJournalHit = useCallback((hit: JournalSearchHit) => {
    setView("journal");
    setAnchorDate(hit.entry_date);
    setAnchorNonce((n) => n + 1);
  }, []);

  // `/` focuses the search input from anywhere non-editable (issue #21).
  const searchInputRef = useRef<HTMLInputElement | null>(null);
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
      searchInputRef.current?.focus();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

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
      const base = current >= 0 ? current : view === "journal" ? 0 : 1;
      const delta = e.key === "ArrowRight" ? 1 : -1;
      const next = tabs[(base + delta + tabs.length) % tabs.length]!;
      next.focus();
      next.click();
    },
    [view]
  );

  // Dismiss the current results without clearing the query input.
  const handleClearSearch = useCallback(() => {
    ++searchGenerationRef.current;
    setSearchHits(null);
    setJournalHits([]);
    setSearchError(null);
    setIsSearching(false);
  }, []);

  // Mobile drill-up: the stacked (<md) Browse shows one level at a time.
  const handleBackToProjects = useCallback(() => {
    ++sessionsGenerationRef.current;
    ++messagesGenerationRef.current;
    setSelectedGroup(null);
    setSessions([]);
    setSessionsError(null);
    setOpenSession(null);
    setMessages([]);
    setTotalCount(null);
    setWindowStart(0);
    setHighlightMessageId(null);
    setMessagesError(null);
  }, []);

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

  return (
    <div
      data-testid="archive-browser"
      className="flex flex-col h-full gap-3 overflow-hidden"
    >
      <form onSubmit={handleSearchSubmit} className="flex items-center gap-2 shrink-0">
        <input
          ref={searchInputRef}
          data-testid="archive-search-input"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          placeholder={t("settings.archiveHub.browser.searchPlaceholder")}
          aria-label={t("settings.archiveHub.browser.searchPlaceholder")}
          className="flex-1 h-9 rounded-md border border-border bg-background px-2.5 text-px14"
        />
        <button
          type="submit"
          className="h-9 shrink-0 rounded-md border border-border px-3 text-px14 hover:bg-muted"
        >
          {t("settings.archiveHub.browser.searchButton")}
        </button>
      </form>

      {/* Search results (global, above both views) */}
      {isSearching && (
        <p className="text-px13 text-muted-foreground shrink-0 flex items-center gap-1.5">
          <Loader2 className="w-3.5 h-3.5 animate-spin" aria-hidden="true" />
          {t("settings.archiveHub.browser.search.loading")}
        </p>
      )}
      {searchError && (
        <p className="text-px13 text-destructive shrink-0">{searchError}</p>
      )}
      {!isSearching && (searchHits != null || journalHits.length > 0) && (
        <div className="flex items-center justify-between shrink-0">
          <p className="text-px12 text-muted-foreground" data-testid="search-result-count">
            {t("settings.archiveHub.browser.search.count", {
              count: (searchHits?.length ?? 0) + journalHits.length,
            })}
          </p>
          <button
            type="button"
            data-testid="search-clear"
            onClick={handleClearSearch}
            aria-label={t("settings.archiveHub.browser.search.clear")}
            title={t("settings.archiveHub.browser.search.clear")}
            className="h-7 w-7 flex items-center justify-center rounded-md border border-border text-muted-foreground hover:bg-muted"
          >
            <X className="w-3.5 h-3.5" aria-hidden="true" />
          </button>
        </div>
      )}
      {journalHits.length > 0 && (
        <section
          data-testid="journal-search-section"
          className="shrink-0 space-y-1 border border-info/40 bg-info/5 rounded-md p-1"
        >
          <p className="px-1 text-px12 font-medium text-info uppercase tracking-wide">
            {t("settings.archiveHub.journal.searchSection")}
          </p>
          <ul className="space-y-1">
            {journalHits.map((hit, index) => (
              <li key={`${hit.entry_date}-${hit.project_path}-${index}`}>
                <button
                  type="button"
                  data-testid="journal-search-hit"
                  onClick={() => handleActivateJournalHit(hit)}
                  className="w-full text-left rounded px-2 py-1.5 hover:bg-muted"
                >
                  <p className="text-px14 font-medium truncate">
                    {hit.headline ?? hit.project_path}
                  </p>
                  <p className="text-px12 text-muted-foreground truncate">
                    <span>{hit.entry_date}</span>
                    {" · "}
                    <span>{hit.project_path}</span>
                  </p>
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
      {searchHits && searchHits.length === 0 && journalHits.length === 0 && !isSearching && (
        <p className="text-px13 text-muted-foreground shrink-0">
          {t("settings.archiveHub.browser.search.empty")}
        </p>
      )}
      {searchHits && searchHits.length > 0 && (
        <ul className="shrink-0 space-y-1 max-h-72 overflow-y-auto border border-border/50 rounded-md p-1">
          {searchHits.map((hit, index) => (
            <li key={`${hit.session_id}-${index}`}>
              <button
                type="button"
                onClick={() => handleActivateHit(hit)}
                className="w-full text-left rounded px-2 py-1.5 hover:bg-muted"
              >
                <p className="text-px14 truncate">{renderSnippet(hit.snippet)}</p>
                <p className="text-px12 text-muted-foreground truncate">
                  <span>{hit.project_name ?? hit.project_path}</span>
                  {" · "}
                  <span>{hit.machine_hostname}</span>
                  {hit.timestamp && (
                    <>
                      {" · "}
                      <span title={hit.timestamp}>
                        {humanizeTimestamp(hit.timestamp)}
                      </span>
                    </>
                  )}
                </p>
              </button>
            </li>
          ))}
        </ul>
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
            "ml-auto flex items-center gap-1 rounded-md border px-2 py-1 text-px12",
            showWorktrees
              ? "border-border text-foreground"
              : "border-border/50 text-muted-foreground line-through"
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
      ) : (
        <div className="flex flex-1 min-h-0 gap-3">
          {/* Projects pane: identity-grouped (one entry per repo identity,
              members inspectable on selection). Below `md` the three panes
              stack: exactly one level is visible with back buttons. */}
          <div
            className={cn(
              "w-full md:w-60 md:shrink-0 overflow-y-auto border border-border/50 rounded-md",
              (selectedGroup || openSession) && "hidden md:block"
            )}
          >
            <p className="px-2 py-1.5 text-px12 font-medium text-muted-foreground">
              {t("settings.archiveHub.browser.projects.title")}
            </p>
            {isLoadingProjects && (
              <p className="px-2 py-1 text-px14 text-muted-foreground">
                {t("settings.archiveHub.browser.projects.loading")}
              </p>
            )}
            {projectsError && (
              <p className="px-2 py-1 text-px14 text-destructive">{projectsError}</p>
            )}
            {!isLoadingProjects && !projectsError && projectGroups.length === 0 && (
              <p className="px-2 py-1 text-px14 text-muted-foreground">
                {t("settings.archiveHub.browser.projects.empty")}
              </p>
            )}
            <ul>
              {projectGroups.map((group) => {
                const isSelected = activeGroup?.key === group.key;
                const identityInfo = group.identityKey
                  ? identities.find((i) => i.identity_key === group.identityKey)
                  : undefined;
                const orphanSuggestions =
                  identityInfo?.suggestions.filter(
                    (s) => s.kind === "orphan_path" && s.project_path
                  ) ?? [];
                return (
                  <li key={group.key}>
                    <button
                      type="button"
                      data-testid="project-group"
                      onClick={() => handleSelectGroup(group)}
                      className={`w-full text-left px-2 py-2 text-px14 hover:bg-muted ${
                        isSelected ? "bg-accent/10" : ""
                      }`}
                      title={group.paths.join("\n")}
                    >
                      <p className="truncate">
                        {group.displayName}
                        {group.disambiguator && (
                          <span className="text-px12 text-muted-foreground">
                            {" — "}
                            {group.disambiguator}
                          </span>
                        )}
                      </p>
                      <p className="text-px12 text-muted-foreground truncate">
                        {group.machines.join(", ")}
                        {group.providers.map((provider) => (
                          <span
                            key={provider}
                            className="ml-1.5 rounded bg-muted px-1 py-px"
                          >
                            {getProviderLabel(t, provider)}
                          </span>
                        ))}
                        {group.worktreePaths.length > 0 && (
                          <span className="ml-1.5 rounded bg-info/10 text-info px-1 py-px">
                            {t("settings.archiveHub.identity.worktree")}
                          </span>
                        )}
                      </p>
                    </button>
                    {/* Member inspection: locations, worktree/linked labels,
                        alias link/unlink affordances. */}
                    {isSelected &&
                      (group.paths.length > 1 ||
                        orphanSuggestions.length > 0 ||
                        (identityInfo?.aliases.length ?? 0) > 0 ||
                        group.worktreePaths.length > 0) && (
                        <div
                          data-testid="identity-members"
                          className="px-2 pb-2 space-y-1"
                        >
                          <p className="text-px11 uppercase tracking-wide text-muted-foreground">
                            {t("settings.archiveHub.identity.locations")}
                          </p>
                          {group.paths.map((path) => {
                            const alias = identityInfo?.aliases.find(
                              (a) => a.project_path === path
                            );
                            return (
                              <div
                                key={path}
                                className="flex items-center gap-1.5 text-px12 text-muted-foreground"
                              >
                                <span className="truncate" title={path}>
                                  {path}
                                </span>
                                {group.worktreePaths.includes(path) && (
                                  <span className="shrink-0 rounded bg-info/10 text-info px-1 py-px">
                                    {t("settings.archiveHub.identity.worktree")}
                                  </span>
                                )}
                                {alias && (
                                  <>
                                    <span className="shrink-0 rounded bg-muted px-1 py-px">
                                      {t("settings.archiveHub.identity.linked")}
                                    </span>
                                    <button
                                      type="button"
                                      data-testid="identity-unlink"
                                      onClick={() => handleUnlinkAlias(alias.id)}
                                      className="shrink-0 rounded border border-border px-1 py-px hover:bg-muted"
                                    >
                                      {t("settings.archiveHub.identity.unlink")}
                                    </button>
                                  </>
                                )}
                              </div>
                            );
                          })}
                          {orphanSuggestions.map((suggestion) => (
                            <div
                              key={suggestion.project_path}
                              className="flex items-center gap-1.5 text-px12 text-muted-foreground"
                            >
                              <span
                                className="truncate"
                                title={`${suggestion.project_path} — ${t(
                                  "settings.archiveHub.identity.suggestionHint"
                                )}`}
                              >
                                {suggestion.project_path}
                              </span>
                              <button
                                type="button"
                                data-testid="identity-link"
                                onClick={() =>
                                  handleLinkAlias(
                                    suggestion.project_path!,
                                    group.identityKey!
                                  )
                                }
                                className="shrink-0 rounded border border-border px-1 py-px hover:bg-muted"
                              >
                                {t("settings.archiveHub.identity.link")}
                              </button>
                            </div>
                          ))}
                          {aliasError && (
                            <p className="text-px12 text-destructive">{aliasError}</p>
                          )}
                        </div>
                      )}
                  </li>
                );
              })}
            </ul>
          </div>

          {/* Sessions pane */}
          <div
            className={cn(
              "w-full md:w-80 md:shrink-0 overflow-y-auto border border-border/50 rounded-md",
              (!selectedGroup || openSession) && "hidden md:block"
            )}
          >
            <div className="flex items-center gap-1 px-2 py-1.5">
              <button
                type="button"
                data-testid="browse-back-to-projects"
                onClick={handleBackToProjects}
                className="md:hidden flex items-center gap-0.5 text-px12 text-muted-foreground hover:text-foreground"
              >
                <ChevronLeft className="w-3.5 h-3.5" aria-hidden="true" />
                {t("settings.archiveHub.browser.backToProjects")}
              </button>
              <p className="text-px12 font-medium text-muted-foreground">
                {t("settings.archiveHub.browser.sessions.title")}
              </p>
            </div>
            {!selectedGroup && (
              <p className="px-2 py-1 text-px14 text-muted-foreground">
                {t("settings.archiveHub.browser.selectProject")}
              </p>
            )}
            {selectedGroup && isLoadingSessions && (
              <p className="px-2 py-1 text-px14 text-muted-foreground">
                {t("settings.archiveHub.browser.sessions.loading")}
              </p>
            )}
            {sessionsError && (
              <p className="px-2 py-1 text-px14 text-destructive">{sessionsError}</p>
            )}
            {selectedGroup &&
              !isLoadingSessions &&
              !sessionsError &&
              sessions.length === 0 && (
                <p className="px-2 py-1 text-px14 text-muted-foreground">
                  {t("settings.archiveHub.browser.sessions.empty")}
                </p>
              )}
            <ul>
              {sessions.map((session) => (
                <li key={session.id}>
                  <button
                    type="button"
                    onClick={() =>
                      openSessionRef(session.id, session.summary ?? session.session_id)
                    }
                    className={`w-full text-left px-2 py-2 text-px14 hover:bg-muted ${
                      openSession?.ref === session.id ||
                      openSession?.ref === session.session_id
                        ? "bg-accent/10"
                        : ""
                    }`}
                  >
                    <p className="truncate">{session.summary ?? session.session_id}</p>
                    <p className="text-px12 text-muted-foreground truncate">
                      {formatCount(session.message_count)}{" "}
                      {t("settings.archiveHub.browser.sessions.messageCountUnit")}
                      {session.last_message_time
                        ? ` · ${humanizeTimestamp(session.last_message_time)}`
                        : ""}
                    </p>
                  </button>
                </li>
              ))}
            </ul>
          </div>

          {/* Messages pane */}
          <div
            className={cn(
              "flex-1 min-w-0 overflow-y-auto border border-border/50 rounded-md",
              !openSession && "hidden md:block"
            )}
          >
            {/* Header rides the same centered column as the messages — on
                ultrawide screens label and count otherwise sit 1400px apart. */}
            <div className="w-full max-w-4xl mx-auto flex items-center gap-2 px-2 py-1.5 min-w-0">
              {openSession && (
                <button
                  type="button"
                  data-testid="browse-back-from-messages"
                  onClick={handleBackFromMessages}
                  className="md:hidden flex items-center gap-0.5 shrink-0 text-px12 text-muted-foreground hover:text-foreground"
                >
                  <ChevronLeft className="w-3.5 h-3.5" aria-hidden="true" />
                  {selectedGroup
                    ? t("settings.archiveHub.browser.backToSessions")
                    : t("settings.archiveHub.browser.backToProjects")}
                </button>
              )}
              <p className="text-px12 font-medium text-muted-foreground truncate">
                {openSession?.label ?? t("settings.archiveHub.browser.selectSession")}
              </p>
              {openSession && totalCount != null && (
                <p
                  className="ml-auto shrink-0 text-px12 text-muted-foreground tabular-nums"
                  data-testid="message-progress"
                >
                  {windowStart > 0
                    ? t("settings.archiveHub.browser.messages.progressRange", {
                        from: formatCount(windowStart + 1),
                        to: formatCount(windowStart + messages.length),
                        total: formatCount(totalCount),
                      })
                    : t("settings.archiveHub.browser.messages.progress", {
                        loaded: formatCount(messages.length),
                        total: formatCount(totalCount),
                      })}
                </p>
              )}
            </div>
            {!openSession && (
              <p className="px-2 py-1 text-px14 text-muted-foreground">
                {t("settings.archiveHub.browser.selectSession")}
              </p>
            )}
            {openSession && isLoadingMessages && messages.length === 0 && (
              <p className="px-2 py-1 text-px14 text-muted-foreground">
                {t("settings.archiveHub.browser.messages.loading")}
              </p>
            )}
            {messagesError && (
              <p className="px-2 py-1 text-px14 text-destructive">{messagesError}</p>
            )}
            {openSession &&
              !isLoadingMessages &&
              !messagesError &&
              messages.length === 0 && (
                <p className="px-2 py-1 text-px14 text-muted-foreground">
                  {t("settings.archiveHub.browser.messages.empty")}
                </p>
              )}
            {openSession && windowStart > 0 && (
              <div className="px-2 pt-1 w-full max-w-4xl mx-auto">
                <button
                  type="button"
                  data-testid="archive-load-earlier"
                  onClick={handleLoadEarlier}
                  disabled={isLoadingMessages}
                  className="w-full rounded-md border border-border px-3 py-2 text-px14 hover:bg-muted disabled:opacity-50"
                >
                  {t("settings.archiveHub.browser.messages.loadEarlier")}
                </button>
              </div>
            )}
            {/* Reading-measure column: don't span the full pane on wide screens. */}
            <ArchiveRenderContext.Provider value={true}>
              <div className="px-2 py-1 space-y-1 w-full max-w-4xl mx-auto">
                {messages.map((row, index) => {
                  const role = row.role ?? row.message_type;
                  // Role/timestamp gutter at turn boundaries only, and only
                  // for real conversation roles: record types like
                  // `attachment`/`mode` interleave constantly and would strew
                  // noise gutters between every real turn (they also must not
                  // RESET the turn, so compare against the last real role).
                  const isConversationRole =
                    role != null && GUTTER_ROLES.has(role);
                  const showGutter =
                    isConversationRole &&
                    role !== lastConversationRole(messages, index);
                  return (
                    <div
                      key={row.id}
                      data-msg-id={row.id}
                      className={cn(
                        row.id === highlightMessageId &&
                          "ring-2 ring-accent/70 rounded-md"
                      )}
                    >
                      {showGutter && (
                        <div
                          data-testid="message-gutter"
                          className="flex items-baseline gap-2 pt-2 pb-0.5 text-px12 text-muted-foreground"
                        >
                          <span className="font-medium">{roleLabel(role, t)}</span>
                          {row.timestamp && (
                            <span title={row.timestamp}>
                              {humanizeTimestamp(row.timestamp)}
                            </span>
                          )}
                        </div>
                      )}
                      <ArchivedMessage
                        row={row}
                        sessionId={String(openSession?.ref ?? "")}
                      />
                    </div>
                  );
                })}
              </div>
            </ArchiveRenderContext.Provider>
            {hasMoreMessages && (
              <div className="px-2 pb-2 w-full max-w-4xl mx-auto">
                <button
                  type="button"
                  data-testid="archive-load-more"
                  onClick={handleLoadMore}
                  disabled={isLoadingMessages}
                  className="w-full rounded-md border border-border px-3 py-2 text-px14 hover:bg-muted disabled:opacity-50"
                >
                  {isLoadingMessages ? (
                    <>
                      <Loader2
                        className="w-3.5 h-3.5 mx-auto animate-spin"
                        aria-hidden="true"
                      />
                      <span className="sr-only">{t("common.loading")}</span>
                    </>
                  ) : (
                    t("settings.archiveHub.browser.messages.loadMore")
                  )}
                </button>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
