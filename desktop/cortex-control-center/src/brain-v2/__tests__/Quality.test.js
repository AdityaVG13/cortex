import { describe, expect, it } from "vitest";

import {
  BRAIN_QUALITY_BUDGETS,
  brainBudgetForQuality,
  detectBrainQualityTier,
  pixelRatioForTier,
} from "../Quality.js";

function windowLike({ width = 1280, dpr = 1, finePointer = true } = {}) {
  return {
    innerWidth: width,
    devicePixelRatio: dpr,
    matchMedia: () => ({ matches: finePointer }),
  };
}

describe("Brain quality tiers", () => {
  it("uses the low tier for touch-sized/mobile devices and disables idle firing", () => {
    const quality = detectBrainQualityTier({
      windowObj: windowLike({ width: 390, dpr: 3, finePointer: false }),
      navigatorObj: { hardwareConcurrency: 8, deviceMemory: 8, userAgent: "iPhone" },
    });

    expect(quality.tier).toBe("low");
    expect(quality.nodeBudget).toEqual(BRAIN_QUALITY_BUDGETS.low);
    expect(quality.pixelRatio).toBe(1);
    expect(quality.idleFiring).toBe(false);
    expect(quality.hitRadiusScale).toBeGreaterThan(2);
  });

  it("keeps high desktop rendering dense while clamping very high DPR", () => {
    const quality = detectBrainQualityTier({
      windowObj: windowLike({ width: 1440, dpr: 3, finePointer: true }),
      navigatorObj: { hardwareConcurrency: 8, deviceMemory: 8, userAgent: "desktop" },
    });

    expect(quality.tier).toBe("high");
    expect(quality.nodeBudget.total).toBe(BRAIN_QUALITY_BUDGETS.high.total);
    expect(quality.pixelRatio).toBe(2);
    expect(quality.idleFiring).toBe(true);
  });

  it("falls back to medium budgets for unknown tiers and pixel ratio inputs", () => {
    expect(brainBudgetForQuality("unknown")).toEqual(BRAIN_QUALITY_BUDGETS.medium);
    expect(pixelRatioForTier("medium", 4)).toBe(1.5);
    expect(pixelRatioForTier("low", 0)).toBe(1);
  });
});
