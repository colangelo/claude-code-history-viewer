/**
 * Persistence for the worktree-visibility toggle (spec:
 * openspec/specs/archive-journal-ui — "Worktree visibility toggle").
 * Mirrors `fontScaleStorage`: a `cchv.archiveWeb.*` localStorage key with a
 * default fallback and try/catch guards (storage can be unavailable).
 */

const STORAGE_KEY = "cchv.archiveWeb.showWorktrees";

export const DEFAULT_SHOW_WORKTREES = true;

export function loadShowWorktrees(): boolean {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === null) return DEFAULT_SHOW_WORKTREES;
    return raw === "true";
  } catch {
    return DEFAULT_SHOW_WORKTREES;
  }
}

export function storeShowWorktrees(value: boolean): void {
  try {
    localStorage.setItem(STORAGE_KEY, String(value));
  } catch {
    // Non-fatal: the toggle still works for the session.
  }
}
