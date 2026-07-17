const BRAIN_QUALITY_BUDGETS = Object.freeze({ low: Object.freeze({ total: 36, decisions: 6, clusters: 20, loose: 10 }),
  medium: Object.freeze({ total: 64, decisions: 10, clusters: 36, loose: 18 }), high: Object.freeze({ total: 90, decisions: 14, clusters: 50, loose: 26 }), });
function brainBudgetForQuality(tier) { return BRAIN_QUALITY_BUDGETS[tier] || BRAIN_QUALITY_BUDGETS.medium;
}
function pixelRatioForTier(tier, devicePixelRatio = 1) { const ratio = Math.max(1, Number(devicePixelRatio) || 1);
  return Math.min(ratio, tier === "low" ? 1 : tier === "medium" ? 1.5 : 2);
}
function detectBrainQualityTier({ windowObj = globalThis.window, navigatorObj = globalThis.navigator } = {}) {
  const width = Number(windowObj?.innerWidth) || 1024, dpr = Number(windowObj?.devicePixelRatio) || 1,
    cores = Number(navigatorObj?.hardwareConcurrency) || 4, memory = Number(navigatorObj?.deviceMemory) || 4,
    userAgent = String(navigatorObj?.userAgent || ""), mobileUserAgent = /Android|iPhone|iPad|iPod/i.test(userAgent), hasFinePointer =
      typeof windowObj?.matchMedia == "function"
        ? !!windowObj.matchMedia("(hover: hover) and (pointer: fine)").matches
        : !0, low = mobileUserAgent || width < 720 || !hasFinePointer || cores <= 2 || memory <= 2,
    medium = !low && (width < 1100 || cores <= 4 || memory <= 4), tier = low ? "low" : medium ? "medium" : "high";
  return { tier, nodeBudget: brainBudgetForQuality(tier), pixelRatio: pixelRatioForTier(tier, dpr),
    idleFiring: tier !== "low", hitRadiusScale: tier === "low" ? 3 : 2, };
}
export { brainBudgetForQuality, detectBrainQualityTier };
