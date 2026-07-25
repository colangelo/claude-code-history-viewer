/**
 * Analytics view for the standalone archive webapp.
 *
 * Every figure comes from the hub's `/v1/stats/*` endpoints — nothing is
 * aggregated client-side from fetched messages. The hub returns the same stat
 * shapes the desktop produced, so `GlobalStatsView` / `ProjectStatsView` and
 * their charts are reused unchanged; only the data source differs.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Loader2 } from "lucide-react";

import { GlobalStatsView, ProjectStatsView } from "../AnalyticsDashboard/views";
import {
  hubApi,
  HubHttpError,
  type HubConfig,
  type HubIdentity,
} from "../../services/hubApi";
import type { GlobalStatsSummary, ProjectStatsSummary } from "../../types";
import { cn } from "@/lib/utils";

export interface AnalyticsViewProps {
  config: HubConfig;
  /** Identities offered as scopes; supplied by the browser shell. */
  identities: HubIdentity[];
}

/** `all` = whole archive; otherwise a project identity key. */
type Scope = { kind: "all" } | { kind: "identity"; key: string; label: string };

/** Relative windows, resolved against today in the viewer's timezone. */
const RANGES = [
  { id: "30d", days: 30 },
  { id: "90d", days: 90 },
  { id: "all", days: null },
] as const;
type RangeId = (typeof RANGES)[number]["id"];

function isoDaysAgo(days: number): string {
  const d = new Date();
  d.setDate(d.getDate() - days);
  return d.toISOString().slice(0, 10);
}

export function AnalyticsView({ config, identities }: AnalyticsViewProps) {
  const { t } = useTranslation();
  const [scope, setScope] = useState<Scope>({ kind: "all" });
  const [range, setRange] = useState<RangeId>("30d");
  const [global, setGlobal] = useState<GlobalStatsSummary | null>(null);
  const [project, setProject] = useState<ProjectStatsSummary | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** Hub predates `/v1/stats/*` — a routine upgrade prompt, not a failure. */
  const [unsupported, setUnsupported] = useState(false);

  // The hub buckets days and hours server-side, so it needs the viewer's zone;
  // otherwise "what hours do I work" is answered in UTC.
  const tz = useMemo(() => {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
    } catch {
      return "UTC";
    }
  }, []);

  const load = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    setUnsupported(false);
    const days = RANGES.find((r) => r.id === range)?.days ?? null;
    const options = { tz, ...(days ? { from: isoDaysAgo(days) } : {}) };
    try {
      if (scope.kind === "all") {
        setProject(null);
        setGlobal(await hubApi.statsGlobal(config, options));
      } else {
        setGlobal(null);
        setProject(await hubApi.statsProject(config, scope.key, options));
      }
    } catch (e) {
      if (e instanceof HubHttpError && e.status === 404) {
        // Distinguish "this hub is too old" from "this identity is unknown":
        // the global endpoint 404s only in the former case.
        setUnsupported(scope.kind === "all");
        if (scope.kind !== "all") setError(t("analytics.scopeNotFound"));
      } else {
        setError(e instanceof Error ? e.message : String(e));
      }
      setGlobal(null);
      setProject(null);
    } finally {
      setIsLoading(false);
    }
  }, [config, range, scope, t, tz]);

  useEffect(() => {
    void load();
  }, [load]);

  const scopes: Scope[] = useMemo(
    () => [
      { kind: "all" as const },
      ...identities
        .filter((i) => i.identity_key)
        .map((i) => ({
          kind: "identity" as const,
          key: i.identity_key as string,
          label: i.display_name ?? (i.identity_key as string),
        })),
    ],
    [identities]
  );

  const scopeId = scope.kind === "all" ? "all" : scope.key;

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <label className="sr-only" htmlFor="analytics-scope">
          {t("analytics.scope")}
        </label>
        <select
          id="analytics-scope"
          className="rounded-md border bg-background px-2 py-1 text-sm"
          value={scopeId}
          onChange={(e) => {
            const next = scopes.find(
              (s) => (s.kind === "all" ? "all" : s.key) === e.target.value
            );
            if (next) setScope(next);
          }}
        >
          {scopes.map((s) => (
            <option key={s.kind === "all" ? "all" : s.key} value={s.kind === "all" ? "all" : s.key}>
              {s.kind === "all" ? t("analytics.allProjects") : s.label}
            </option>
          ))}
        </select>

        <div className="flex items-center gap-1">
          {RANGES.map((r) => (
            <button
              key={r.id}
              type="button"
              onClick={() => setRange(r.id)}
              className={cn(
                "rounded-md px-2 py-1 text-xs transition-colors",
                range === r.id
                  ? "bg-accent/15 text-accent-foreground"
                  : "text-muted-foreground hover:bg-accent/10"
              )}
            >
              {t(`analytics.range.${r.id}`)}
            </button>
          ))}
        </div>

        {isLoading && (
          <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" aria-hidden />
        )}
      </div>

      {unsupported && (
        <div className="flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-sm">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-500" aria-hidden />
          <div>
            <p className="font-medium">{t("analytics.hubTooOld")}</p>
            <p className="text-muted-foreground">{t("analytics.hubTooOldHint")}</p>
          </div>
        </div>
      )}

      {error && !unsupported && (
        <p className="rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm">
          {error}
        </p>
      )}

      {global && (
        // `globalConversationSummary` is the desktop's second pass over local
        // files; the hub models no such split, so it is passed as the same
        // summary rather than null — null leaves the billing card showing
        // "Calculating" forever, which reads as a hung request.
        <GlobalStatsView globalSummary={global} globalConversationSummary={global} />
      )}
      {project && (
        <ProjectStatsView projectSummary={project} conversationSummary={project} />
      )}

      {!isLoading && !global && !project && !error && !unsupported && (
        <p className="p-6 text-center text-sm text-muted-foreground">
          {t("analytics.empty")}
        </p>
      )}
    </div>
  );
}
