/**
 * Window → `from=` mapping for the Analytics range control
 * (analytics-ux-costs 5.3).
 */

import { describe, expect, it } from "vitest";
import {
  RANGE_PRESET_DAYS,
  isoDaysAgo,
  parseCustomDays,
} from "../components/ArchiveBrowser/analyticsRange";

describe("analyticsRange", () => {
  it("maps a day count to an ISO date that many days back", () => {
    const now = new Date("2026-07-26T12:00:00Z");
    expect(isoDaysAgo(7, now)).toBe("2026-07-19");
    expect(isoDaysAgo(45, now)).toBe("2026-06-11");
    expect(isoDaysAgo(365, now)).toBe("2025-07-26");
  });

  it("crosses month and year boundaries", () => {
    expect(isoDaysAgo(30, new Date("2026-01-15T00:00:00Z"))).toBe("2025-12-16");
  });

  it("offers the spec'd presets", () => {
    expect([...RANGE_PRESET_DAYS]).toEqual([7, 14, 30, 60, 90, 180, 365]);
  });

  it("accepts only positive integers as custom day counts", () => {
    expect(parseCustomDays("45")).toBe(45);
    expect(parseCustomDays(" 7 ")).toBe(7);
    expect(parseCustomDays("0")).toBeNull();
    expect(parseCustomDays("-3")).toBeNull();
    expect(parseCustomDays("3.5")).toBeNull();
    expect(parseCustomDays("abc")).toBeNull();
    expect(parseCustomDays("")).toBeNull();
  });
});
