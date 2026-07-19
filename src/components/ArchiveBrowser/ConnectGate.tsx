/**
 * Connect gate for the standalone static archive webapp
 * (spec: openspec/specs/static-archive-webapp/spec.md).
 *
 * Owns the hub connection lifecycle. Order of attempts:
 * 1. stored config from a previous manual connect (`localStorage`);
 * 2. same-origin auto-connect — a tokenless probe of the page origin, which
 *    succeeds when the host authenticates the request itself (hub behind
 *    Tailscale serve with `trust_tailscale_identity`); persists nothing;
 * 3. the manual URL+token form, validated with an authenticated probe
 *    before persisting (healthz alone would not validate the token).
 */

import { useCallback, useEffect, useId, useRef, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ArchiveBrowser } from "./index";
import { FontScaleControl } from "./FontScaleControl";
import { hubApi, type HubConfig } from "../../services/hubApi";
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

export function ConnectGate() {
  const { t } = useTranslation();
  const [config, setConfig] = useState<HubConfig | null>(() => loadStoredHubConfig());
  const [probing, setProbing] = useState(() => loadStoredHubConfig() == null);
  const [url, setUrl] = useState("");
  const [token, setToken] = useState("");
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const probeStarted = useRef(false);
  const urlId = useId();
  const tokenId = useId();

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

  const handleSubmit = useCallback(
    async (event: FormEvent) => {
      event.preventDefault();
      const candidate: HubConfig = { url: url.trim(), token: token.trim() };
      if (!candidate.url || !candidate.token) return;
      setConnecting(true);
      setError(null);
      try {
        // Authenticated probe: validates URL, reachability AND token.
        await hubApi.listProjects(candidate, { limit: 1 });
        storeHubConfig(candidate);
        setConfig(candidate);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setConnecting(false);
      }
    },
    [url, token]
  );

  const handleDisconnect = useCallback(() => {
    clearStoredHubConfig();
    setConfig(null);
    setError(null);
  }, []);

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
              className="text-px12 font-mono text-muted-foreground truncate"
              title={config.url}
            >
              {hubHost(config.url)}
            </span>
          </div>
          <div className="flex items-center gap-2 shrink-0">
            <FontScaleControl />
            <Button variant="outline" size="sm" onClick={handleDisconnect}>
              {t("archive.web.disconnect")}
            </Button>
          </div>
        </div>
        <div className="flex-1 min-h-0">
          <ArchiveBrowser config={config} />
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
          <h1 className="text-base font-semibold">{t("archive.web.title")}</h1>
          <p className="text-xs text-muted-foreground">{t("archive.web.subtitle")}</p>
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
            placeholder={t("archive.web.urlPlaceholder")}
            required
          />
        </div>
        <div className="space-y-1">
          <label htmlFor={tokenId} className="text-xs font-medium">
            {t("archive.web.tokenLabel")}
          </label>
          <Input
            id={tokenId}
            type="password"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder={t("archive.web.tokenPlaceholder")}
            required
          />
        </div>
        {error != null && (
          <p role="alert" className="text-xs text-destructive">
            {t("archive.web.connectFailed", { error })}
          </p>
        )}
        <Button type="submit" className="w-full" disabled={connecting}>
          {connecting ? (
            <>
              <Loader2 className="mr-2 h-3 w-3 animate-spin" aria-hidden="true" />
              {t("archive.web.connecting")}
            </>
          ) : (
            t("archive.web.connect")
          )}
        </Button>
        <p className="text-px12 text-muted-foreground">{t("archive.web.storageHint")}</p>
      </form>
    </div>
  );
}
