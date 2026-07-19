/**
 * Theme cycle button for the static archive webapp header: light → dark →
 * system. Persistence rides the existing ThemeProvider path (storageAdapter →
 * localStorage in web mode), so the choice survives reloads.
 */

import { Moon, Sun, MonitorSmartphone } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useTheme, type Theme } from "@/contexts/theme";

const ORDER: Theme[] = ["light", "dark", "system"];

export function ThemeToggle() {
  const { t } = useTranslation();
  const { theme, setTheme } = useTheme();

  const labels: Record<Theme, string> = {
    light: t("archive.web.themeLight"),
    dark: t("archive.web.themeDark"),
    system: t("archive.web.themeSystem"),
  };
  const Icon =
    theme === "light" ? Sun : theme === "dark" ? Moon : MonitorSmartphone;
  const next = ORDER[(ORDER.indexOf(theme) + 1) % ORDER.length]!;

  // Borderless to pair with the `Aa` trigger beside it: the two are quiet
  // reader utilities and should read as one group, not as two more outlined
  // buttons competing with Disconnect.
  return (
    <button
      type="button"
      data-testid="theme-toggle"
      onClick={() => void setTheme(next)}
      aria-label={t("archive.web.themeToggle")}
      title={`${t("archive.web.themeToggle")}: ${labels[theme]}`}
      className="h-7 w-7 flex items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
    >
      <Icon className="w-3.5 h-3.5" aria-hidden="true" />
    </button>
  );
}
