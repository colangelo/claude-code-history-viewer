/**
 * Journal view: a reverse-chronological day feed of distilled journal entries
 * from the hub (`hubApi.journalEntries`). Entries arrive newest-first and are
 * grouped client-side by `entry_date` under humanized day headers. Provides
 * quick-nav date pills, a date-picker jump, a project filter, and load-more
 * pagination, plus loading / error / empty states.
 *
 * Stale-response protection mirrors `ArchiveBrowser`: a monotonic generation
 * counter guards every fetch, and any filter/date/anchor change resets
 * pagination to offset 0.
 */

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";
import {
  hubApi,
  identityProjectFilter,
  type HubConfig,
  type HubSession,
  type JournalEntry,
} from "../../services/hubApi";
import { JournalEntryCard } from "./JournalEntryCard";
import type { ProjectGroup } from "./projectGrouping";
import { cn } from "@/lib/utils";
import { dayLabel, shortDayLabel } from "@/utils/journalFormat";
import type { SessionOpenContext } from "./index";

/** Hub's max page size (`crates/hub/src/pagination.rs::MAX_LIMIT`). */
const PAGE_SIZE = 200;

interface JournalViewProps {
  config: HubConfig;
  /** When set, the feed jumps to this `entry_date` (from a search hit). */
  anchorDate: string | null;
  /** Bumped whenever an anchor is (re)requested, even to the same date. */
  anchorNonce: number;
  /** Identity grouping computed by the parent (drives the project filter). */
  projectGroups: ProjectGroup[];
  /** Worktree visibility (identity-scoped filters pass it to the hub). */
  showWorktrees: boolean;
  /** Open a session in the Browse view (reuses the existing message path). */
  onOpenSession: (
    sessionId: number,
    label: string,
    context?: SessionOpenContext
  ) => void;
  /** Notifies the parent of the current date filter (hash routing). */
  onDateChange?: (date: string) => void;
}

interface DayGroup {
  date: string;
  entries: JournalEntry[];
}

