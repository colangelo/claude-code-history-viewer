/**
 * Browse's third pane: the open session's loaded message window, plus the
 * controls that extend it. The window need not start at the session's
 * beginning — a search hit opens the page CONTAINING the match (issue #20),
 * so "Load earlier" appears whenever `windowStart > 0`.
 */

import { ChevronLeft, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { ExpandKeyProvider } from "@/contexts/CaptureExpandContext";
import { MessageContentDisplay } from "@/components/messageRenderer";
import { ClaudeContentArrayRenderer } from "@/components/contentRenderer";
import { cn } from "@/lib/utils";
import { formatCount, humanizeTimestamp } from "@/utils/journalFormat";
import { ArchiveRenderContext } from "@/contexts/ArchiveRenderContext";
import {
  hubMessageToClaudeMessage,
  type HubMessage,
} from "../../services/hubApi";

/** The session Browse currently has open: `ref` addresses it on the hub,
 * `label` is what the header shows. */
export interface OpenSession {
  ref: number | string;
  label: string;
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
function ArchivedMessage({
  row,
  sessionId,
}: {
  row: HubMessage;
  sessionId: string;
}) {
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

export interface MessagesPaneProps {
  messages: HubMessage[];
  openSession: OpenSession | null;
  totalCount: number | null;
  /** Offset of `messages[0]` within the session. */
  windowStart: number;
  highlightMessageId: number | null;
  isLoadingMessages: boolean;
  isLoadingAll: boolean;
  error: string | null;
  hasMore: boolean;
  remaining: number;
  /** Decides whether the mobile back button says "sessions" or "projects". */
  hasSelectedGroup: boolean;
  /** Collapsed by the mobile drill-down (no session open at this level). */
  hidden: boolean;
  onBack: () => void;
  onLoadEarlier: () => void;
  onLoadMore: () => void;
  onLoadAll: () => void;
  onStopLoadAll: () => void;
}

export function MessagesPane({
  messages,
  openSession,
  totalCount,
  windowStart,
  highlightMessageId,
  isLoadingMessages,
  isLoadingAll,
  error,
  hasMore,
  remaining,
  hasSelectedGroup,
  hidden,
  onBack,
  onLoadEarlier,
  onLoadMore,
  onLoadAll,
  onStopLoadAll,
}: MessagesPaneProps) {
  const { t } = useTranslation();

  return (
    <div
      className={cn(
        "flex-1 min-w-0 overflow-y-auto border border-border/50 rounded-md",
        hidden && "hidden md:block"
      )}
    >
      {/* Header rides the same centered column as the messages — on
          ultrawide screens label and count otherwise sit 1400px apart. */}
      <div className="w-full max-w-4xl mx-auto flex items-center gap-2 px-2 py-1.5 min-w-0">
        {openSession && (
          <button
            type="button"
            data-testid="browse-back-from-messages"
            onClick={onBack}
            className="md:hidden flex items-center gap-0.5 shrink-0 text-px12 text-muted-foreground hover:text-foreground"
          >
            <ChevronLeft className="w-3.5 h-3.5" aria-hidden="true" />
            {hasSelectedGroup
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
      {error && <p className="px-2 py-1 text-px14 text-destructive">{error}</p>}
      {openSession && !isLoadingMessages && !error && messages.length === 0 && (
        <p className="px-2 py-1 text-px14 text-muted-foreground">
          {t("settings.archiveHub.browser.messages.empty")}
        </p>
      )}
      {openSession && windowStart > 0 && (
        <div className="px-2 pt-1 w-full max-w-4xl mx-auto">
          <button
            type="button"
            data-testid="archive-load-earlier"
            onClick={onLoadEarlier}
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
            const isConversationRole = role != null && GUTTER_ROLES.has(role);
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
      {hasMore && (
        <div className="px-2 pb-2 w-full max-w-4xl mx-auto flex gap-2">
          <button
            type="button"
            data-testid="archive-load-more"
            onClick={onLoadMore}
            disabled={isLoadingMessages}
            className="flex-1 rounded-md border border-border px-3 py-2 text-px14 hover:bg-muted disabled:opacity-50"
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
          {/* Quieter than its neighbour: paging one screen at a time is
              the ordinary move, walking the whole session is the
              occasional one. The count is on the label so a 5,000-message
              session announces the cost before the click, not after. */}
          <button
            type="button"
            data-testid="archive-load-all"
            onClick={() => (isLoadingAll ? onStopLoadAll() : onLoadAll())}
            disabled={isLoadingMessages && !isLoadingAll}
            className="shrink-0 rounded-md px-3 py-2 text-px14 text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-50"
          >
            {isLoadingAll
              ? t("settings.archiveHub.browser.messages.loadAllStop")
              : t("settings.archiveHub.browser.messages.loadAll", {
                  remaining,
                })}
          </button>
        </div>
      )}
    </div>
  );
}
