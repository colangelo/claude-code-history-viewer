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
  type HubConfig,
  type HubSession,
  type JournalEntry,
} from "../../services/hubApi";
import { JournalEntryCard } from "./JournalEntryCard";
import { dayLabel, shortDayLabel } from "@/utils/journalFormat";

/** Hub's max page size (`crates/hub/src/pagination.rs::MAX_LIMIT`). */
const PAGE_SIZE = 200;

interface JournalViewProps {
  config: HubConfig;
  /** When set, the feed jumps to this `entry_date` (from a search hit). */
  anchorDate: string | null;
  /** Bumped whenever an anchor is (re)requested, even to the same date. */
  anchorNonce: number;
  /** Open a session in the Browse view (reuses the existing message path). */
  onOpenSession: (sessionId: number, label: string) => void;
}

interface DayGroup {
  date: string;
  entries: JournalEntry[];
}

export function JournalView({
  config,
  anchorDate,
  anchorNonce,
  onOpenSession,
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

  // Jump to an anchor date requested from a journal search hit. `anchorNonce`
  // is a dep so a repeated jump to the same date still re-triggers.
  useEffect(() => {
    if (anchorDate != null) setDate(anchorDate);
  }, [anchorDate, anchorNonce]);

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
        limit: PAGE_SIZE,
        offset: 0,
      })
      .then((list) => {
        if (generationRef.current !== generation) return;
        setEntries(list);
        setHasMore(list.length === PAGE_SIZE);
        mergeProjectOptions(list);
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
  }, [config, date, projectFilter, mergeProjectOptions]);

  const handleLoadMore = useCallback(() => {
    if (isLoading) return;
    const generation = generationRef.current;
    setIsLoading(true);
    hubApi
      .journalEntries(config, {
        from: date || undefined,
        to: date || undefined,
        project: projectFilter || undefined,
        limit: PAGE_SIZE,
        offset: entries.length,
      })
      .then((list) => {
        if (generationRef.current !== generation) return;
        setEntries((prev) => [...prev, ...list]);
        setHasMore(list.length === PAGE_SIZE);
        mergeProjectOptions(list);
      })
      .catch((err) => {
        if (generationRef.current !== generation) return;
        setError(String(err));
      })
      .finally(() => {
        if (generationRef.current !== generation) return;
        setIsLoading(false);
      });
  }, [config, date, projectFilter, entries.length, isLoading, mergeProjectOptions]);

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

  // Distinct loaded dates for quick-nav pills (already newest-first).
  const quickNavDates = useMemo(() => groups.map((g) => g.date), [groups]);

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

  return (
    <div className="flex flex-col flex-1 min-h-0 gap-2 w-full max-w-4xl mx-auto">
      {/* Controls: date picker, project filter, quick-nav pills */}
      <div className="flex flex-wrap items-center gap-2 shrink-0">
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
          {projectOptions.map((path) => (
            <option key={path} value={path}>
              {path}
            </option>
          ))}
        </select>
      </div>

      {quickNavDates.length > 0 && (
        <div className="flex flex-wrap gap-1 shrink-0">
          {quickNavDates.map((d) => (
            <button
              key={d}
              type="button"
              onClick={() => setDate(d)}
              className={`rounded-full border border-border px-2.5 py-1 text-px12 hover:bg-muted ${
                date === d ? "bg-accent/10" : ""
              }`}
            >
              {pillLabel(d)}
            </button>
          ))}
        </div>
      )}

      {/* Feed */}
      <div className="flex-1 min-h-0 overflow-y-auto space-y-4 pr-1">
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
          <p
            data-testid="journal-empty"
            className="text-px14 text-muted-foreground"
          >
            {t("settings.archiveHub.journal.empty")}
          </p>
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
  );
}
