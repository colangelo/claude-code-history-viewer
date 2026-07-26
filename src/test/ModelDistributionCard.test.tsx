/**
 * `<synthetic>` containment in the model breakdown (cchv #32).
 *
 * Claude Code writes `model: "<synthetic>"` on messages it generated itself
 * (injected errors and the like). The archive stores it faithfully, so it
 * reaches the client as a zero-token "model" and used to render as a 0% row.
 */

import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ModelDistributionCard } from "../components/AnalyticsDashboard/components/ModelDistributionCard";
import type { ModelStats } from "../types";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

function model(name: string, tokens: number): ModelStats {
  return {
    model_name: name,
    message_count: 10,
    token_count: tokens,
    input_tokens: tokens,
    output_tokens: 0,
    cache_creation_tokens: 0,
    cache_read_tokens: 0,
  };
}

describe("ModelDistributionCard", () => {
  it("hides the <synthetic> sentinel but keeps real models", () => {
    render(
      <ModelDistributionCard
        models={[
          model("claude-opus-5", 1000),
          model("<synthetic>", 0),
          model("gpt-5.6-sol", 500),
        ]}
        totalTokens={1500}
      />
    );

    expect(screen.queryByText("<synthetic>")).toBeNull();
    // Each real model renders its name in both the trigger and the tooltip
    // content, so assert presence rather than an exact node count.
    expect(screen.getAllByText("claude-opus-5").length).toBeGreaterThan(0);
    expect(screen.getAllByText("gpt-5.6-sol").length).toBeGreaterThan(0);
  });

  it("renders nothing when the sentinel is the only entry", () => {
    // Callers gate on the unfiltered length, so the card must not leave an
    // empty titled shell behind.
    const { container } = render(
      <ModelDistributionCard models={[model("<synthetic>", 0)]} totalTokens={0} />
    );

    expect(container.firstChild).toBeNull();
  });

  it("renders normally when no sentinel is present", () => {
    render(
      <ModelDistributionCard models={[model("claude-opus-5", 1000)]} totalTokens={1000} />
    );

    expect(screen.getAllByText("claude-opus-5").length).toBeGreaterThan(0);
  });
});
