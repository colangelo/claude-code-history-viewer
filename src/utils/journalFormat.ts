/**
 * Small `Intl`-based formatting helpers for the archive journal UI (no date
 * library). Day labels are derived from the `entry_date` calendar string, NOT
 * from wall-clock arithmetic on the entry's `generated_at` timestamp — journal
 * entries are logical days (a UTC fold), so their date string is authoritative.
 */

/** A humanized label for a journal day header. */
export interface DayLabel {
  /**
   * i18n key suffix for a relative label (`today` / `yesterday`), or `null`
   * when the day is far enough back to warrant an absolute weekday+date label.
   */
  relativeKey: "today" | "yesterday" | null;
  /** Absolute weekday+date label (always populated as the non-relative form). */
  absolute: string;
}

/** Parse a `YYYY-MM-DD` calendar string into a local-midnight Date, or null. */
function parseCalendarDate(dateStr: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(dateStr.trim());
  if (!m) return null;
  const year = Number(m[1]);
  const month = Number(m[2]);
  const day = Number(m[3]);
  const d = new Date(year, month - 1, day);
  // Reject overflow (e.g. 2026-13-40 rolling over into another month).
  if (
    d.getFullYear() !== year ||
    d.getMonth() !== month - 1 ||
    d.getDate() !== day
  ) {
    return null;
  }
  return d;
}

/** Whole calendar days between two local-midnight dates (a − b). */
function calendarDaysBetween(a: Date, b: Date): number {
  return Math.round((a.getTime() - b.getTime()) / 86_400_000);
}

/**
 * Humanize a journal `entry_date` (`YYYY-MM-DD`). The most recent closed days
 * get relative labels (Today/Yesterday); older days get an absolute
 * weekday+date. `today` is injectable for testing.
 */
export function dayLabel(entryDate: string, today: Date = new Date()): DayLabel {
  const parsed = parseCalendarDate(entryDate);
  if (!parsed) {
    // Unparseable input: fall back to the raw string as its own absolute label.
    return { relativeKey: null, absolute: entryDate };
  }
  const todayMidnight = new Date(
    today.getFullYear(),
    today.getMonth(),
    today.getDate()
  );
  const diff = calendarDaysBetween(todayMidnight, parsed);
  const absolute = new Intl.DateTimeFormat(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
    year: "numeric",
  }).format(parsed);

  if (diff === 0) return { relativeKey: "today", absolute };
  if (diff === 1) return { relativeKey: "yesterday", absolute };
  return { relativeKey: null, absolute };
}

/** A short label for a quick-nav date pill (relative or month/day). */
export function shortDayLabel(entryDate: string, today: Date = new Date()): DayLabel {
  const label = dayLabel(entryDate, today);
  if (label.relativeKey) return label;
  const parsed = parseCalendarDate(entryDate);
  if (!parsed) return label;
  return {
    relativeKey: null,
    absolute: new Intl.DateTimeFormat(undefined, {
      month: "short",
      day: "numeric",
    }).format(parsed),
  };
}

/**
 * Humanize an ISO timestamp to a locale date (no raw ISO string). Invalid
 * input is returned unchanged so callers never render "Invalid Date".
 */
export function humanizeTimestamp(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  }).format(d);
}

/** Locale-format an integer count (thousands grouping). */
export function formatCount(n: number): string {
  return new Intl.NumberFormat().format(n);
}
