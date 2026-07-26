/**
 * Global Analytics Calculations
 *
 * Utility functions for global (cross-project) analytics calculations.
 */

import {
  calculateModelPrice,
  formatNumber,
  hasExplicitModelPricing,
} from "./calculations";

// ============================================================================
// Model Distribution Metrics
// ============================================================================

export interface ModelDisplayMetrics {
  percentage: number;
  price: number;
  formattedPrice: string;
  formattedTokens: string;
}

interface ModelUsageLike {
  model_name: string;
  token_count: number;
  input_tokens: number;
  output_tokens: number;
  cache_creation_tokens: number;
  cache_read_tokens: number;
}

/** One rendering rule for every dollar figure in Analytics, so the model,
 * provider and project cards cannot drift apart on formatting. Coarser as the
 * number grows: cents matter at $3.40, not at $8,777. */
export const formatUsd = (price: number): string =>
  `$${price.toFixed(price >= 100 ? 0 : price >= 10 ? 1 : 2)}`;

export interface RowCost {
  /** `null` when this hub reported no model breakdown for the row — render
   * "not reported by this hub version", NEVER `$0`, which would read as a
   * measured zero rather than an absent measurement. */
  formatted: string | null;
  /** Share of the ROW's own tokens that carry an explicitly priced model.
   * Below 100% for two distinct reasons that look the same here: rows whose
   * `model` is NULL (excluded from the breakdown server-side) and models with
   * no entry in the pricing table. */
  coveragePercent: number;
}

/**
 * Cost for one provider or project row, priced from its own model breakdown.
 *
 * Cost is not stored — `cost_usd` is NULL throughout the archive — so it is
 * derived from the model AND the token type, which is why a row needs its
 * breakdown rather than just a token total. Priced against the row's own
 * tokens so the coverage figure describes that row, not the archive.
 */
export const calculateRowCost = (
  models: ModelUsageLike[] | undefined,
  rowTokens: number
): RowCost => {
  if (!models || models.length === 0) {
    return { formatted: null, coveragePercent: 0 };
  }
  const { totalEstimatedCost, coveragePercent } = calculateGlobalCostSummary(
    models,
    rowTokens
  );
  return { formatted: formatUsd(totalEstimatedCost), coveragePercent };
};

export interface GlobalCostSummary {
  totalEstimatedCost: number;
  coveragePercent: number;
  coveredTokens: number;
}

/**
 * Calculate display metrics for a single model
 */
export const calculateModelMetrics = (
  modelName: string,
  tokenCount: number,
  inputTokens: number,
  outputTokens: number,
  cacheCreationTokens: number,
  cacheReadTokens: number,
  totalTokens: number
): ModelDisplayMetrics => {
  const price = calculateModelPrice(
    modelName,
    inputTokens,
    outputTokens,
    cacheCreationTokens,
    cacheReadTokens
  );

  const percentage = (tokenCount / Math.max(totalTokens, 1)) * 100;

  const formattedPrice = formatUsd(price);
  const formattedTokens = formatNumber(tokenCount);

  return {
    percentage,
    price,
    formattedPrice,
    formattedTokens,
  };
};

export const calculateGlobalCostSummary = (
  models: ModelUsageLike[],
  totalTokens: number
): GlobalCostSummary => {
  let totalEstimatedCost = 0;
  let coveredTokens = 0;

  for (const model of models) {
    totalEstimatedCost += calculateModelPrice(
      model.model_name,
      model.input_tokens,
      model.output_tokens,
      model.cache_creation_tokens,
      model.cache_read_tokens
    );

    if (hasExplicitModelPricing(model.model_name)) {
      coveredTokens += model.token_count;
    }
  }

  const denominator = Math.max(totalTokens, 1);
  const coveragePercent = (coveredTokens / denominator) * 100;

  return {
    totalEstimatedCost,
    coveragePercent,
    coveredTokens,
  };
};

// ============================================================================
// Project Ranking
// ============================================================================

export type RankMedal = "🥇" | "🥈" | "🥉" | null;

/**
 * Get medal emoji for top 3 ranks
 */
export const getRankMedal = (index: number): RankMedal => {
  const medals: RankMedal[] = ["🥇", "🥈", "🥉"];
  return index < 3 ? (medals[index] as RankMedal) : null;
};

/**
 * Check if index qualifies for medal display
 */
export const hasMedal = (index: number): boolean => index < 3;
