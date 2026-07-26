/**
 * ProjectStatsView Component
 *
 * Displays project-level analytics and statistics.
 */

import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { MessageCircle, Activity, Clock, DollarSign, Layers, Cpu, TrendingUp, Database, Sparkles, Bot } from "lucide-react";
import { LoadingState } from "@/components/ui/loading";
import type { ProjectStatsSummary, ProviderId } from "../../../types";
import { formatDuration } from "../../../utils/time";
import {
  MetricCard,
  SectionCard,
  BillingBreakdownCard,
  ActivityHeatmapComponent,
  ToolUsageChart,
  DailyTrendChart,
  TokenDistributionChart,
  ModelDistributionCard,
} from "../components";
import {
  formatNumber,
  formatCurrency,
  calculateGlobalCostSummary,
  generateTrendData,
  extractProjectGrowth,
} from "../utils";
import { supportsConversationBreakdown } from "../../../utils/providers";

interface ProjectStatsViewProps {
  projectSummary: ProjectStatsSummary | null;
  conversationSummary: ProjectStatsSummary | null;
  providerId?: ProviderId;
  /** Hide the conversation/non-conversation billing split. The hub does not
   * model that distinction, so rendering it there would show an invented
   * 100%/0% breakdown. Defaults to shown, leaving the desktop untouched. */
  showBillingBreakdown?: boolean;
}

export const ProjectStatsView: React.FC<ProjectStatsViewProps> = ({
  projectSummary,
  conversationSummary,
  showBillingBreakdown = true,
  providerId = "claude",
}) => {
  const { t } = useTranslation();

  // Generate full range daily data using utility function
  const dailyData = useMemo(
    () => generateTrendData(projectSummary?.daily_stats),
    [projectSummary?.daily_stats]
  );

  // Estimated cost, priced client-side from the per-model split
  // (analytics-ux-costs). `null` when the hub predates the field — that MUST
  // render as "unavailable", never as $0.00, which would read as "free".
  const costSummary = useMemo(() => {
    const models = projectSummary?.model_distribution;
    if (!models || models.length === 0) return null;
    return calculateGlobalCostSummary(models, projectSummary.total_tokens);
  }, [projectSummary]);

  // 데이터가 없으면 항상 로딩 상태 표시 (뷰 전환 직후 isLoading이 false일 수 있음)
  if (!projectSummary) {
    return (
      <div className="flex items-center justify-center h-full min-h-[400px]">
        <LoadingState
          isLoading={true}
          loadingMessage={t("analytics.loading")}
          spinnerSize="lg"
          withSparkle={true}
        />
      </div>
    );
  }

  // Calculate growth metrics using utility function
  const { tokenGrowth, messageGrowth } = extractProjectGrowth(projectSummary);
  const billingTokens = projectSummary.total_tokens;
  const modelDistribution = projectSummary.model_distribution ?? [];

  return (
    <div className="space-y-6 animate-stagger">
      {/* Metric Cards Grid. Cost leads, mirroring the global view — it was
          the one metric that vanished when a project was selected. */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
        <MetricCard
          icon={DollarSign}
          label={t("analytics.estimatedCost")}
          value={
            costSummary ? formatCurrency(costSummary.totalEstimatedCost) : "—"
          }
          subValue={
            costSummary
              ? `${t("analytics.pricingCoverage", "Pricing coverage")}: ${costSummary.coveragePercent.toFixed(1)}%`
              : t("analytics.costUnavailableHub")
          }
          colorVariant="amber"
        />
        <MetricCard
          icon={Activity}
          label={t("analytics.totalTokens")}
          value={formatNumber(projectSummary.total_tokens)}
          trend={tokenGrowth}
          subValue={t("analytics.sessionCount", "{{count}} sessions", {
            count: projectSummary.total_sessions,
          })}
          colorVariant="blue"
        />
        <MetricCard
          icon={MessageCircle}
          label={t("analytics.totalMessages")}
          value={formatNumber(projectSummary.total_messages)}
          trend={messageGrowth}
          colorVariant="purple"
        />
        <MetricCard
          icon={Clock}
          label={t("analytics.totalSessionTime")}
          value={formatDuration(projectSummary.total_session_duration)}
          subValue={`${t("analytics.avgSessionTime", "Avg Session Time")}: ${formatDuration(
            projectSummary.avg_session_duration
          )}`}
          colorVariant="green"
        />
      </div>

      {costSummary && (
        <div className="flex flex-wrap items-center gap-2">
          <span className="px-2 py-1 rounded-md bg-amber-500/10 text-amber-700 dark:text-amber-300 text-px11">
            {t("analytics.estimatedLabel", "Estimated")}
          </span>
        </div>
      )}

      {showBillingBreakdown && (
      <BillingBreakdownCard
        billingTokens={billingTokens}
        conversationTokens={conversationSummary != null ? conversationSummary.total_tokens : null}
        showProviderLimitHelp={!supportsConversationBreakdown(providerId)}
      />
      )}

      {/* Charts Row 1 — heatmap and per-model costs lead (analytics-ux-costs) */}
      <div className="grid grid-cols-1 lg:grid-cols-2 items-start gap-5">
        <SectionCard title={t("analytics.activityHeatmapTitle")} icon={Layers} colorVariant="green">
          {projectSummary.daily_stats.length > 0 ? (
            <ActivityHeatmapComponent data={projectSummary.daily_stats} />
          ) : (
            <div className="text-center py-8 text-muted-foreground text-px12">
              {t("analytics.No activity data available")}
            </div>
          )}
        </SectionCard>

        {modelDistribution.length > 0 && (
          <ModelDistributionCard
            models={modelDistribution}
            totalTokens={projectSummary.total_tokens}
          />
        )}
      </div>

      {/* Tool / Skill / Subagent usage (#321) */}
      <div className="grid grid-cols-1 lg:grid-cols-2 items-start gap-5">
        <SectionCard title={t("analytics.mostUsedToolsTitle")} icon={Cpu} colorVariant="purple">
          <ToolUsageChart tools={projectSummary.most_used_tools} />
        </SectionCard>
        {projectSummary.most_used_skills.length > 0 && (
          <SectionCard title={t("analytics.mostUsedSkillsTitle")} icon={Sparkles} colorVariant="pink">
            <ToolUsageChart tools={projectSummary.most_used_skills} />
          </SectionCard>
        )}
        {projectSummary.most_used_subagents.length > 0 && (
          <SectionCard title={t("analytics.mostUsedSubagentsTitle")} icon={Bot} colorVariant="teal">
            <ToolUsageChart tools={projectSummary.most_used_subagents} />
          </SectionCard>
        )}
      </div>

      {/* Daily Trend Chart */}
      {projectSummary.daily_stats.length > 0 && (
        <SectionCard title={t("analytics.recentActivityTrend")} icon={TrendingUp} colorVariant="blue">
          <DailyTrendChart dailyData={dailyData} />
        </SectionCard>
      )}

      {/* Token Distribution */}
      <SectionCard title={t("analytics.tokenTypeDistribution")} icon={Database} colorVariant="amber">
        <TokenDistributionChart
          distribution={projectSummary.token_distribution}
          total={projectSummary.total_tokens}
        />
      </SectionCard>
    </div>
  );
};

ProjectStatsView.displayName = "ProjectStatsView";
