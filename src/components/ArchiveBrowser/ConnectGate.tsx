/**
 * Connect gate for the standalone static archive webapp
 * (spec: openspec/specs/static-archive-webapp/spec.md).
 *
 * Owns the hub connection lifecycle: shows a URL+token form when nothing is
 * stored, probes the hub with an authenticated call before persisting
 * (healthz alone would not validate the token), and renders `ArchiveBrowser`
 * once connected. The config lives in `localStorage` only — the static build
 * has no settings backend.
 */

import { useCallback, useId, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ArchiveBrowser } from "./index";
import { hubApi, type HubConfig } from "../../services/hubApi";
import {
  clearStoredHubConfig,
  loadStoredHubConfig,
  storeHubConfig,
} from "./hubConfigStorage";

export function ConnectGate() {
  const { t } = useTranslation();
  const [config, setConfig] = useState<HubConfig | null>(() => loadStoredHubConfig());
  const [url, setUrl] = useState("");
  const [token, setToken] = useState("");
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const urlId = useId();
  const tokenId = useId();

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

  if (config) {
    return (
      <div className="flex flex-col h-full gap-2">
        <div className="flex items-center justify-between shrink-0 border-b border-border pb-2">
          <h1 className="text-sm font-semibold">{t("archive.web.title")}</h1>
          <Button variant="outline" size="sm" onClick={handleDisconnect}>
            {t("archive.web.disconnect")}
          </Button>
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
        <p className="text-[11px] text-muted-foreground">{t("archive.web.storageHint")}</p>
      </form>
    </div>
  );
}
