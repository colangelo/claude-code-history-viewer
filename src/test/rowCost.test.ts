/**
 * Per-row cost pricing for the provider and project cards.
 *
 * The branch that matters is the absent one: a hub predating the per-row model
 * breakdown must render "not reported by this hub version", never `$0` — a
 * fabricated zero reads as a measured zero, which is the one wrong answer.
 */

import { describe, expect, it } from "vitest";
import { calculateRowCost, formatUsd } from "../components/AnalyticsDashboard/utils";
import type { ModelStats } from "../types";

function model(name: string, tokens: number): ModelStats {
  return {
    model_name: name,
    message_count: 1,
    token_count: tokens,
    input_tokens: tokens,
    output_tokens: 0,
    cache_creation_tokens: 0,
    cache_read_tokens: 0,
  };
}

describe("calculateRowCost", () => {
  it("returns null — not $0 — when the hub reported no breakdown", () => {
    expect(calculateRowCost(undefined, 1000).formatted).toBeNull();
    expect(calculateRowCost([], 1000).formatted).toBeNull();
  });

  it("prices a row from its own model breakdown", () => {
    const cost = calculateRowCost([model("claude-opus-4-7", 1_000_000)], 1_000_000);
    expect(cost.formatted).not.toBeNull();
    expect(cost.formatted).toMatch(/^\$/);
  });

  it("reports coverage against the ROW's tokens, not the archive's", () => {
    // Half the row's tokens come from a model with no entry in the pricing
    // table, so coverage must read ~50%: it is still *priced* (via the default
    // rate) but not *covered*, which is exactly the distinction the number
    // exists to make.
    const cost = calculateRowCost(
      [model("claude-opus-4-7", 500), model("some-unpriced-local-gguf", 500)],
      1000
    );
    expect(cost.coveragePercent).toBeGreaterThan(40);
    expect(cost.coveragePercent).toBeLessThan(60);
  });

  it("coverage falls below 100% when the row has tokens no model claims", () => {
    // Rows with a NULL model are excluded from the breakdown server-side, so
    // the split can legitimately be smaller than the row's own total.
    const cost = calculateRowCost([model("claude-opus-4-7", 400)], 1000);
    expect(cost.coveragePercent).toBeCloseTo(40, 0);
  });
});

describe("formatUsd", () => {
  it("gets coarser as the number grows", () => {
    expect(formatUsd(3.404)).toBe("$3.40");
    expect(formatUsd(42.44)).toBe("$42.4");
    expect(formatUsd(8777.4)).toBe("$8777");
  });
});
