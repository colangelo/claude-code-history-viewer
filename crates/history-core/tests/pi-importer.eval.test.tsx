/**
 * @fileoverview Acceptance eval for AC6 of the "Pi coding agent provider" spec:
 * the frontend must register "pi" as a first-class provider — the provider id
 * lookup used by the UI recognizes "pi", its display label resolves through
 * the `common.provider.pi` i18n key (present in all 5 locales), and a project
 * tagged `provider: "pi"` renders with the Pi label, not the default
 * provider's ("claude" / "Claude Code").
 *
 * This must fail against the current, unmodified frontend: "pi" is absent
 * from `PROVIDER_IDS`/`ProviderId`/`PROVIDER_TRANSLATIONS`, so
 * `getProviderId("pi")` currently falls back to the default provider
 * ("claude") and `common.provider.pi` doesn't exist in any locale file.
 */
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import {
  PROVIDER_IDS,
  getProviderId,
  getProviderLabel,
  getProviderBadgeStyle,
} from "@/utils/providers";
import type { ClaudeProject, ProviderId } from "@/types";
import { ProjectItem } from "@/components/ProjectTree/components/ProjectItem";

import en from "@/i18n/locales/en/common.json";
import ko from "@/i18n/locales/ko/common.json";
import ja from "@/i18n/locales/ja/common.json";
import zhCN from "@/i18n/locales/zh-CN/common.json";
import zhTW from "@/i18n/locales/zh-TW/common.json";

vi.mock("react-i18next", async () => {
  const actual = await vi.importActual<typeof import("react-i18next")>(
    "react-i18next"
  );
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, fallback?: string) => fallback || key,
    }),
  };
});

function createMockProject(overrides: Partial<ClaudeProject> = {}): ClaudeProject {
  return {
    name: overrides.name ?? "pi-project",
    path: overrides.path ?? "/virtual/pi-project",
    actual_path: overrides.actual_path ?? "/Users/test/pi-project",
    session_count: overrides.session_count ?? 3,
    message_count: overrides.message_count ?? 10,
    last_modified: overrides.last_modified ?? "2026-01-01T00:00:00Z",
    provider: overrides.provider,
  };
}

describe("Pi provider frontend registration (AC6)", () => {
  it("recognizes \"pi\" as a first-class provider id", () => {
    expect(PROVIDER_IDS).toContain("pi");
    expect(getProviderId("pi" as ProviderId)).toBe("pi");
  });

  it("resolves the Pi display label through the common.provider.pi i18n key", () => {
    const translate = (key: string, fallback: string) => `${key}:${fallback}`;
    const label = getProviderLabel(translate, "pi" as ProviderId);
    expect(label.startsWith("common.provider.pi:")).toBe(true);
  });

  it("has the common.provider.pi key present in all 5 locales", () => {
    for (const [locale, dict] of [
      ["en", en],
      ["ko", ko],
      ["ja", ja],
      ["zh-CN", zhCN],
      ["zh-TW", zhTW],
    ] as const) {
      expect(
        Object.prototype.hasOwnProperty.call(dict, "common.provider.pi"),
        `expected common.provider.pi to exist in locale "${locale}"`
      ).toBe(true);
    }
  });

  it("gives Pi a distinct badge style from the default provider", () => {
    expect(getProviderBadgeStyle("pi" as ProviderId)).not.toBe(
      getProviderBadgeStyle("claude")
    );
  });

  it("renders a project tagged provider:\"pi\" with the Pi label, not the default provider's", () => {
    const project = createMockProject({ provider: "pi" as ProviderId });

    render(
      <ProjectItem
        project={project}
        isExpanded={false}
        isSelected={false}
        onToggle={() => {}}
        onClick={() => {}}
        onContextMenu={() => {}}
      />
    );

    expect(screen.queryByText("Claude Code")).not.toBeInTheDocument();
    expect(screen.getByText("Pi")).toBeInTheDocument();
  });
});
