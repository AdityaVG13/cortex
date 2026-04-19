import { describe, expect, it } from "vitest";

import { buildMonteCarloProjection } from "./analytics-projection.js";

describe("buildMonteCarloProjection", () => {
  it("returns a deterministic projection with two daily points", () => {
    const daily = [
      { date: "2026-04-18", saved: 771954 },
      { date: "2026-04-19", saved: 237271050 },
    ];
    const cumulative = [
      { date: "2026-04-18", savedTotal: 788763 },
      { date: "2026-04-19", savedTotal: 238060137 },
    ];

    const first = buildMonteCarloProjection(daily, cumulative);
    const second = buildMonteCarloProjection(daily, cumulative);

    expect(first).not.toBeNull();
    expect(second).not.toBeNull();
    expect(first.horizonDays).toBe(30);
    expect(first.simulationCount).toBe(180);
    expect(first.summary.avgDaily).toBeGreaterThan(0);
    expect(first.summary.p50Gain).toBeGreaterThan(0);
    expect(first.summary.p50Gain).toBeLessThan(1_000_000_000_000_000);
    expect(first.summary.p50Gain).toBe(second.summary.p50Gain);
    expect(first.bandSeries).toHaveLength(30);
  });

  it("falls back to cumulative saved deltas when daily series is empty", () => {
    const cumulative = [
      { date: "2026-04-16", savedDelta: 12, savedTotal: 12 },
      { date: "2026-04-17", savedDelta: 21, savedTotal: 33 },
      { date: "2026-04-18", savedDelta: 34, savedTotal: 67 },
    ];

    const projection = buildMonteCarloProjection([], cumulative, 14, 40);

    expect(projection).not.toBeNull();
    expect(projection.horizonDays).toBe(14);
    expect(projection.simulationCount).toBe(40);
    expect(projection.summary.startTotal).toBe(67);
    expect(projection.samples.length).toBeGreaterThan(0);
  });

  it("suppresses explosive outliers and keeps gains finite", () => {
    const daily = [
      { date: "2026-04-16", saved: 120_000_000 },
      { date: "2026-04-17", saved: 121_000_000 },
      { date: "2026-04-18", saved: 4.6628816004992054e76 },
      { date: "2026-04-19", saved: 119_000_000 },
    ];
    const cumulative = [
      { date: "2026-04-16", savedTotal: 120_000_000 },
      { date: "2026-04-17", savedTotal: 241_000_000 },
      { date: "2026-04-18", savedTotal: 361_000_000 },
      { date: "2026-04-19", savedTotal: 480_000_000 },
    ];

    const projection = buildMonteCarloProjection(daily, cumulative, 30, 180);
    expect(projection).not.toBeNull();
    expect(Number.isFinite(projection.summary.p50Gain)).toBe(true);
    expect(Number.isFinite(projection.summary.p90Gain)).toBe(true);
    expect(projection.summary.p50Gain).toBeLessThan(1_000_000_000_000_000);
    expect(projection.summary.p90Gain).toBeLessThan(2_000_000_000_000_000);
  });

  it("returns null when there is not enough history", () => {
    const projection = buildMonteCarloProjection(
      [{ date: "2026-04-19", saved: 50 }],
      [{ date: "2026-04-19", savedTotal: 50 }]
    );
    expect(projection).toBeNull();
  });
});
