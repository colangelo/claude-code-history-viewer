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
import {
  RANGE_ALL,
  RANGE_CUSTOM,
  RANGE_PRESET_DAYS,
  isoDaysAgo,
  parseCustomDays,
} from "./analyticsRange";

export interface AnalyticsViewProps {
  config: HubConfig;
  /** Identities offered as scopes; supplied by the browser shell. */
  identities: HubIdentity[];
}

/** `all` = whole archive; otherwise a project identity key. */
type Scope = { kind: "all" } | { kind: "identity"; key: string; label: string };

export function AnalyticsView({ config, identities }: AnalyticsViewProps) {
  const { t } = useTranslation();
  const [scope, setScope] = useState<Scope>({ kind: "all" });
  // The select holds a preset day count, "all", or "custom"; a committed
  // custom count lives beside it so an uncommitted draft keeps the previous
  // window instead of fetching something surprising (analytics-ux-costs).
  const [rangeSel, setRangeSel] = useState<string>("30");
  const [customDays, setCustomDays] = useState<number | null>(null);
  const [customDraft, setCustomDraft] = useState("");
  const rangeDays: number | null =
    rangeSel === RANGE_ALL
      ? null
      : rangeSel === RANGE_CUSTOM
        ? customDays
        : Number(rangeSel);
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
    // Custom picked but no count committed yet: keep showing the previous
    // window rather than silently fetching all time.
    if (rangeSel === RANGE_CUSTOM && rangeDays == null) return;
    setIsLoading(true);
    setError(null);
    setUnsupported(false);
    const options = {
      tz,
      ...(rangeDays ? { from: isoDaysAgo(rangeDays) } : {}),
    };
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
  }, [config, rangeSel, rangeDays, scope, t, tz]);

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

        <label className="sr-only" htmlFor="analytics-range">
          {t("analytics.range.label")}
        </label>
        <select
          id="analytics-range"
          data-testid="analytics-range"
          className="rounded-md border bg-background px-2 py-1 text-sm"
          value={rangeSel}
          onChange={(e) => {
            setRangeSel(e.target.value);
            if (e.target.value !== RANGE_CUSTOM) {
              setCustomDays(null);
              setCustomDraft("");
            }
          }}
        >
          {RANGE_PRESET_DAYS.map((d) => (
            <option key={d} value={String(d)}>
              {t("analytics.range.days", { days: d })}
            </option>
          ))}
          <option value={RANGE_ALL}>{t("analytics.range.all")}</option>
          <option value={RANGE_CUSTOM}>{t("analytics.range.custom")}</option>
        </select>
        {rangeSel === RANGE_CUSTOM && (
          <input
            type="number"
            min={1}
            data-testid="analytics-range-custom"
            aria-label={t("analytics.range.customDays")}
            placeholder={t("analytics.range.customDays")}
            className="w-28 rounded-md border bg-background px-2 py-1 text-sm"
            value={customDraft}
            onChange={(e) => setCustomDraft(e.target.value)}
            // Applied on commit (blur / Enter), not per keystroke — every
            // committed value triggers a full stats fetch.
            onBlur={() => setCustomDays(parseCustomDays(customDraft))}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                setCustomDays(parseCustomDays(customDraft));
              }
            }}
          />
        )}

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
        // The hub models no conversation/non-conversation split, so that card
        // is hidden rather than shown with an invented 100%/0% breakdown.
        <GlobalStatsView
          globalSummary={global}
          globalConversationSummary={null}
          showBillingBreakdown={false}
        />
      )}
      {project && (
        <ProjectStatsView
          projectSummary={project}
          conversationSummary={null}
          showBillingBreakdown={false}
        />
      )}

      {!isLoading && !global && !project && !error && !unsupported && (
        <p className="p-6 text-center text-sm text-muted-foreground">
          {t("analytics.empty")}
        </p>
      )}
    </div>
  );
}
