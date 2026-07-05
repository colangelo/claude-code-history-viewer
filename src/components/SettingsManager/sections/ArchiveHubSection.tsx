/**
 * Settings section for the archive hub connection (URL + bearer token).
 * Values persist via UserSettings (archiveHubUrl / archiveHubToken).
 */

import * as React from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export interface ArchiveHubSectionProps {
  initialUrl?: string;
  initialToken?: string;
  onSave: (url: string, token: string) => void | Promise<void>;
}

export function ArchiveHubSection({
  initialUrl,
  initialToken,
  onSave,
}: ArchiveHubSectionProps) {
  const { t } = useTranslation();
  const [url, setUrl] = React.useState(initialUrl ?? "");
  const [token, setToken] = React.useState(initialToken ?? "");
  const [isSaving, setIsSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const urlInputId = React.useId();
  const tokenInputId = React.useId();

  React.useEffect(() => {
    setUrl(initialUrl ?? "");
  }, [initialUrl]);

  React.useEffect(() => {
    setToken(initialToken ?? "");
  }, [initialToken]);

  const handleSave = async () => {
    setIsSaving(true);
    setError(null);
    try {
      await onSave(url.trim(), token.trim());
    } catch (err) {
      setError(String(err));
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div data-testid="archive-hub-section" className="space-y-3 px-3 pb-3">
      <p className="text-xs text-muted-foreground">
        {t("settings.archiveHub.description")}
      </p>

      <div className="space-y-1">
        <Label htmlFor={urlInputId} className="text-xs">
          {t("settings.archiveHub.url")}
        </Label>
        <Input
          id={urlInputId}
          data-testid="archive-hub-url-input"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder={t("settings.archiveHub.urlPlaceholder")}
          className="h-8 text-xs font-mono"
          autoComplete="off"
        />
      </div>

      <div className="space-y-1">
        <Label htmlFor={tokenInputId} className="text-xs">
          {t("settings.archiveHub.token")}
        </Label>
        <Input
          id={tokenInputId}
          data-testid="archive-hub-token-input"
          type="password"
          value={token}
          onChange={(e) => setToken(e.target.value)}
          placeholder={t("settings.archiveHub.tokenPlaceholder")}
          className="h-8 text-xs font-mono"
          autoComplete="off"
        />
      </div>

      {error && <p className="text-xs text-destructive">{error}</p>}

      <div className="flex justify-end">
        <Button
          size="sm"
          className="h-7 text-xs"
          data-testid="archive-hub-save-button"
          onClick={handleSave}
          disabled={isSaving}
        >
          {t("settings.archiveHub.save")}
        </Button>
      </div>
    </div>
  );
}
