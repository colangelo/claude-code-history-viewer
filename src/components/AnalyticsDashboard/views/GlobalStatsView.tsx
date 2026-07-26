/**
 * GlobalStatsView Component
 *
 * Displays global statistics across all projects.
 */

import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import {
  Activity,
  MessageCircle,
  Clock,
  DollarSign,
  Wrench,
  Layers,
  BarChart3,
  Server,
  Sparkles,
  Bot,
} from "lucide-react";
import type { GlobalStatsSummary } from "../../../types";
import { formatDuration } from "../../../utils/time";
import { cn } from "@/lib/utils";
import {
  MetricCard,
  SectionCard,
  BillingBreakdownCard,
  ActivityHeatmapComponent,
  ToolUsageChart,
  ProviderDistributionChart,
  ModelDistributionCard,
} from "../components";
import {
  formatNumber,
  formatCurrency,
  calculateGlobalCostSummary,
  getRankMedal,
  hasMedal,
} from "../utils";
import { calculateConversationBreakdownCoverage } from "../../../utils/providers";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";

interface GlobalStatsViewProps {
  globalSummary: GlobalStatsSummary;
  globalConversationSummary: GlobalStatsSummary | null;
  /** Hide the conversation/non-conversation billing split. The hub does not
   * model that distinction, so rendering it there would show an invented
   * 100%/0% breakdown. Defaults to shown, leaving the desktop untouched. */
  showBillingBreakdown?: boolean;
}

