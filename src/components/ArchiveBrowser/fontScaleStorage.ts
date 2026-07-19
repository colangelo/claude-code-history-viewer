/**
 * Persistence + application of the webapp font scale
 * (spec: openspec/specs/static-archive-webapp/spec.md, Reader controls).
 * Separate from `FontScaleControl` so the component file keeps
 * component-only exports (react-refresh rule) and the logic stays
 * unit-testable — same split as `hubConfigStorage`.
 */

const STORAGE_KEY = "cchv.archiveWeb.fontScale";
/** Integer tenths to avoid float-step artifacts (8..14 → 0.8..1.4). */
export const MIN_TENTHS = 8;
export const MAX_TENTHS = 14;
/** Webapp default 1.1 — the shared type scale is tuned for the dense desktop
 *  viewer; the webapp reads one step larger. */
export const DEFAULT_TENTHS = 11;

export function loadStoredTenths(): number {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw == null) return DEFAULT_TENTHS;
    const parsed = Number(raw);
    if (
      Number.isInteger(parsed) &&
      parsed >= MIN_TENTHS &&
      parsed <= MAX_TENTHS
    ) {
      return parsed;
    }
    return DEFAULT_TENTHS;
  } catch {
    return DEFAULT_TENTHS;
  }
}

export function storeTenths(tenths: number): void {
  try {
    localStorage.setItem(STORAGE_KEY, String(tenths));
  } catch {
    // Storage unavailable — the scale still applies for this session.
  }
}

/** Set `--app-font-scale`, the var every `text-pxN` utility multiplies by. */
export function applyScale(tenths: number): void {
  document.documentElement.style.setProperty(
    "--app-font-scale",
    (tenths / 10).toFixed(1)
  );
}

/** Apply the persisted (or default) scale before the app's first paint. */
export function initFontScale(): void {
  applyScale(loadStoredTenths());
}
