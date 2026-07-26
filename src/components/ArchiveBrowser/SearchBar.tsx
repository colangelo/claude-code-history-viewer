/**
 * The archive's global search input. Rendered above both Journal and Browse
 * (Analytics keeps its own toolbar), so it is a sibling of the views rather
 * than part of either.
 *
 * `inputRef` is owned by the parent because `/`-to-focus (issue #21) has to
 * reach the input from a window-level key handler, and from Analytics — where
 * this component isn't mounted at all — by first switching views.
 */

import type { FormEvent, RefObject } from "react";
import { useTranslation } from "react-i18next";

export interface SearchBarProps {
  inputRef: RefObject<HTMLInputElement | null>;
  query: string;
  onQueryChange: (query: string) => void;
  onSubmit: (e: FormEvent) => void;
}

export function SearchBar({
  inputRef,
  query,
  onQueryChange,
  onSubmit,
}: SearchBarProps) {
  const { t } = useTranslation();

  return (
    <form onSubmit={onSubmit} className="flex items-center gap-2 shrink-0">
      <input
        ref={inputRef}
        data-testid="archive-search-input"
        value={query}
        onChange={(e) => onQueryChange(e.target.value)}
        placeholder={t("settings.archiveHub.browser.searchPlaceholder")}
        aria-label={t("settings.archiveHub.browser.searchPlaceholder")}
        className="flex-1 h-9 rounded-md border border-border bg-background px-2.5 text-px14"
      />
      {/* The one primary verb in the archive: solid accent, so it is never
          mistaken for the neutral utilities sharing the toolbar. */}
      <button
        type="submit"
        className="h-9 shrink-0 rounded-md bg-accent px-3 text-px14 font-medium text-accent-foreground transition-colors hover:bg-accent/90"
      >
        {t("settings.archiveHub.browser.searchButton")}
      </button>
    </form>
  );
}
