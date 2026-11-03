/**
 * Connect gate for the standalone static archive webapp
 * (spec: openspec/specs/static-archive-webapp/spec.md).
 *
 * Owns the hub connection lifecycle. Order of attempts:
 * 1. stored hub URL from a previous login (`localStorage`, never a token);
 * 2. same-origin auto-connect — a tokenless probe of the page origin, which
 *    succeeds while the browser holds a live session cookie; persists nothing;
 * 3. the login form. On the page's own hub the token is traded once for the
 *    hub's `HttpOnly` session cookie (`POST /v1/login`) and kept nowhere
 *    (openspec hub-read-needs-credential D3). Another origin cannot use that
 *    cookie, so there the token is held in memory for this tab only.
 *
 * A read that answers 401 later (session expired or revoked) brings the form
 * back in place; the hash route is kept, so logging in returns to the view.
 */

import { useCallback, useEffect, useId, useRef, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ArchiveBrowser } from "./index";
import { FontScaleControl } from "./FontScaleControl";
import { ThemeToggle } from "./ThemeToggle";
import {
  HUB_UNAUTHORIZED_EVENT,
  HubHttpError,
  hubApi,
  type HubConfig,
} from "../../services/hubApi";
import {
  clearStoredHubConfig,
  loadStoredHubConfig,
  storeHubConfig,
} from "./hubConfigStorage";

