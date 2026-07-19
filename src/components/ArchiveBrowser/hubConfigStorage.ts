/**
 * localStorage persistence for the static archive webapp's hub connection
 * (spec: openspec/specs/static-archive-webapp/spec.md). Separate from the
 * ConnectGate component so the file keeps component-only exports
 * (react-refresh rule) and the logic stays unit-testable.
 */

import type { HubConfig } from "../../services/hubApi";

const STORAGE_KEY = "cchv.archiveWeb.hubConfig";

/** Versioned persisted shape — bump `v` if the layout ever changes. */
interface StoredHubConfig {
  v: 1;
  url: string;
  token: string;
}

export function loadStoredHubConfig(): HubConfig | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw == null) return null;
    const parsed = JSON.parse(raw) as Partial<StoredHubConfig>;
    if (
      parsed.v === 1 &&
      typeof parsed.url === "string" &&
      parsed.url.length > 0 &&
      // Empty token is valid since #21: identity-authed hubs need none.
      typeof parsed.token === "string"
    ) {
      return { url: parsed.url, token: parsed.token };
    }
    return null;
  } catch {
    return null;
  }
}

export function storeHubConfig(config: HubConfig): void {
  try {
    const stored: StoredHubConfig = {
      v: 1,
      url: config.url,
      token: config.token,
    };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(stored));
  } catch {
    // Storage unavailable (private mode, quota) — the session still works,
    // the user just reconnects next visit.
  }
}

export function clearStoredHubConfig(): void {
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    // Nothing to clean up if storage is unavailable.
  }
}
