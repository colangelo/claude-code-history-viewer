/**
 * Platform utilities for the WebUI: OS detection, API base paths, auth and
 * external links.
 */

declare global {
  interface Window {
    __WEBUI_API_BASE__?: string;
    __WEBUI_BASE_PATH__?: string;
  }
}

/** True when the current platform is macOS. */
export const isMacOS = (): boolean =>
  typeof navigator !== "undefined" && /mac/i.test(navigator.userAgent);

/** True when the current platform is Windows. */
export const isWindows = (): boolean =>
  typeof navigator !== "undefined" && /windows/i.test(navigator.userAgent);

/** True when the action modifier key is held (Cmd on macOS, Ctrl elsewhere). */
export const isActionModifier = (e: { metaKey: boolean; ctrlKey: boolean }): boolean =>
  isMacOS() ? e.metaKey : e.ctrlKey;

const normalizeWebUIBasePath = (value?: string): string => {
  if (!value) return "";

  const trimmed = value.trim();
  if (trimmed.length === 0 || trimmed === "/") return "";

  const withLeadingSlash = trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
  return withLeadingSlash.replace(/\/+$/, "");
};

/** Path prefix where the WebUI is mounted, or an empty string for root. */
export const getWebUIBasePath = (): string =>
  typeof window !== "undefined"
    ? normalizeWebUIBasePath(window.__WEBUI_BASE_PATH__)
    : "";

/**
 * Base URL for WebUI API calls.
 *
 * Defaults to the current origin (same-origin requests when the SPA is
 * served by the Axum server). If the server injected `window.__WEBUI_BASE_PATH__`,
 * API requests are sent below that path prefix. Can be overridden via
 * `window.__WEBUI_API_BASE__` for development scenarios (e.g. Vite dev server
 * proxying to a remote host).
 */
export const getApiBase = (): string => {
  if (typeof window !== "undefined" && window.__WEBUI_API_BASE__) {
    return window.__WEBUI_API_BASE__;
  }
  return typeof window !== "undefined"
    ? `${window.location.origin}${getWebUIBasePath()}`
    : "";
};

/** URL for public WebUI assets, respecting reverse-proxy path prefixes. */
export const getAssetPath = (path: string): string => {
  const normalizedPath = path.replace(/^\/+/, "");
  const basePath = getWebUIBasePath();
  return basePath ? `${basePath}/${normalizedPath}` : `/${normalizedPath}`;
};

// ---------------------------------------------------------------------------
// Auth token helpers
// ---------------------------------------------------------------------------

const AUTH_TOKEN_KEY = "webui-auth-token";
const CSRF_COOKIE_NAME = "cchv_csrf";
export const EXTERNAL_OPEN_HELPER_ATTRIBUTE = "data-external-open-helper";

export interface WebUILoginCredentials {
  token?: string;
  username?: string;
  password?: string;
}

export interface WebUILoginResult {
  ok: boolean;
  status: number;
}

/**
 * Initialise the auth token from the URL query string.
 *
 * Call once at app startup (before React renders).  If the URL contains
 * `?token=<value>`, the token is persisted to `localStorage` and the
 * query parameter is stripped from the address bar so it isn't leaked via
 * Referer headers or browser history.
 */
export function initAuthToken(): void {
  const url = new URL(window.location.href);
  const token = url.searchParams.get("token");
  if (token) {
    setAuthToken(token);
    url.searchParams.delete("token");
    window.history.replaceState(window.history.state, "", url.toString());
  }
}

/** True when the app should render the WebUI login screen. */
export function hasAuthErrorQuery(): boolean {
  const url = new URL(window.location.href);
  return url.searchParams.get("auth_error") === "1";
}

/** Remove the explicit WebUI auth-error marker from the current URL. */
export function clearAuthErrorQuery(): void {
  const url = new URL(window.location.href);
  url.searchParams.delete("auth_error");
  window.history.replaceState(window.history.state, "", url.toString());
}

