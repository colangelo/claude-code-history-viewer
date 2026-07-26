/**
 * ModelDistributionCard Component
 *
 * Per-model token/cost breakdown, shared by the global and project analytics
 * views (analytics-ux-costs): extracted from GlobalStatsView when the project
 * scope gained `model_distribution`, so the two scopes cannot drift.
 */

import React from "react";
import { useTranslation } from "react-i18next";
import { Cpu } from "lucide-react";
import type { ModelStats } from "../../../types";
import { SectionCard } from "./SectionCard";
import { calculateModelMetrics } from "../utils";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";

interface ModelDistributionCardProps {
  models: ModelStats[];
  totalTokens: number;
}

/** Claude Code's own sentinel for messages it generated itself rather than
 * getting from a model — injected errors ("API Error", "Prompt is too long")
 * and similar. It arrives as a literal `model` value and the archive stores it
 * faithfully, but it is not a model: it carries zero tokens, so it renders as
 * a 0% row with an empty bar and only dilutes the breakdown. Filtered here
 * rather than in the hub's rollup so the API stays a truthful record of what
 * was stored, and so the fix reaches the desktop dashboard too — both scopes
 * share this card. */
const NON_MODEL_SENTINEL = "<synthetic>";

export const ModelDistributionCard: React.FC<ModelDistributionCardProps> = ({
  models,
  totalTokens,
}) => {
  const { t } = useTranslation();

  const realModels = models.filter((m) => m.model_name !== NON_MODEL_SENTINEL);
  // Callers gate on the UNFILTERED length, so a scope whose only entry was the
  // sentinel would otherwise render an empty card.
  if (realModels.length === 0) return null;

  return (
    <SectionCard title={t("analytics.modelDistribution")} icon={Cpu} colorVariant="blue">
      <div className="space-y-3">
        {realModels.map((model) => {
          const { percentage, formattedPrice, formattedTokens } = calculateModelMetrics(
            model.model_name,
            model.token_count,
            model.input_tokens,
            model.output_tokens,
            model.cache_creation_tokens,
            model.cache_read_tokens,
            totalTokens
          );

          return (
            <div key={model.model_name}>
              <div className="flex items-center justify-between mb-1.5">
                <Tooltip>
                  <TooltipTrigger asChild>
                    <button
                      type="button"
                      className="block max-w-[60%] text-px12 font-medium text-foreground truncate text-left cursor-default"
                    >
                      {model.model_name}
                    </button>
                  </TooltipTrigger>
                  <TooltipContent>
                    {model.model_name}
                  </TooltipContent>
                </Tooltip>
                <div className="flex items-center gap-2">
                  <span className="font-mono text-px12 text-muted-foreground">
                    {formattedPrice}
                  </span>
                  <span className="font-mono text-px12 font-semibold text-foreground">
                    {formattedTokens}
                  </span>
                </div>
              </div>
              <div className="h-2 bg-muted/30 rounded-full overflow-hidden">
                <div
                  className="h-full rounded-full"
                  style={{
                    width: `${percentage}%`,
                    background:
                      "linear-gradient(90deg, var(--metric-purple), var(--metric-blue))",
                  }}
                />
              </div>
            </div>
          );
        })}
      </div>
    </SectionCard>
  );
};

ModelDistributionCard.displayName = "ModelDistributionCard";
