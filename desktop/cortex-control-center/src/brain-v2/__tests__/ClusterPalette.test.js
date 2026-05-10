import { describe, expect, it } from "vitest";
import { paletteForCluster, paletteForId, clearPaletteCache } from "../ClusterPalette.js";

describe("ClusterPalette", () => {
  it("paletteForCluster is reproducible for identical input", () => {
    const a = paletteForCluster("centroid-fixture-001");
    const b = paletteForCluster("centroid-fixture-001");
    expect(a.seed).toBe(b.seed);
    expect(a.hue).toBe(b.hue);
    expect(a.color.getHex()).toBe(b.color.getHex());
  });

  it("paletteForId is stable across calls", () => {
    clearPaletteCache();
    const a = paletteForId(42);
    clearPaletteCache();
    const b = paletteForId(42);
    expect(a.seed).toBe(b.seed);
  });

  it("hue is in [0, 360)", () => {
    for (let i = 0; i < 50; i += 1) {
      const p = paletteForId(i);
      expect(p.hue).toBeGreaterThanOrEqual(0);
      expect(p.hue).toBeLessThan(360);
    }
  });

  it("uses fixed saturation 0.70 and lightness 0.58", () => {
    const p = paletteForId(7);
    expect(p.saturation).toBeCloseTo(0.70, 5);
    expect(p.lightness).toBeCloseTo(0.58, 5);
  });
});
