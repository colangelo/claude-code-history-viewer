/**
 * One journal entry rendered as a rich card. At rest it shows the entry's
 * meta (project, session count, model), headline, a 2-line-clamped summary and
 * topic chips. Expanding reveals the full summary, any open questions, and one
 * link per session id. Session labels resolve lazily on first expand via the
 * parent-provided `resolveSessions` (one `GET /v1/sessions` per project,
 * cached) — never during feed rendering.
 */

import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";
import type { JournalEntry, HubSession } from "../../services/hubApi";

interface JournalEntryCardProps {
  entry: JournalEntry;
  /** Resolve (and cache) the sessions for a project path. Called on expand. */
  resolveSessions: (projectPath: string) => Promise<HubSession[]>;
  /** Open a session in the Browse view. */
  onOpenSession: (sessionId: number, label: string) => void;
}

export function JournalEntryCard({
  entry,
  resolveSessions,
  onOpenSession,
}: JournalEntryCardProps) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  // `null` = not yet resolved; an array (possibly empty) once resolution ran.
  const [sessions, setSessions] = useState<HubSession[] | null>(null);

  const toggle = useCallback(() => {
    const next = !expanded;
    setExpanded(next);
    // Resolve session labels lazily, once, on first expand.
    if (next && sessions === null) {
      resolveSessions(entry.project_path)
        .then(setSessions)
        .catch(() => setSessions([]));
    }
  }, [expanded, sessions, entry.project_path, resolveSessions]);

  const sessionLabel = (id: number): string => {
    const match = sessions?.find((s) => s.id === id);
    return match?.summary ?? String(id);
  };

  return (
    <div
      data-testid="journal-entry-card"
      className="rounded-md border border-border/60 bg-card/40 p-3 space-y-2"
    >
      {/* Meta row */}
      <div className="flex items-center gap-2 text-px11 text-muted-foreground">
        <span className="font-mono truncate">{entry.project_path}</span>
        <span aria-hidden="true">·</span>
        <span>
          {t("settings.archiveHub.journal.sessionCount", {
            count: entry.session_ids.length,
          })}
        </span>
        {entry.model && (
          <>
            <span aria-hidden="true">·</span>
            <span className="truncate">{entry.model}</span>
          </>
        )}
      </div>

      {/* Headline + expand toggle */}
      <button
        type="button"
        data-testid="journal-entry-toggle"
        onClick={toggle}
        aria-expanded={expanded}
        aria-label={
          expanded
            ? t("settings.archiveHub.journal.collapse")
            : t("settings.archiveHub.journal.expand")
        }
        className="w-full flex items-start gap-1.5 text-left"
      >
        <ChevronRight
          className={cn(
            "w-4 h-4 mt-0.5 shrink-0 text-muted-foreground transition-transform",
            expanded && "rotate-90"
          )}
          aria-hidden="true"
        />
        <span className="text-px14 font-medium leading-snug">
          {entry.headline ?? entry.project_path}
        </span>
      </button>

      {/* Summary — clamped at rest, full when expanded */}
      {entry.summary && (
        <p
          className={cn(
            "text-px13 text-foreground/80 leading-relaxed",
            !expanded && "line-clamp-2"
          )}
        >
          {entry.summary}
        </p>
      )}

      {/* Topic chips */}
      {entry.topics.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {entry.topics.map((topic) => (
            <span
              key={topic}
              className="rounded-full bg-muted px-2 py-0.5 text-px11 text-muted-foreground"
            >
              {topic}
            </span>
          ))}
        </div>
      )}

      {/* Expanded detail */}
      {expanded && (
        <div className="space-y-3 pt-1">
          {entry.open_questions.length > 0 && (
            <div className="space-y-1">
              <p className="text-px11 font-medium text-muted-foreground uppercase tracking-wide">
                {t("settings.archiveHub.journal.openQuestions")}
              </p>
              <ul className="list-disc pl-4 space-y-0.5 text-px13 text-foreground/80">
                {entry.open_questions.map((q, i) => (
                  <li key={i}>{q}</li>
                ))}
              </ul>
            </div>
          )}

          <div className="space-y-1">
            <p className="text-px11 font-medium text-muted-foreground uppercase tracking-wide">
              {t("settings.archiveHub.journal.sessions")}
            </p>
            <ul className="space-y-1">
              {entry.session_ids.map((id) => {
                const match = sessions?.find((s) => s.id === id);
                return (
                  <li key={id}>
                    <button
                      type="button"
                      data-testid="journal-session-link"
                      onClick={() => onOpenSession(id, sessionLabel(id))}
                      className="w-full text-left rounded px-2 py-1 text-px13 hover:bg-muted"
                    >
                      <span className="truncate">{sessionLabel(id)}</span>
                      {match && (
                        <span className="text-px11 text-muted-foreground">
                          {" · "}
                          {t("settings.archiveHub.journal.sessionMessages", {
                            count: match.message_count,
                          })}
                        </span>
                      )}
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        </div>
      )}
    </div>
  );
}
