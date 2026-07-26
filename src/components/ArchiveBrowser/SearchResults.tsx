/**
 * Search results overlay: the journal-hit section, the message-hit list, and
 * the loading/error/empty states between them. Renders as a fragment so the
 * rows stay direct flex children of the browser's column — each carries its
 * own `shrink-0`, which only works if nothing wraps them.
 *
 * Activation is delegated: the parent dismisses these results before
 * navigating, so the user lands on what they picked rather than under the
 * overlay.
 */

import { AlertTriangle, Loader2, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { humanizeTimestamp } from "@/utils/journalFormat";
import { renderSnippet } from "@/utils/searchSnippet";
import type { HubSearchHit, JournalSearchHit } from "../../services/hubApi";

export interface SearchResultsProps {
  isSearching: boolean;
  error: string | null;
  /** `null` means "no search has run" — distinct from an empty result set. */
  hits: HubSearchHit[] | null;
  journalHits: JournalSearchHit[];
  journalDegraded: boolean;
  onClear: () => void;
  onActivateHit: (hit: HubSearchHit) => void;
  onActivateJournalHit: (hit: JournalSearchHit) => void;
}

export function SearchResults({
  isSearching,
  error,
  hits,
  journalHits,
  journalDegraded,
  onClear,
  onActivateHit,
  onActivateJournalHit,
}: SearchResultsProps) {
  const { t } = useTranslation();

  return (
    <>
      {isSearching && (
        <p className="text-px13 text-muted-foreground shrink-0 flex items-center gap-1.5">
          <Loader2 className="w-3.5 h-3.5 animate-spin" aria-hidden="true" />
          {t("settings.archiveHub.browser.search.loading")}
        </p>
      )}
      {error && <p className="text-px13 text-destructive shrink-0">{error}</p>}
      {!isSearching && (hits != null || journalHits.length > 0) && (
        <div className="flex items-center justify-between shrink-0">
          <p
            className="text-px12 text-muted-foreground"
            data-testid="search-result-count"
          >
            {t("settings.archiveHub.browser.search.count", {
              count: (hits?.length ?? 0) + journalHits.length,
            })}
          </p>
          <button
            type="button"
            data-testid="search-clear"
            onClick={onClear}
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
          {/* Its own row, not a tail on the eyebrow: inline, the sentence
              orphan-wraps under the label on narrow viewports. `warning` (amber)
              rather than `muted-foreground` because the neutral gray was
              pixel-identical to each hit's date/path line — a status styled like
              a timestamp reads as a timestamp. Amber, not `destructive`: search
              still returned results, it just ranked them by keyword. */}
          {journalDegraded && (
            <p
              data-testid="journal-search-degraded"
              className="flex items-start gap-1.5 px-1 text-px12 text-warning"
            >
              <AlertTriangle
                className="w-3.5 h-3.5 shrink-0 mt-px"
                aria-hidden="true"
              />
              <span>{t("settings.archiveHub.journal.searchDegraded")}</span>
            </p>
          )}
          {/* Same cap as the message-hits list below: 100 journal hits once
              rendered as a ~3000px wall burying the active view. */}
          <ul className="space-y-1 max-h-72 overflow-y-auto">
            {journalHits.map((hit, index) => (
              <li key={`${hit.entry_date}-${hit.project_path}-${index}`}>
                <button
                  type="button"
                  data-testid="journal-search-hit"
                  onClick={() => onActivateJournalHit(hit)}
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
      {hits && hits.length === 0 && journalHits.length === 0 && !isSearching && (
        <p className="text-px13 text-muted-foreground shrink-0">
          {t("settings.archiveHub.browser.search.empty")}
        </p>
      )}
      {hits && hits.length > 0 && (
        <ul className="shrink-0 space-y-1 max-h-72 overflow-y-auto border border-border/50 rounded-md p-1">
          {hits.map((hit, index) => (
            <li key={`${hit.session_id}-${index}`}>
              <button
                type="button"
                onClick={() => onActivateHit(hit)}
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
    </>
  );
}
