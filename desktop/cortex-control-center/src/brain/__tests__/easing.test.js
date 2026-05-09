import { describe, expect, it } from "vitest";
import { clamp01, easeOutCubic, expDecay, riseDecay } from "../easing.js";

describe("easing helpers", () => {
  it("clamp01 caps to [0,1]", () => {
    expect(clamp01(-2)).toBe(0);
    expect(clamp01(0.4)).toBe(0.4);
    expect(clamp01(2)).toBe(1);
  });

  it("easeOutCubic boundaries and monotonic", () => {
    expect(easeOutCubic(0)).toBe(0);
    expect(easeOutCubic(1)).toBe(1);
    let prev = -Infinity;
    for (let i = 0; i <= 20; i += 1) {
      const v = easeOutCubic(i / 20);
      expect(v).toBeGreaterThanOrEqual(prev);
      prev = v;
    }
  });

  it("expDecay matches exp(-t/tau) at canonical points", () => {
    const tau = 280;
    expect(Math.abs(expDecay(0, tau) - 1)).toBeLessThan(1e-6);
    expect(Math.abs(expDecay(tau * Math.LN2, tau) - 0.5)).toBeLessThan(1e-3);
    expect(Math.abs(expDecay(tau * Math.log(6), tau) - 1 / 6)).toBeLessThan(0.01);
  });

  it("riseDecay rises within riseMs then decays exp afterwards", () => {
    const value0 = riseDecay(0, 80, 280);
    const valueRise = riseDecay(80, 80, 280);
    const valueAfter = riseDecay(80 + 280 * Math.LN2, 80, 280);
    expect(value0).toBe(0);
    expect(valueRise).toBeCloseTo(1, 5);
    expect(Math.abs(valueAfter - 0.5)).toBeLessThan(1e-2);
  });
});