export function JournalView({
  config,
  anchorDate,
  anchorNonce,
  projectGroups,
  showWorktrees,
  onOpenSession,
  onDateChange,
}: JournalViewProps) {
  const { t } = useTranslation();

  const [entries, setEntries] = useState<JournalEntry[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hasMore, setHasMore] = useState(false);

  const [date, setDate] = useState<string>(anchorDate ?? "");
  const [projectFilter, setProjectFilter] = useState<string>("");
  // Union of every project path seen, so the filter options never shrink when
  // the feed is narrowed to a single project.
  const [projectOptions, setProjectOptions] = useState<string[]>([]);
  // Union of every entry date seen: the quick-nav pills must survive a date
  // jump (a filtered fetch returns only one day) so the user can hop back.
  const [knownDates, setKnownDates] = useState<string[]>([]);

  // Mirror the date filter to the parent (hash routing).
  useEffect(() => {
    onDateChange?.(date);
  }, [date, onDateChange]);

  const generationRef = useRef(0);

  // Lazy per-project session resolution: cache + in-flight dedupe so two cards
  // of the same project trigger exactly one `GET /v1/sessions`.
  const sessionCacheRef = useRef(new Map<string, HubSession[]>());
  const inflightRef = useRef(new Map<string, Promise<HubSession[]>>());

  const resolveSessions = useCallback(
    (projectPath: string): Promise<HubSession[]> => {
      const cached = sessionCacheRef.current.get(projectPath);
      if (cached) return Promise.resolve(cached);
      const inflight = inflightRef.current.get(projectPath);
      if (inflight) return inflight;
      const promise = hubApi
        .listSessions(config, { project: projectPath })
        .then((list) => {
          sessionCacheRef.current.set(projectPath, list);
          inflightRef.current.delete(projectPath);
          return list;
        })
        .catch((err) => {
          inflightRef.current.delete(projectPath);
          throw err;
        });
      inflightRef.current.set(projectPath, promise);
      return promise;
    },
    [config]
  );

  const mergeProjectOptions = useCallback((list: JournalEntry[]) => {
    setProjectOptions((prev) => {
      const seen = new Set(prev);
      let changed = false;
      for (const e of list) {
        if (!seen.has(e.project_path)) {
          seen.add(e.project_path);
          changed = true;
        }
      }
      return changed ? Array.from(seen) : prev;
    });
  }, []);

  const mergeKnownDates = useCallback((list: JournalEntry[]) => {
    setKnownDates((prev) => {
      const seen = new Set(prev);
      let changed = false;
      for (const e of list) {
        if (!seen.has(e.entry_date)) {
          seen.add(e.entry_date);
          changed = true;
        }
      }
      if (!changed) return prev;
      // Newest first, capped — the pills are quick nav, not a calendar.
      return Array.from(seen).sort().reverse().slice(0, 21);
    });
  }, []);

  // Jump to an anchor date requested from a journal search hit. `anchorNonce`
  // is a dep so a repeated jump to the same date still re-triggers.
  useEffect(() => {
    if (anchorDate != null) setDate(anchorDate);
  }, [anchorDate, anchorNonce]);

  // In identity scope the worktree toggle affects which member paths count.
  const includeWorktrees =
    projectFilter.startsWith("identity:") && !showWorktrees ? false : undefined;

  // Primary feed fetch: refetches (offset 0) on any config/date/filter change.
  useEffect(() => {
    const generation = ++generationRef.current;
    setIsLoading(true);
    setError(null);
    hubApi
      .journalEntries(config, {
        from: date || undefined,
        to: date || undefined,
        project: projectFilter || undefined,
        include_worktrees: includeWorktrees,
        limit: PAGE_SIZE,
        offset: 0,
      })
      .then((list) => {
        if (generationRef.current !== generation) return;
        setEntries(list);
        setHasMore(list.length === PAGE_SIZE);
        mergeProjectOptions(list);
        mergeKnownDates(list);
      })
      .catch((err) => {
        if (generationRef.current !== generation) return;
        setEntries([]);
        setHasMore(false);
        setError(String(err));
      })
      .finally(() => {
        if (generationRef.current !== generation) return;
        setIsLoading(false);
      });
  }, [config, date, projectFilter, includeWorktrees, mergeProjectOptions, mergeKnownDates]);

  const handleLoadMore = useCallback(() => {
    if (isLoading) return;
    const generation = generationRef.current;
    setIsLoading(true);
    hubApi
      .journalEntries(config, {
        from: date || undefined,
        to: date || undefined,
        project: projectFilter || undefined,
        include_worktrees: includeWorktrees,
        limit: PAGE_SIZE,
        offset: entries.length,
      })
      .then((list) => {
        if (generationRef.current !== generation) return;
        setEntries((prev) => [...prev, ...list]);
        setHasMore(list.length === PAGE_SIZE);
        mergeProjectOptions(list);
        mergeKnownDates(list);
      })
      .catch((err) => {
        if (generationRef.current !== generation) return;
        setError(String(err));
      })
      .finally(() => {
        if (generationRef.current !== generation) return;
        setIsLoading(false);
      });
  }, [config, date, projectFilter, includeWorktrees, entries.length, isLoading, mergeProjectOptions, mergeKnownDates]);

  const groups = useMemo<DayGroup[]>(() => {
    const map = new Map<string, JournalEntry[]>();
    for (const entry of entries) {
      const bucket = map.get(entry.entry_date);
      if (bucket) bucket.push(entry);
      else map.set(entry.entry_date, [entry]);
    }
    return Array.from(map, ([groupDate, groupEntries]) => ({
      date: groupDate,
      entries: groupEntries,
    }));
  }, [entries]);

  // Quick-nav pills come from the accumulated date union (`knownDates`), not
  // the currently-loaded groups — a date jump must not collapse them.
  const quickNavDates = knownDates;

  // Filter options: identity groups (one option per repo identity — a moved
  // repo appears once, filtering via `identity:<key>` server-side expansion),
  // plus plain path options for entry paths no group covers. Labels keep the
  // basename + collision-disambiguator rule; sorted by label.
  const filterOptions = useMemo(() => {
    const basename = (p: string) =>
      p.split(/[\\/]/).filter(Boolean).pop() ?? p;
    const seenPaths = new Set(projectOptions);
    const covered = new Set<string>();
    const options: { value: string; label: string; title: string }[] = [];
    for (const group of projectGroups) {
      if (!group.paths.some((p) => seenPaths.has(p))) continue;
      for (const p of group.paths) covered.add(p);
      options.push({
        value: group.identityKey
          ? identityProjectFilter(group.identityKey)
          : group.paths[0] ?? "",
        label: group.disambiguator
          ? `${group.displayName} — ${group.disambiguator}`
          : group.displayName,
        title: group.paths.join("\n"),
      });
    }
    // Fallback for entry paths outside the (paginated) projects listing.
    const uncovered = projectOptions.filter((p) => !covered.has(p));
    const counts = new Map<string, number>();
    for (const p of uncovered) {
      const b = basename(p);
      counts.set(b, (counts.get(b) ?? 0) + 1);
    }
    for (const p of uncovered) {
      const parts = p.split(/[\\/]/).filter(Boolean);
      const b = parts.pop() ?? p;
      const parent = parts.pop();
      options.push({
        value: p,
        label: (counts.get(b) ?? 0) > 1 && parent ? `${b} — ${parent}` : b,
        title: p,
      });
    }
    return options.sort((a, b) => a.label.localeCompare(b.label));
  }, [projectOptions, projectGroups]);

  const dayHeader = (dateStr: string): string => {
    const label = dayLabel(dateStr);
    return label.relativeKey
      ? t(`settings.archiveHub.journal.relative.${label.relativeKey}`)
      : label.absolute;
  };

  const pillLabel = (dateStr: string): string => {
    const label = shortDayLabel(dateStr);
    return label.relativeKey
      ? t(`settings.archiveHub.journal.relative.${label.relativeKey}`)
      : label.absolute;
  };

  // The row is capped + centered so on ultrawide screens the rail sits NEXT
  // to the reading column instead of marooned at the viewport edge.
  return (
    <div className="flex flex-1 min-h-0 gap-6 w-full max-w-[80rem] mx-auto">
      {/* Wide screens: the quick-nav dates become a left timeline rail beside
          the reading column — the flank space works instead of sitting empty.
          Below xl the horizontal pills row (further down) takes over. */}
      {quickNavDates.length > 0 && (
        <nav
          aria-label={t("settings.archiveHub.journal.dateNavTitle")}
          className="hidden xl:block w-52 shrink-0 overflow-y-auto py-1"
        >
          <p className="px-2 text-px12 font-medium text-muted-foreground uppercase tracking-wide">
            {t("settings.archiveHub.journal.dateNavTitle")}
          </p>
          <ul className="mt-1 space-y-0.5">
            {quickNavDates.map((d) => (
              <li key={d}>
                <button
                  type="button"
                  data-testid="journal-date-rail-item"
                  onClick={() => setDate(d)}
                  className={cn(
                    "w-full text-left rounded px-2 py-1 text-px13 hover:bg-muted",
                    date === d && "bg-accent/15 dark:bg-accent/25 text-accent font-medium"
                  )}
                >
                  {dayHeader(d)}
                </button>
              </li>
            ))}
          </ul>
        </nav>
      )}

      <div className="flex flex-col flex-1 min-h-0 gap-2">
      {/* Controls: date picker, project filter, quick-nav pills */}
      <div className="w-full max-w-4xl mx-auto flex flex-wrap items-center gap-2 shrink-0">
        <input
          type="date"
          data-testid="journal-date-picker"
          value={date}
          onChange={(e) => setDate(e.target.value)}
          aria-label={t("settings.archiveHub.journal.datePickerLabel")}
          className="h-9 rounded-md border border-border bg-background px-2 text-px14 dark:[color-scheme:dark]"
        />
        {date && (
          <button
            type="button"
            onClick={() => setDate("")}
            className="h-9 rounded-md border border-border px-2.5 text-px13 hover:bg-muted"
          >
            {t("settings.archiveHub.journal.clearDate")}
          </button>
        )}
        <select
          data-testid="journal-project-filter"
          value={projectFilter}
          onChange={(e) => setProjectFilter(e.target.value)}
          aria-label={t("settings.archiveHub.journal.projectFilterLabel")}
          className="h-9 max-w-64 rounded-md border border-border bg-background px-2 text-px14"
        >
          <option value="">
            {t("settings.archiveHub.journal.filterAll")}
          </option>
          {filterOptions.map((option) => (
            <option key={option.value} value={option.value} title={option.title}>
              {option.label}
            </option>
          ))}
        </select>
      </div>

      {quickNavDates.length > 0 && (
        <div className="w-full max-w-4xl mx-auto flex flex-wrap gap-1 shrink-0 xl:hidden">
          {quickNavDates.map((d) => (
            <button
              key={d}
              type="button"
              onClick={() => setDate(d)}
              className={`rounded-full border px-2.5 py-1 text-px12 hover:bg-muted ${
                date === d
                  ? "border-accent/60 bg-accent/15 text-accent dark:bg-accent/25"
                  : "border-border"
              }`}
            >
              {pillLabel(d)}
            </button>
          ))}
        </div>
      )}

      {/* Feed: the SCROLL CONTAINER spans full width (scrollbar at the
          viewport edge, not floating mid-screen); the content centers inside
          it at reading measure. */}
      <div className="flex-1 min-h-0 overflow-y-auto">
        <div className="w-full max-w-4xl mx-auto space-y-4 pb-2">
        {isLoading && entries.length === 0 && (
          <p className="text-px14 text-muted-foreground flex items-center gap-1.5">
            <Loader2 className="w-3.5 h-3.5 animate-spin" aria-hidden="true" />
            {t("settings.archiveHub.journal.loading")}
          </p>
        )}

        {error && (
          <p data-testid="journal-error" className="text-px14 text-destructive">
            {t("settings.archiveHub.journal.error")}
          </p>
        )}

        {!isLoading && !error && entries.length === 0 && (
          <div data-testid="journal-empty" className="space-y-2">
            <p className="text-px14 text-muted-foreground">
              {t("settings.archiveHub.journal.empty")}
            </p>
            {date && (
              <button
                type="button"
                data-testid="journal-show-latest"
                onClick={() => setDate("")}
                className="rounded-md border border-border px-3 py-1.5 text-px13 hover:bg-muted"
              >
                {t("settings.archiveHub.journal.showLatest")}
              </button>
            )}
          </div>
        )}

        {groups.map((group) => (
          <section key={group.date} className="space-y-2">
            <h3
              data-testid="journal-day-header"
              className="text-px15 font-semibold text-foreground sticky top-0 bg-background/95 py-1"
            >
              {dayHeader(group.date)}
            </h3>
            <div className="space-y-2">
              {group.entries.map((entry, i) => (
                <JournalEntryCard
                  key={`${entry.entry_date}-${entry.project_path}-${i}`}
                  entry={entry}
                  resolveSessions={resolveSessions}
                  onOpenSession={onOpenSession}
                />
              ))}
            </div>
          </section>
        ))}

        {hasMore && (
          <button
            type="button"
            data-testid="journal-load-more"
            onClick={handleLoadMore}
            disabled={isLoading}
            className="w-full rounded-md border border-border px-3 py-2 text-px14 hover:bg-muted disabled:opacity-50"
          >
            {isLoading ? (
              <>
                <Loader2
                  className="w-3.5 h-3.5 mx-auto animate-spin"
                  aria-hidden="true"
                />
                <span className="sr-only">{t("common.loading")}</span>
              </>
            ) : (
              t("settings.archiveHub.journal.loadMore")
            )}
          </button>
        )}
        </div>
      </div>
      </div>
    </div>
  );
}
