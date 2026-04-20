import { describe, expect, it } from "vitest";
import { summarizeBootThroughput } from "./analytics-metrics.js";

describe("summarizeBootThroughput", () => {
  it("counts boots across the last 7 calendar days with zero-fill", () => {
    const result = summarizeBootThroughput(
      [
        { date: "2026-04-14", boots: 1 },
        { date: "2026-04-20", boots: 6 },
      ],
      7,
      new Date("2026-04-20T12:00:00Z")
    );

    expect(result.boots).toBe(7);
    expect(result.windowStart).toBe("2026-04-14");
    expect(result.windowEnd).toBe("2026-04-20");
    expect(result.daysRepresented).toBe(7);
    expect(result.avgPerDay).toBe(1);
    expect(result.isPartialHistory).toBe(false);
  });

  it("uses partial-history averaging when first boot is inside the trailing window", () => {
    const result = summarizeBootThroughput(
      [
        { date: "2026-04-20", boots: 3 },
        { date: "2026-04-18", boots: 2 },
        { date: "2026-04-16", boots: 5 },
      ],
      7,
      new Date("2026-04-20T12:00:00Z")
    );

    expect(result.boots).toBe(10);
    expect(result.daysRepresented).toBe(5);
    expect(result.avgPerDay).toBe(2);
    expect(result.isPartialHistory).toBe(true);
  });

  it("sums duplicate rows per day and ignores invalid rows", () => {
    const result = summarizeBootThroughput(
      [
        { date: "2026-04-20", boots: 2 },
        { date: "2026-04-20", boots: 3 },
        { date: "2026-04-19", boots: "NaN" },
        { date: "not-a-day", boots: 10 },
      ],
      7,
      new Date("2026-04-20T12:00:00Z")
    );

    expect(result.boots).toBe(5);
    expect(result.daysRepresented).toBe(1);
    expect(result.avgPerDay).toBe(5);
  });

  it("returns zeroed metrics when there is no boot history", () => {
    const result = summarizeBootThroughput([], 7, new Date("2026-04-20T12:00:00Z"));

    expect(result.boots).toBe(0);
    expect(result.daysRepresented).toBe(0);
    expect(result.avgPerDay).toBe(0);
    expect(result.isPartialHistory).toBe(false);
  });
});