/** Display form of the hub URL — host only; the full URL goes in `title`. */
function hubHost(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/** Whether `url` is the page's own origin (where the session cookie lives). */
function isPageOrigin(url: string): boolean {
  try {
    return new URL(url).origin === window.location.origin;
  } catch {
    return false;
  }
}

export function ConnectGate() {
  const { t } = useTranslation();
  const [config, setConfig] = useState<HubConfig | null>(() => loadStoredHubConfig());
  const [probing, setProbing] = useState(() => loadStoredHubConfig() == null);
  const [url, setUrl] = useState(() => window.location.origin);
  const [token, setToken] = useState("");
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sessionEnded, setSessionEnded] = useState(false);
  const probeStarted = useRef(false);
  const urlId = useId();
  const tokenId = useId();
  const endedId = useId();
  const tokenRef = useRef<HTMLInputElement>(null);

  // Same-origin auto-connect: if the host authenticates us (e.g. Tailscale
  // serve identity headers), skip the gate entirely. Never persisted — the
  // next load re-probes, so revoking host-side access takes effect.
  useEffect(() => {
    if (config != null || probeStarted.current) {
      setProbing(false);
      return;
    }
    probeStarted.current = true;
    let cancelled = false;
    const sameOrigin: HubConfig = { url: window.location.origin, token: "" };
    hubApi
      .listProjects(sameOrigin, { limit: 1 })
      .then(() => {
        if (!cancelled) setConfig(sameOrigin);
      })
      .catch(() => {
        // Host doesn't vouch for us — fall through to the manual form.
      })
      .finally(() => {
        if (!cancelled) setProbing(false);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // A read answered 401 while connected: the session ended. Back to the form.
  useEffect(() => {
    if (config == null) return;
    const onUnauthorized = () => {
      setConfig(null);
      setSessionEnded(true);
    };
    window.addEventListener(HUB_UNAUTHORIZED_EVENT, onUnauthorized);
    return () => window.removeEventListener(HUB_UNAUTHORIZED_EVENT, onUnauthorized);
  }, [config]);

  // After a session ends, land on the token field: keyboard users start
  // there, and a password manager offers to fill it.
  useEffect(() => {
    if (sessionEnded && config == null) tokenRef.current?.focus();
  }, [sessionEnded, config]);

  const handleSubmit = useCallback(
    async (event: FormEvent) => {
      event.preventDefault();
      const target = url.trim();
      const secret = token.trim();
      if (!target) return;
      setConnecting(true);
      setError(null);
      try {
        let candidate: HubConfig;
        if (secret && isPageOrigin(target)) {
          // Trade the token for the session cookie; keep the token nowhere.
          await hubApi.login(target, secret);
          candidate = { url: target, token: "" };
        } else {
          // Another origin: bearer for this tab only. An empty token probes
          // with whatever session the browser already has.
          candidate = { url: target, token: secret };
        }
        // Authenticated probe: validates URL, reachability AND the credential.
        await hubApi.listProjects(candidate, { limit: 1 });
        if (candidate.token === "") storeHubConfig(candidate);
        setToken("");
        setSessionEnded(false);
        setConfig(candidate);
      } catch (e) {
        if (e instanceof HubHttpError && e.status === 401) {
          setError(t("archive.web.tokenRejected"));
        } else if (e instanceof HubHttpError && e.status === 429) {
          setError(t("archive.web.tooManyTries", { seconds: e.retryAfterSecs ?? 60 }));
        } else {
          setError(
            t("archive.web.connectFailed", { error: e instanceof Error ? e.message : String(e) })
          );
        }
      } finally {
        setConnecting(false);
      }
    },
    [url, token, t]
  );

  const handleDisconnect = useCallback(() => {
    if (config != null && config.token === "") {
      // Best effort: the cookie is ended server-side; a failure leaves it to expire.
      hubApi.logout(config.url).catch(() => undefined);
    }
    clearStoredHubConfig();
    setConfig(null);
    setError(null);
    setSessionEnded(false);
  }, [config]);

  if (probing && config == null) {
    return (
      <div
        className="flex h-full items-center justify-center"
        aria-busy="true"
        aria-live="polite"
      >
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" aria-hidden="true" />
        <span className="sr-only">{t("archive.web.connecting")}</span>
      </div>
    );
  }

  if (config) {
    return (
      <div className="flex flex-col h-full gap-2">
        <div className="flex items-center justify-between gap-3 shrink-0 border-b border-border pb-2">
          <div className="flex items-baseline gap-2 min-w-0">
            <h1 className="text-px16 font-semibold shrink-0">
              {t("archive.web.title")}
            </h1>
            <span
              data-testid="app-version"
              className="text-px12 text-muted-foreground shrink-0"
              title={`cchv-v${__APP_VERSION__}`}
            >
              (v{__APP_VERSION__})
            </span>
            <span
              className="text-px12 text-muted-foreground shrink-0"
              aria-hidden="true"
            >
              –
            </span>
            <span
              className="text-px12 font-mono text-muted-foreground truncate"
              title={config.url}
            >
              {hubHost(config.url)}
            </span>
          </div>
          <div className="flex items-center gap-2 shrink-0">
            <ThemeToggle />
            <FontScaleControl />
            {/* The one exit action: neutral at rest so it never nags, but it
                drops the stored hub config, so the hover states it. Same
                language as Unlink in the Locations block. */}
            <Button
              variant="outline"
              size="sm"
              onClick={handleDisconnect}
              className="text-muted-foreground transition-colors hover:border-destructive/40 hover:bg-destructive/10 hover:text-destructive"
            >
              {t("archive.web.logout")}
            </Button>
          </div>
        </div>
        <div className="flex-1 min-h-0">
          <ArchiveBrowser config={config} enableHashRoutes />
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full items-center justify-center p-4">
      <form
        onSubmit={handleSubmit}
        className="w-full max-w-sm space-y-4 rounded-lg border border-border bg-background p-6"
      >
        <div className="space-y-1">
          <div className="flex items-baseline gap-2">
            <h1 className="text-base font-semibold">{t("archive.web.title")}</h1>
            <span
              data-testid="app-version"
              className="text-px12 text-muted-foreground"
              title={`cchv-v${__APP_VERSION__}`}
            >
              (v{__APP_VERSION__})
            </span>
          </div>
          <p className="text-xs text-muted-foreground">{t("archive.web.subtitle")}</p>
          {sessionEnded && (
            <p id={endedId} role="status" className="text-xs font-medium text-foreground">
              {t("archive.web.sessionEnded")}
            </p>
          )}
        </div>
        <div className="space-y-1">
          <label htmlFor={urlId} className="text-xs font-medium">
            {t("archive.web.urlLabel")}
          </label>
          <Input
            id={urlId}
            type="url"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            autoComplete="url"
            placeholder={t("archive.web.urlPlaceholder")}
            required
          />
        </div>
        {/* A fixed account name, so a password manager files the token under
            one entry for this hub and fills it next time. Visually hidden, not
            `hidden`: password managers skip fields that are not rendered. */}
        <input
          type="text"
          name="username"
          autoComplete="username"
          value="cchv-reader"
          readOnly
          tabIndex={-1}
          aria-hidden="true"
          className="sr-only"
        />
        <div className="space-y-1">
          <label htmlFor={tokenId} className="text-xs font-medium">
            {t("archive.web.tokenLabel")}
          </label>
          <Input
            id={tokenId}
            ref={tokenRef}
            aria-describedby={sessionEnded ? endedId : undefined}
            type="password"
            name="password"
            autoComplete="current-password"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder={t("archive.web.tokenPlaceholder")}
          />
        </div>
        {error != null && (
          <p role="alert" className="text-xs text-destructive">
            {error}
          </p>
        )}
        <Button type="submit" className="w-full" disabled={connecting}>
          {connecting ? (
            <>
              <Loader2 className="mr-2 h-3 w-3 animate-spin" aria-hidden="true" />
              {t("archive.web.loggingIn")}
            </>
          ) : (
            t("archive.web.login")
          )}
        </Button>
        <p className="text-px12 text-muted-foreground">{t("archive.web.storageHint")}</p>
      </form>
    </div>
  );
}
