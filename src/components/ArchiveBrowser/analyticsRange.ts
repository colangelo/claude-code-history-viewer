/**
 * Relative date-window selection for the Analytics view (analytics-ux-costs).
 * Kept out of the component so the "N days → from=YYYY-MM-DD" mapping and the
 * custom-input parsing are unit-testable without mounting the view.
 */

/** Preset windows offered in the range select, in days. */
export const RANGE_PRESET_DAYS = [7, 14, 30, 60, 90, 180, 365] as const;

/** Values the range `<select>` can hold beyond plain day counts. */
export const RANGE_ALL = "all";
export const RANGE_CUSTOM = "custom";

/** `days` resolved against today in the viewer's timezone; `null` = all time. */
export function isoDaysAgo(days: number, now: Date = new Date()): string {
  const d = new Date(now);
  d.setDate(d.getDate() - days);
  return d.toISOString().slice(0, 10);
}

/**
 * Parse the custom "last N days" input. Returns the day count, or `null` for
 * anything that is not a positive integer — the caller keeps the previous
 * window rather than fetching something surprising.
 */
export function parseCustomDays(raw: string): number | null {
  const trimmed = raw.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const days = Number(trimmed);
  return days > 0 ? days : null;
}