export const GlobalStatsView: React.FC<GlobalStatsViewProps> = ({
  globalSummary,
  globalConversationSummary,
  showBillingBreakdown = true,
}) => {
  const { t } = useTranslation();
  const totalSessionTime = globalSummary.total_session_duration_minutes;
  const costSummary = useMemo(
    () =>
      calculateGlobalCostSummary(
        globalSummary.model_distribution,
        globalSummary.total_tokens
      ),
    [globalSummary.model_distribution, globalSummary.total_tokens]
  );
  const totalEstimatedCost = costSummary.totalEstimatedCost;
  const conversationCostSummary = useMemo(() => {
    if (!globalConversationSummary) {
      return null;
    }
    return calculateGlobalCostSummary(
      globalConversationSummary.model_distribution,
      globalConversationSummary.total_tokens
    );
  }, [globalConversationSummary]);

  const billingTokens = globalSummary.total_tokens;
  const billingCost = totalEstimatedCost;
  const conversationBreakdownCoverage = useMemo(
    () =>
      calculateConversationBreakdownCoverage(globalSummary.provider_distribution),
    [globalSummary.provider_distribution]
  );

  const lastUpdated = useMemo(() => {
    const raw = globalSummary.date_range.last_message;
    if (!raw) {
      return t("analytics.lastUpdatedUnknown", "Unknown");
    }
    const parsed = new Date(raw);
    if (Number.isNaN(parsed.getTime())) {
      return t("analytics.lastUpdatedUnknown", "Unknown");
    }
    return new Intl.DateTimeFormat(undefined, {
      year: "numeric",
      month: "short",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    }).format(parsed);
  }, [globalSummary.date_range.last_message, t]);

  return (
    <div className="flex-1 p-3 md:p-6 overflow-auto bg-background space-y-4 md:space-y-6 animate-stagger">
      <p className="text-px11 text-muted-foreground">
        {t(
          "analytics.providerScopeProjectTree",
          "Provider scope follows Project Tree provider tabs."
        )}
      </p>

      {/* Metric Cards. Cost leads (analytics-ux-costs): it is the figure a
          user looks for first. The old "N tools used" card is gone — a count
          of distinct tools was the least informative number on the page, and
          its content lives in the Most Used Tools chart below. */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-3 md:gap-4">
        <MetricCard
          icon={DollarSign}
          label={t("analytics.estimatedCost")}
          value={formatCurrency(totalEstimatedCost)}
          subValue={`${t("analytics.pricingCoverage", "Pricing coverage")}: ${costSummary.coveragePercent.toFixed(1)}%`}
          colorVariant="amber"
        />
        <MetricCard
          icon={Activity}
          label={t("analytics.totalTokens")}
          value={formatNumber(globalSummary.total_tokens)}
          colorVariant="blue"
        />
        <MetricCard
          icon={MessageCircle}
          label={t("analytics.totalMessages")}
          value={formatNumber(globalSummary.total_messages)}
          subValue={`${t("analytics.totalSessions")}: ${globalSummary.total_sessions}`}
          colorVariant="purple"
        />
        <MetricCard
          icon={Clock}
          label={t("analytics.sessionTime")}
          value={formatDuration(totalSessionTime)}
          colorVariant="green"
        />
      </div>

      {showBillingBreakdown && (
      <BillingBreakdownCard
        billingTokens={billingTokens}
        conversationTokens={globalConversationSummary?.total_tokens ?? null}
        billingCost={billingCost}
        conversationCost={conversationCostSummary?.totalEstimatedCost ?? null}
        showProviderLimitHelp={conversationBreakdownCoverage.hasLimitedProviders}
      />
      )}

      {/* Coverage moved onto the cost card itself — one fact, one place. */}
      <div className="flex flex-wrap items-center gap-2">
        <span className="px-2 py-1 rounded-md bg-amber-500/10 text-amber-700 dark:text-amber-300 text-px11">
          {t("analytics.estimatedLabel", "Estimated")}
        </span>
        <span className="px-2 py-1 rounded-md bg-muted/40 text-muted-foreground text-px11">
          {t("analytics.lastUpdated", "Last updated")}: {lastUpdated}
        </span>
      </div>

      {/* Section order (analytics-ux-costs): the heatmap and per-model costs
          lead — they are what gets read; tool charts follow. */}
      <div className="grid grid-cols-1 lg:grid-cols-2 items-start gap-5">
        <SectionCard title={t("analytics.activityHeatmapTitle")} icon={Layers} colorVariant="green">
          {globalSummary.daily_stats.length > 0 ? (
            <ActivityHeatmapComponent data={globalSummary.daily_stats} />
          ) : (
            <div className="text-center py-8 text-muted-foreground text-px12">
              {t("analytics.No activity data available")}
            </div>
          )}
        </SectionCard>

        {globalSummary.model_distribution.length > 0 && (
          <ModelDistributionCard
            models={globalSummary.model_distribution}
            totalTokens={globalSummary.total_tokens}
          />
        )}
      </div>

      {/* Provider split & Top Projects */}
      <div className="grid grid-cols-1 lg:grid-cols-2 items-start gap-5">
        {globalSummary.provider_distribution.length > 0 && (
          <SectionCard
            title={t("analytics.providerDistribution", "Provider Distribution")}
            icon={Server}
            colorVariant="green"
          >
            <ProviderDistributionChart providers={globalSummary.provider_distribution} />
          </SectionCard>
        )}

        {globalSummary.top_projects.length > 0 && (
          <SectionCard title={t("analytics.topProjects")} icon={BarChart3} colorVariant="purple">
            <div className="space-y-2">
              {globalSummary.top_projects.slice(0, 8).map((project, index) => {
                const medal = getRankMedal(index);
                return (
                  <div
                    key={project.project_name}
                    className={cn(
                      "flex items-center justify-between p-2.5 rounded-lg",
                      "bg-muted/30 hover:bg-muted/50 transition-colors"
                    )}
                  >
                    <div className="flex items-center gap-3 flex-1 min-w-0">
                      <div
                        className={cn(
                          "w-6 h-6 rounded-md flex items-center justify-center text-px12 font-bold",
                          hasMedal(index) ? "text-base" : "bg-muted text-muted-foreground"
                        )}
                      >
                        {medal ?? index + 1}
                      </div>
                      <div className="flex-1 min-w-0">
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <button
                              type="button"
                              className="block w-full text-px12 font-medium text-foreground truncate text-left cursor-default"
                            >
                              {project.project_name}
                            </button>
                          </TooltipTrigger>
                          <TooltipContent>
                            {project.project_name}
                          </TooltipContent>
                        </Tooltip>
                        <p className="text-px12 text-muted-foreground">
                          {t(
                            "analytics.topProjectMeta",
                            "{{sessions}} sessions • {{messages}} msgs",
                            {
                              sessions: project.sessions,
                              messages: project.messages,
                            }
                          )}
                        </p>
                      </div>
                    </div>
                    <div className="text-right">
                      <p className="font-mono text-px12 font-bold text-foreground">
                        {formatNumber(project.tokens)}
                      </p>
                      <p className="text-px12 text-muted-foreground">{t("analytics.tokens")}</p>
                    </div>
                  </div>
                );
              })}
            </div>
          </SectionCard>
        )}
      </div>

      {/* Tool / Skill / Subagent usage (#321) — demoted below the charts that
          answer "what did this cost and when was I active". */}
      <div className="grid grid-cols-1 lg:grid-cols-2 items-start gap-5">
        <SectionCard
          title={t("analytics.mostUsedToolsTitle")}
          icon={Wrench}
          colorVariant="amber"
        >
          <ToolUsageChart tools={globalSummary.most_used_tools} />
        </SectionCard>
        {globalSummary.most_used_skills.length > 0 && (
          <SectionCard title={t("analytics.mostUsedSkillsTitle")} icon={Sparkles} colorVariant="pink">
            <ToolUsageChart tools={globalSummary.most_used_skills} />
          </SectionCard>
        )}
        {globalSummary.most_used_subagents.length > 0 && (
          <SectionCard title={t("analytics.mostUsedSubagentsTitle")} icon={Bot} colorVariant="teal">
            <ToolUsageChart tools={globalSummary.most_used_subagents} />
          </SectionCard>
        )}
      </div>
    </div>
  );
};

GlobalStatsView.displayName = "GlobalStatsView";