/** Read the saved auth token (returns `null` when unavailable). */
export function getAuthToken(): string | null {
  try {
    return localStorage.getItem(AUTH_TOKEN_KEY);
  } catch {
    return null;
  }
}

/** Persist an auth token to `localStorage`. */
export function setAuthToken(token: string): void {
  try {
    const normalized = token.trim();
    if (normalized.length === 0) {
      localStorage.removeItem(AUTH_TOKEN_KEY);
      return;
    }
    localStorage.setItem(AUTH_TOKEN_KEY, normalized);
  } catch {
    // localStorage unavailable (e.g. private browsing quota exceeded)
  }
}

export async function loginWebUI(
  credentials: WebUILoginCredentials,
): Promise<WebUILoginResult> {
  const body: WebUILoginCredentials = {};
  if (credentials.token != null) {
    body.token = credentials.token.trim();
  }
  if (credentials.username != null) {
    body.username = credentials.username.trim();
  }
  if (credentials.password != null) {
    body.password = credentials.password;
  }

  try {
    const response = await fetch(`${getApiBase()}/api/auth/login`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      credentials: "same-origin",
      body: JSON.stringify(body),
    });

    if (response.ok) {
      clearAuthToken();
    }

    return { ok: response.ok, status: response.status };
  } catch {
    return { ok: false, status: 0 };
  }
}

/**
 * Exchange the temporary localStorage token for an HttpOnly auth cookie.
 *
 * This keeps the existing URL-token/Bearer flow backward-compatible while
 * moving the browser session to a cookie that JavaScript cannot read. On
 * failure, the localStorage token is kept so existing Bearer auth still works.
 */
export async function syncAuthCookieFromStoredToken(): Promise<boolean> {
  const token = getAuthToken();
  if (!token) return false;

  const response = await loginWebUI({ token });
  return response.ok;
}

/** Ask the WebUI server to clear its HttpOnly auth cookie. */
export async function clearAuthCookie(): Promise<void> {
  try {
    await fetch(`${getApiBase()}/api/auth/logout`, {
      method: "POST",
      credentials: "same-origin",
    });
  } catch {
    // Best-effort cleanup only.
  }
}

export function getCookieValue(name: string): string | null {
  if (typeof document === "undefined") return null;

  const prefix = `${name}=`;
  const cookie = document.cookie
    .split(";")
    .map((part) => part.trim())
    .find((part) => part.startsWith(prefix));

  if (!cookie) return null;

  try {
    return decodeURIComponent(cookie.slice(prefix.length));
  } catch {
    return cookie.slice(prefix.length);
  }
}

export function getCsrfToken(): string | null {
  return getCookieValue(CSRF_COOKIE_NAME);
}

/**
 * Open a URL in the system default browser.
 *
 * Uses a hidden `noopener` anchor click, which browsers allow from a user
 * gesture.
 */
export async function openExternalUrl(url: string): Promise<void> {
  const normalized = url.trim();
  if (!/^https?:\/\//i.test(normalized) && !/^mailto:/i.test(normalized)) {
    throw new Error(`Unsupported URL scheme: ${normalized}`);
  }

  const root = document.body ?? document.documentElement;
  if (!root) {
    throw new Error("Document root unavailable");
  }

  const link = document.createElement("a");
  link.href = normalized;
  link.target = "_blank";
  link.rel = "noopener noreferrer";
  link.style.display = "none";
  link.setAttribute(EXTERNAL_OPEN_HELPER_ATTRIBUTE, "true");

  root.appendChild(link);
  try {
    link.click();
  } finally {
    root.removeChild(link);
  }
}

/** Remove persisted auth token from `localStorage`. */
export function clearAuthToken(): void {
  try {
    localStorage.removeItem(AUTH_TOKEN_KEY);
  } catch {
    // localStorage unavailable
  }
}
