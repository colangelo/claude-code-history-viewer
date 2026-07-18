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
import { Loader2 } from "lucide-react";
import { ExpandKeyProvider } from "@/contexts/CaptureExpandContext";
import { MessageContentDisplay } from "@/components/messageRenderer";
import { ClaudeContentArrayRenderer } from "@/components/contentRenderer";
import { cn } from "@/lib/utils";
import { formatCount, humanizeTimestamp } from "@/utils/journalFormat";
import { JournalView } from "./JournalView";
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
}

/** Hub's max page size (`crates/hub/src/pagination.rs::MAX_LIMIT`). */
const PAGE_SIZE = 200;

type ArchiveView = "journal" | "browse";

interface OpenSession {
  ref: number | string;
  label: string;
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

export function ArchiveBrowser({ config }: ArchiveBrowserProps) {
  const { t } = useTranslation();

  const [view, setView] = useState<ArchiveView>("journal");
  // Anchor requested from a journal search hit; `nonce` re-triggers a jump even
  // when the date is unchanged.
  const [anchorDate, setAnchorDate] = useState<string | null>(null);
  const [anchorNonce, setAnchorNonce] = useState(0);

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

  const [searchQuery, setSearchQuery] = useState("");
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

  const openSessionRef = useCallback(
    (ref: number | string, label: string) => {
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
    },
    [config]
  );

  // Open a session from the Journal view: switch to Browse and load messages
  // through the existing message-fetch path.
  const handleOpenSessionFromJournal = useCallback(
    (sessionId: number, label: string) => {
      setView("browse");
      openSessionRef(sessionId, label);
    },
    [openSessionRef]
  );

  const handleSelectProject = useCallback(
    (project: HubProject) => {
      const generation = ++sessionsGenerationRef.current;
      // Invalidate any in-flight message fetch for the previously open
      // session — it belongs to a project we're navigating away from.
      ++messagesGenerationRef.current;
      setSelectedProject(project);
      setOpenSession(null);
      setMessages([]);
      setTotalCount(null);
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

  const handleSearchSubmit = useCallback(
    (e: FormEvent) => {
      e.preventDefault();
      const query = searchQuery.trim();
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
    [config, searchQuery]
  );

  const handleActivateHit = useCallback(
    (hit: HubSearchHit) => {
      openSessionRef(hit.session_id, hit.session_summary ?? hit.session_id);
    },
    [openSessionRef]
  );

  const handleActivateJournalHit = useCallback((hit: JournalSearchHit) => {
    setView("journal");
    setAnchorDate(hit.entry_date);
    setAnchorNonce((n) => n + 1);
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
          className="flex-1 h-8 rounded-md border border-border bg-background px-2 text-px13"
        />
        <button
          type="submit"
          className="h-8 shrink-0 rounded-md border border-border px-3 text-px13 hover:bg-muted"
        >
          {t("settings.archiveHub.browser.searchButton")}
        </button>
      </form>

      {/* Search results (global, above both views) */}
      {isSearching && (
        <p className="text-px12 text-muted-foreground shrink-0">
          {t("settings.archiveHub.browser.search.loading")}
        </p>
      )}
      {searchError && (
        <p className="text-px12 text-destructive shrink-0">{searchError}</p>
      )}
      {journalHits.length > 0 && (
        <section
          data-testid="journal-search-section"
          className="shrink-0 space-y-1 border border-info/40 bg-info/5 rounded-md p-1"
        >
          <p className="px-1 text-px11 font-medium text-info uppercase tracking-wide">
            {t("settings.archiveHub.journal.searchSection")}
          </p>
          <ul className="space-y-1">
            {journalHits.map((hit, index) => (
              <li key={`${hit.entry_date}-${hit.project_path}-${index}`}>
                <button
                  type="button"
                  data-testid="journal-search-hit"
                  onClick={() => handleActivateJournalHit(hit)}
                  className="w-full text-left rounded px-2 py-1 hover:bg-muted"
                >
                  <p className="text-px13 font-medium truncate">
                    {hit.headline ?? hit.project_path}
                  </p>
                  <p className="text-px11 text-muted-foreground truncate">
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
        <p className="text-px12 text-muted-foreground shrink-0">
          {t("settings.archiveHub.browser.search.empty")}
        </p>
      )}
      {searchHits && searchHits.length > 0 && (
        <ul className="shrink-0 space-y-1 max-h-40 overflow-y-auto border border-border/50 rounded-md p-1">
          {searchHits.map((hit, index) => (
            <li key={`${hit.session_id}-${index}`}>
              <button
                type="button"
                onClick={() => handleActivateHit(hit)}
                className="w-full text-left rounded px-2 py-1 hover:bg-muted"
              >
                <p className="text-px13 truncate">{hit.snippet}</p>
                <p className="text-px11 text-muted-foreground truncate">
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
            "px-3 py-1.5 text-px13 border-b-2 -mb-px",
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
            "px-3 py-1.5 text-px13 border-b-2 -mb-px",
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
        />
      ) : (
        <div className="flex flex-1 min-h-0 gap-3">
          {/* Projects pane */}
          <div className="w-60 shrink-0 overflow-y-auto border border-border/50 rounded-md">
            <p className="px-2 py-1.5 text-px11 font-medium text-muted-foreground">
              {t("settings.archiveHub.browser.projects.title")}
            </p>
            {isLoadingProjects && (
              <p className="px-2 py-1 text-px13 text-muted-foreground">
                {t("settings.archiveHub.browser.projects.loading")}
              </p>
            )}
            {projectsError && (
              <p className="px-2 py-1 text-px13 text-destructive">{projectsError}</p>
            )}
            {!isLoadingProjects && !projectsError && projects.length === 0 && (
              <p className="px-2 py-1 text-px13 text-muted-foreground">
                {t("settings.archiveHub.browser.projects.empty")}
              </p>
            )}
            <ul>
              {projects.map((project) => (
                <li key={project.id}>
                  <button
                    type="button"
                    onClick={() => handleSelectProject(project)}
                    className={`w-full text-left px-2 py-1.5 text-px13 hover:bg-muted ${
                      selectedProject?.id === project.id ? "bg-accent/10" : ""
                    }`}
                  >
                    <p className="truncate">{project.name ?? project.project_path}</p>
                    <p className="text-px11 text-muted-foreground truncate">
                      {project.machine_hostname}
                    </p>
                  </button>
                </li>
              ))}
            </ul>
          </div>

          {/* Sessions pane */}
          <div className="w-80 shrink-0 overflow-y-auto border border-border/50 rounded-md">
            <p className="px-2 py-1.5 text-px11 font-medium text-muted-foreground">
              {t("settings.archiveHub.browser.sessions.title")}
            </p>
            {!selectedProject && (
              <p className="px-2 py-1 text-px13 text-muted-foreground">
                {t("settings.archiveHub.browser.selectProject")}
              </p>
            )}
            {selectedProject && isLoadingSessions && (
              <p className="px-2 py-1 text-px13 text-muted-foreground">
                {t("settings.archiveHub.browser.sessions.loading")}
              </p>
            )}
            {sessionsError && (
              <p className="px-2 py-1 text-px13 text-destructive">{sessionsError}</p>
            )}
            {selectedProject &&
              !isLoadingSessions &&
              !sessionsError &&
              sessions.length === 0 && (
                <p className="px-2 py-1 text-px13 text-muted-foreground">
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
                    className={`w-full text-left px-2 py-1.5 text-px13 hover:bg-muted ${
                      openSession?.ref === session.id ? "bg-accent/10" : ""
                    }`}
                  >
                    <p className="truncate">{session.summary ?? session.session_id}</p>
                    <p className="text-px11 text-muted-foreground truncate">
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
          <div className="flex-1 min-w-0 overflow-y-auto border border-border/50 rounded-md">
            <p className="px-2 py-1.5 text-px11 font-medium text-muted-foreground truncate">
              {openSession?.label ?? t("settings.archiveHub.browser.selectSession")}
            </p>
            {!openSession && (
              <p className="px-2 py-1 text-px13 text-muted-foreground">
                {t("settings.archiveHub.browser.selectSession")}
              </p>
            )}
            {openSession && isLoadingMessages && messages.length === 0 && (
              <p className="px-2 py-1 text-px13 text-muted-foreground">
                {t("settings.archiveHub.browser.messages.loading")}
              </p>
            )}
            {messagesError && (
              <p className="px-2 py-1 text-px13 text-destructive">{messagesError}</p>
            )}
            {openSession &&
              !isLoadingMessages &&
              !messagesError &&
              messages.length === 0 && (
                <p className="px-2 py-1 text-px13 text-muted-foreground">
                  {t("settings.archiveHub.browser.messages.empty")}
                </p>
              )}
            <div className="px-2 py-1 space-y-1">
              {messages.map((row) => (
                <ArchivedMessage
                  key={row.id}
                  row={row}
                  sessionId={String(openSession?.ref ?? "")}
                />
              ))}
            </div>
            {hasMoreMessages && (
              <div className="px-2 pb-2">
                <button
                  type="button"
                  data-testid="archive-load-more"
                  onClick={handleLoadMore}
                  disabled={isLoadingMessages}
                  className="w-full rounded-md border border-border px-3 py-1.5 text-px13 hover:bg-muted disabled:opacity-50"
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
