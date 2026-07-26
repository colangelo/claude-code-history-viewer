/**
 * Browse's second pane: the selected project group's sessions. Takes
 * `hasSelection` rather than the group itself — nothing here reads the
 * group's fields, only whether one is selected, which decides between the
 * "pick a project" prompt and the list's own empty/loading states.
 */

import { ChevronLeft } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import { formatCount, humanizeTimestamp } from "@/utils/journalFormat";
import type { HubSession } from "../../services/hubApi";

export interface SessionsPaneProps {
  sessions: HubSession[];
  /** Whether a project group is selected at all. */
  hasSelection: boolean;
  /** The open session's ref, matched against each row to mark the selection. */
  openSessionRef: number | string | null;
  isLoading: boolean;
  error: string | null;
  /** Collapsed by the mobile drill-down (a deeper level is showing). */
  hidden: boolean;
  onBackToProjects: () => void;
  onOpenSession: (ref: number | string, label: string) => void;
}

export function SessionsPane({
  sessions,
  hasSelection,
  openSessionRef,
  isLoading,
  error,
  hidden,
  onBackToProjects,
  onOpenSession,
}: SessionsPaneProps) {
  const { t } = useTranslation();

  return (
    <div
      className={cn(
        "w-full md:w-80 md:shrink-0 overflow-y-auto border border-border/50 rounded-md",
        hidden && "hidden md:block"
      )}
    >
      <div className="flex items-center gap-1 px-2 py-1.5">
        <button
          type="button"
          data-testid="browse-back-to-projects"
          onClick={onBackToProjects}
          className="md:hidden flex items-center gap-0.5 text-px12 text-muted-foreground hover:text-foreground"
        >
          <ChevronLeft className="w-3.5 h-3.5" aria-hidden="true" />
          {t("settings.archiveHub.browser.backToProjects")}
        </button>
        <p className="text-px12 font-medium text-muted-foreground">
          {t("settings.archiveHub.browser.sessions.title")}
        </p>
      </div>
      {!hasSelection && (
        <p className="px-2 py-1 text-px14 text-muted-foreground">
          {t("settings.archiveHub.browser.selectProject")}
        </p>
      )}
      {hasSelection && isLoading && (
        <p className="px-2 py-1 text-px14 text-muted-foreground">
          {t("settings.archiveHub.browser.sessions.loading")}
        </p>
      )}
      {error && <p className="px-2 py-1 text-px14 text-destructive">{error}</p>}
      {hasSelection && !isLoading && !error && sessions.length === 0 && (
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
                onOpenSession(session.id, session.summary ?? session.session_id)
              }
              className={`w-full text-left px-2 py-2 text-px14 hover:bg-muted ${
                openSessionRef === session.id ||
                openSessionRef === session.session_id
                  ? "bg-accent/15 dark:bg-accent/25"
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
  );
}
