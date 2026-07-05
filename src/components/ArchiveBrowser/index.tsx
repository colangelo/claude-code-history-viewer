/**
 * Archive browser: browse and search the cross-machine hub archive
 * (projects → sessions → messages, plus full-text search) via
 * `services/hubApi.ts`. Rendered as its own mode — archived history spans
 * machines and outlives local retention, so it is presented separately from
 * the local provider tree, with provenance (machine hostname) visible.
 */

import { useCallback, useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";
import { ExpandKeyProvider } from "@/contexts/CaptureExpandContext";
import { MessageContentDisplay } from "@/components/messageRenderer";
import { extractClaudeMessageContent } from "@/utils/messageUtils";
import {
  hubApi,
  hubMessageToClaudeMessage,
  type HubConfig,
  type HubProject,
  type HubSession,
  type HubMessage,
  type HubSearchHit,
} from "../../services/hubApi";

export interface ArchiveBrowserProps {
  /** Hub connection; callers normally derive this from user settings. */
  config: HubConfig;
}

/** Hub's max page size (`crates/hub/src/pagination.rs::MAX_LIMIT`). */
const PAGE_SIZE = 200;

interface OpenSession {
  ref: number | string;
  label: string;
}

export function ArchiveBrowser({ config }: ArchiveBrowserProps) {
  const { t } = useTranslation();

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
  const [isSearching, setIsSearching] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);

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
      setOpenSession({ ref, label });
      setMessages([]);
      setTotalCount(null);
      setMessagesError(null);
      setIsLoadingMessages(true);
      hubApi
        .sessionMessages(config, ref, { limit: PAGE_SIZE, offset: 0 })
        .then((page) => {
          setMessages(page.messages);
          setTotalCount(page.totalCount);
        })
        .catch((err) => setMessagesError(String(err)))
        .finally(() => setIsLoadingMessages(false));
    },
    [config]
  );

  const handleSelectProject = useCallback(
    (project: HubProject) => {
      setSelectedProject(project);
      setOpenSession(null);
      setMessages([]);
      setTotalCount(null);
      setSessions([]);
      setSessionsError(null);
      setIsLoadingSessions(true);
      hubApi
        .listSessions(config, { project: project.name ?? project.project_path })
        .then((list) => setSessions(list))
        .catch((err) => setSessionsError(String(err)))
        .finally(() => setIsLoadingSessions(false));
    },
    [config]
  );

  const handleLoadMore = useCallback(() => {
    if (!openSession) return;
    setIsLoadingMessages(true);
    hubApi
      .sessionMessages(config, openSession.ref, {
        limit: PAGE_SIZE,
        offset: messages.length,
      })
      .then((page) => {
        setMessages((prev) => [...prev, ...page.messages]);
        setTotalCount(page.totalCount);
      })
      .catch((err) => setMessagesError(String(err)))
      .finally(() => setIsLoadingMessages(false));
  }, [config, openSession, messages.length]);

  const handleSearchSubmit = useCallback(
    (e: FormEvent) => {
      e.preventDefault();
      const query = searchQuery.trim();
      if (!query) return;
      setIsSearching(true);
      setSearchError(null);
      hubApi
        .search(config, query)
        .then((hits) => setSearchHits(hits))
        .catch((err) => setSearchError(String(err)))
        .finally(() => setIsSearching(false));
    },
    [config, searchQuery]
  );

  const handleActivateHit = useCallback(
    (hit: HubSearchHit) => {
      openSessionRef(hit.session_id, hit.session_summary ?? hit.session_id);
    },
    [openSessionRef]
  );

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
          placeholder={t("archiveHub.browser.searchPlaceholder")}
          aria-label={t("archiveHub.browser.searchPlaceholder")}
          className="flex-1 h-8 rounded-md border border-border bg-background px-2 text-xs"
        />
        <button
          type="submit"
          className="h-8 shrink-0 rounded-md border border-border px-3 text-xs hover:bg-muted"
        >
          {t("archiveHub.browser.searchButton")}
        </button>
      </form>

      {isSearching && (
        <p className="text-xs text-muted-foreground shrink-0">
          {t("archiveHub.browser.search.loading")}
        </p>
      )}
      {searchError && (
        <p className="text-xs text-destructive shrink-0">{searchError}</p>
      )}
      {searchHits && searchHits.length === 0 && !isSearching && (
        <p className="text-xs text-muted-foreground shrink-0">
          {t("archiveHub.browser.search.empty")}
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
                <p className="text-xs truncate">{hit.snippet}</p>
                <p className="text-2xs text-muted-foreground truncate">
                  <span>{hit.project_name ?? hit.project_path}</span>
                  {" · "}
                  <span>{hit.machine_hostname}</span>
                </p>
              </button>
            </li>
          ))}
        </ul>
      )}

      <div className="flex flex-1 min-h-0 gap-3">
        {/* Projects pane */}
        <div className="w-48 shrink-0 overflow-y-auto border border-border/50 rounded-md">
          <p className="px-2 py-1.5 text-2xs font-medium text-muted-foreground">
            {t("archiveHub.browser.projects.title")}
          </p>
          {isLoadingProjects && (
            <p className="px-2 py-1 text-xs text-muted-foreground">
              {t("archiveHub.browser.projects.loading")}
            </p>
          )}
          {projectsError && (
            <p className="px-2 py-1 text-xs text-destructive">{projectsError}</p>
          )}
          {!isLoadingProjects && !projectsError && projects.length === 0 && (
            <p className="px-2 py-1 text-xs text-muted-foreground">
              {t("archiveHub.browser.projects.empty")}
            </p>
          )}
          <ul>
            {projects.map((project) => (
              <li key={project.id}>
                <button
                  type="button"
                  onClick={() => handleSelectProject(project)}
                  className={`w-full text-left px-2 py-1.5 text-xs hover:bg-muted ${
                    selectedProject?.id === project.id ? "bg-accent/10" : ""
                  }`}
                >
                  <p className="truncate">{project.name ?? project.project_path}</p>
                  <p className="text-2xs text-muted-foreground truncate">
                    {project.machine_hostname}
                  </p>
                </button>
              </li>
            ))}
          </ul>
        </div>

        {/* Sessions pane */}
        <div className="w-64 shrink-0 overflow-y-auto border border-border/50 rounded-md">
          <p className="px-2 py-1.5 text-2xs font-medium text-muted-foreground">
            {t("archiveHub.browser.sessions.title")}
          </p>
          {!selectedProject && (
            <p className="px-2 py-1 text-xs text-muted-foreground">
              {t("archiveHub.browser.selectProject")}
            </p>
          )}
          {selectedProject && isLoadingSessions && (
            <p className="px-2 py-1 text-xs text-muted-foreground">
              {t("archiveHub.browser.sessions.loading")}
            </p>
          )}
          {sessionsError && (
            <p className="px-2 py-1 text-xs text-destructive">{sessionsError}</p>
          )}
          {selectedProject &&
            !isLoadingSessions &&
            !sessionsError &&
            sessions.length === 0 && (
              <p className="px-2 py-1 text-xs text-muted-foreground">
                {t("archiveHub.browser.sessions.empty")}
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
                  className={`w-full text-left px-2 py-1.5 text-xs hover:bg-muted ${
                    openSession?.ref === session.id ? "bg-accent/10" : ""
                  }`}
                >
                  <p className="truncate">{session.summary ?? session.session_id}</p>
                  <p className="text-2xs text-muted-foreground truncate">
                    {session.message_count} {t("archiveHub.browser.sessions.messageCountUnit")}
                    {session.last_message_time ? ` · ${session.last_message_time}` : ""}
                  </p>
                </button>
              </li>
            ))}
          </ul>
        </div>

        {/* Messages pane */}
        <div className="flex-1 min-w-0 overflow-y-auto border border-border/50 rounded-md">
          <p className="px-2 py-1.5 text-2xs font-medium text-muted-foreground truncate">
            {openSession?.label ?? t("archiveHub.browser.selectSession")}
          </p>
          {!openSession && (
            <p className="px-2 py-1 text-xs text-muted-foreground">
              {t("archiveHub.browser.selectSession")}
            </p>
          )}
          {openSession && isLoadingMessages && messages.length === 0 && (
            <p className="px-2 py-1 text-xs text-muted-foreground">
              {t("archiveHub.browser.messages.loading")}
            </p>
          )}
          {messagesError && (
            <p className="px-2 py-1 text-xs text-destructive">{messagesError}</p>
          )}
          {openSession &&
            !isLoadingMessages &&
            !messagesError &&
            messages.length === 0 && (
              <p className="px-2 py-1 text-xs text-muted-foreground">
                {t("archiveHub.browser.messages.empty")}
              </p>
            )}
          <div className="px-2 py-1 space-y-1">
            {messages.map((row) => {
              const claudeMessage = hubMessageToClaudeMessage(
                row,
                String(openSession?.ref ?? "")
              );
              const text = extractClaudeMessageContent(claudeMessage);
              return (
                <ExpandKeyProvider key={row.id} value={claudeMessage.uuid}>
                  <MessageContentDisplay content={text} messageType={claudeMessage.type} />
                </ExpandKeyProvider>
              );
            })}
          </div>
          {hasMoreMessages && (
            <div className="px-2 pb-2">
              <button
                type="button"
                data-testid="archive-load-more"
                onClick={handleLoadMore}
                disabled={isLoadingMessages}
                className="w-full rounded-md border border-border px-3 py-1.5 text-xs hover:bg-muted disabled:opacity-50"
              >
                {isLoadingMessages ? (
                  <Loader2 className="w-3.5 h-3.5 mx-auto animate-spin" />
                ) : (
                  t("archiveHub.browser.messages.loadMore")
                )}
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
