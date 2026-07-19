/**
 * Archive browser: browse and search the cross-machine hub archive via
 * `services/hubApi.ts`. Presents two views behind a tab switcher —
 * **Journal** (the default landing view: a distilled day-timeline feed) and
 * **Browse** (projects → sessions → messages) — with a global search bar above
 * both. Rendered as its own mode: archived history spans machines and outlives
 * local retention, so it is presented separately from the local provider tree,
 * with provenance (machine hostname) visible.
 */

import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { ChevronLeft, Loader2, X } from "lucide-react";
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
  formatArchiveHash,
  parseArchiveHash,
  type ArchiveRoute,
} from "./archiveRoute";
import {
  hubApi,
  hubMessageToClaudeMessage,
  type HubConfig,
  type HubProject,
  type HubSession,
  type HubMessage,
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
  const [isLoadingProjects, setIsLoadingProjects] = useState(false);
  const [projectsError, setProjectsError] = useState<string | null>(null);
  const [selectedProject, setSelectedProject] = useState<HubProject | null>(null);

  const [sessions, setSessions] = useState<HubSession[]>([]);
  const [isLoadingSessions, setIsLoadingSessions] = useState(false);
  const [sessionsError, setSessionsError] = useState<string | null>(null);

  const [openSession, setOpenSession] = useState<OpenSession | null>(null);
  const [messages, setMessages] = useState<HubMessage[]>([]);
  const [totalCount, setTotalCount] = useState<number | null>(null);
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

  const fetchSessionsFor = useCallback(
    (project: HubProject) => {
      const generation = ++sessionsGenerationRef.current;
      setSessions([]);
      setSessionsError(null);
      setIsLoadingSessions(true);
      hubApi
        .listSessions(config, {
          project: project.name ?? project.project_path,
          machine: project.machine_hostname,
          provider: project.provider,
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

  // Select a project WITHOUT clearing the open session — pane sync for a
  // session opened from Journal or a search hit.
  const syncProjectSelection = useCallback(
    (project: HubProject) => {
      setSelectedProject(project);
      fetchSessionsFor(project);
    },
    [fetchSessionsFor]
  );

  const openSessionRef = useCallback(
    (ref: number | string, label: string, context?: SessionOpenContext) => {
      const generation = ++messagesGenerationRef.current;
      setOpenSession({ ref, label });
      setMessages([]);
      setTotalCount(null);
      setMessagesError(null);
      setIsLoadingMessages(true);
      hubApi
        .sessionMessages(config, ref, { limit: PAGE_SIZE, offset: 0 })
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
      // Pane sync: only when the context pins exactly one known project —
      // an ambiguous match (same path on several machines) syncs nothing.
      if (context?.project_path) {
        const matches = projects.filter(
          (p) =>
            p.project_path === context.project_path &&
            (context.machine_hostname == null ||
              p.machine_hostname === context.machine_hostname) &&
            (context.provider == null || p.provider === context.provider)
        );
        if (matches.length === 1 && matches[0]!.id !== selectedProject?.id) {
          syncProjectSelection(matches[0]!);
        }
      }
    },
    [config, projects, selectedProject?.id, syncProjectSelection]
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

  const handleSelectProject = useCallback(
    (project: HubProject) => {
      // Invalidate any in-flight message fetch for the previously open
      // session — it belongs to a project we're navigating away from.
      ++messagesGenerationRef.current;
      setSelectedProject(project);
      setOpenSession(null);
      setMessages([]);
      setTotalCount(null);
      setMessagesError(null);
      fetchSessionsFor(project);
    },
    [fetchSessionsFor]
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
        offset: messages.length,
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
  }, [config, openSession, messages.length, isLoadingMessages]);

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
      openSessionRef(hit.session_id, hit.session_summary ?? hit.session_id, {
        project_path: hit.project_path,
        machine_hostname: hit.machine_hostname,
        provider: hit.provider,
      });
    },
    [openSessionRef]
  );

  const handleActivateJournalHit = useCallback((hit: JournalSearchHit) => {
    setView("journal");
    setAnchorDate(hit.entry_date);
    setAnchorNonce((n) => n + 1);
  }, []);

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
    setSelectedProject(null);
    setSessions([]);
    setSessionsError(null);
    setOpenSession(null);
    setMessages([]);
    setTotalCount(null);
    setMessagesError(null);
  }, []);

  const handleBackFromMessages = useCallback(() => {
    ++messagesGenerationRef.current;
    setOpenSession(null);
    setMessages([]);
    setTotalCount(null);
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

  const hasMoreMessages = totalCount !== null && messages.length < totalCount;

  return (
    <div
      data-testid="archive-browser"
      className="flex flex-col h-full gap-3 overflow-hidden"
    >
      <form onSubmit={handleSearchSubmit} className="flex items-center gap-2 shrink-0">
        <input
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
                </p>
              </button>
            </li>
          ))}
        </ul>
      )}

      {/* View switcher */}
      <div
        role="tablist"
        aria-label={t("settings.archiveHub.journal.tabsLabel")}
        className="flex items-center gap-1 shrink-0 border-b border-border/50"
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

      {view === "journal" ? (
        <JournalView
          config={config}
          anchorDate={anchorDate}
          anchorNonce={anchorNonce}
          onOpenSession={handleOpenSessionFromJournal}
          onDateChange={setJournalDate}
        />
      ) : (
        <div className="flex flex-1 min-h-0 gap-3">
          {/* Projects pane. Below `md` the three panes stack: exactly one level
              is visible (messages > sessions > projects) with back buttons. */}
          <div
            className={cn(
              "w-full md:w-60 md:shrink-0 overflow-y-auto border border-border/50 rounded-md",
              (selectedProject || openSession) && "hidden md:block"
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
            {!isLoadingProjects && !projectsError && projects.length === 0 && (
              <p className="px-2 py-1 text-px14 text-muted-foreground">
                {t("settings.archiveHub.browser.projects.empty")}
              </p>
            )}
            <ul>
              {projects.map((project) => (
                <li key={project.id}>
                  <button
                    type="button"
                    onClick={() => handleSelectProject(project)}
                    className={`w-full text-left px-2 py-2 text-px14 hover:bg-muted ${
                      selectedProject?.id === project.id ? "bg-accent/10" : ""
                    }`}
                  >
                    <p className="truncate">{project.name ?? project.project_path}</p>
                    <p className="text-px12 text-muted-foreground truncate">
                      {project.machine_hostname}
                      <span className="ml-1.5 rounded bg-muted px-1 py-px">
                        {getProviderLabel(t, project.provider)}
                      </span>
                    </p>
                  </button>
                </li>
              ))}
            </ul>
          </div>

          {/* Sessions pane */}
          <div
            className={cn(
              "w-full md:w-80 md:shrink-0 overflow-y-auto border border-border/50 rounded-md",
              (!selectedProject || openSession) && "hidden md:block"
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
            {!selectedProject && (
              <p className="px-2 py-1 text-px14 text-muted-foreground">
                {t("settings.archiveHub.browser.selectProject")}
              </p>
            )}
            {selectedProject && isLoadingSessions && (
              <p className="px-2 py-1 text-px14 text-muted-foreground">
                {t("settings.archiveHub.browser.sessions.loading")}
              </p>
            )}
            {sessionsError && (
              <p className="px-2 py-1 text-px14 text-destructive">{sessionsError}</p>
            )}
            {selectedProject &&
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
            <div className="flex items-center gap-2 px-2 py-1.5 min-w-0">
              {openSession && (
                <button
                  type="button"
                  data-testid="browse-back-from-messages"
                  onClick={handleBackFromMessages}
                  className="md:hidden flex items-center gap-0.5 shrink-0 text-px12 text-muted-foreground hover:text-foreground"
                >
                  <ChevronLeft className="w-3.5 h-3.5" aria-hidden="true" />
                  {selectedProject
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
                  {t("settings.archiveHub.browser.messages.progress", {
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
            {/* Reading-measure column: don't span the full pane on wide screens. */}
            <ArchiveRenderContext.Provider value={true}>
              <div className="px-2 py-1 space-y-1 w-full max-w-4xl mx-auto">
                {messages.map((row, index) => {
                  const prev = index > 0 ? messages[index - 1] : undefined;
                  const role = row.role ?? row.message_type;
                  // Role/timestamp gutter at turn boundaries only — a header
                  // on every row would drown the content.
                  const showGutter =
                    role != null && role !== (prev?.role ?? prev?.message_type);
                  return (
                    <div key={row.id}>
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
