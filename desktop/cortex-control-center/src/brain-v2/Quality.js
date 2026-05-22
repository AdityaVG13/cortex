export const BRAIN_QUALITY_BUDGETS = Object.freeze({
  low: Object.freeze({ total: 36, decisions: 6, clusters: 20, loose: 10 }),
  medium: Object.freeze({ total: 64, decisions: 10, clusters: 36, loose: 18 }),
  high: Object.freeze({ total: 90, decisions: 14, clusters: 50, loose: 26 }),
});

export function brainBudgetForQuality(tier) {
  return BRAIN_QUALITY_BUDGETS[tier] || BRAIN_QUALITY_BUDGETS.medium;
}

export function pixelRatioForTier(tier, devicePixelRatio = 1) {
  const ratio = Math.max(1, Number(devicePixelRatio) || 1);
  const max = tier === "low" ? 1 : tier === "medium" ? 1.5 : 2;
  return Math.min(ratio, max);
}

export function detectBrainQualityTier({
  windowObj = globalThis.window,
  navigatorObj = globalThis.navigator,
} = {}) {
  const width = Number(windowObj?.innerWidth) || 1024;
  const dpr = Number(windowObj?.devicePixelRatio) || 1;
  const cores = Number(navigatorObj?.hardwareConcurrency) || 4;
  const memory = Number(navigatorObj?.deviceMemory) || 4;
  const userAgent = String(navigatorObj?.userAgent || "");
  const mobileUserAgent = /Android|iPhone|iPad|iPod/i.test(userAgent);
  const hasFinePointer = typeof windowObj?.matchMedia === "function"
    ? Boolean(windowObj.matchMedia("(hover: hover) and (pointer: fine)").matches)
    : true;

  const low = mobileUserAgent || width < 720 || !hasFinePointer || cores <= 2 || memory <= 2;
  const medium = !low && (width < 1100 || cores <= 4 || memory <= 4);
  const tier = low ? "low" : medium ? "medium" : "high";

  return {
    tier,
    nodeBudget: brainBudgetForQuality(tier),
    pixelRatio: pixelRatioForTier(tier, dpr),
    idleFiring: tier !== "low",
    hitRadiusScale: tier === "low" ? 3.0 : 2.0,
  };
}
