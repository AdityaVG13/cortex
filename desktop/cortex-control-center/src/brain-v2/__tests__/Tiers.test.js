import { describe, expect, it } from "vitest";
import { buildTiers, TIER_RADII } from "../Tiers.js";

function makeDump({ memories = [], decisions = [], clusters = [] } = {}) {
  return { memories, decisions, clusters };
}

describe("buildTiers", () => {
  it("places decisions on R=80, clusters on R=140, loose on R=180-220", () => {
    const dump = makeDump({
      decisions: [
        { id: 1, decision: "d1", source_agent: "a" },
        { id: 2, decision: "d2", source_agent: "a" },
      ],
      clusters: [
        { id: 10, label: "c10", member_count: 3 },
        { id: 11, label: "c11", member_count: 5 },
      ],
      memories: Array.from({ length: 6 }, (_, i) => ({ id: i + 1, score: i, source: `m${i}` })),
    });

    const tiers = buildTiers(dump);
    expect(tiers.coldStart).toBe(false);

    for (const d of tiers.decisions) {
      const r = Math.hypot(d.x, d.y, d.z);
      expect(Math.abs(r - TIER_RADII.decision)).toBeLessThan(0.01);
    }
    for (const c of tiers.clusters) {
      const r = Math.hypot(c.x, c.y, c.z);
      expect(Math.abs(r - TIER_RADII.cluster)).toBeLessThan(0.01);
    }
    for (const m of tiers.looseMemories) {
      const r = Math.hypot(m.x, m.y, m.z);
      expect(r).toBeGreaterThanOrEqual(TIER_RADII.looseMin - 0.01);
      expect(r).toBeLessThanOrEqual(TIER_RADII.looseMax + 0.01);
    }
  });

  it("is deterministic: same dump → same positions across calls", () => {
    const dump = makeDump({
      clusters: Array.from({ length: 12 }, (_, i) => ({ id: 100 + i, label: `c${i}`, member_count: i + 1 })),
      memories: Array.from({ length: 30 }, (_, i) => ({ id: i + 1, score: i })),
    });
    const a = buildTiers(dump);
    const b = buildTiers(dump);
    expect(a.clusters.map(c => c.id)).toEqual(b.clusters.map(c => c.id));
    for (let i = 0; i < a.clusters.length; i += 1) {
      expect(a.clusters[i].x).toBeCloseTo(b.clusters[i].x, 6);
      expect(a.clusters[i].y).toBeCloseTo(b.clusters[i].y, 6);
      expect(a.clusters[i].z).toBeCloseTo(b.clusters[i].z, 6);
    }
  });

  it("falls back to cold-start cluster mode when no clusters exist", () => {
    const dump = makeDump({
      memories: Array.from({ length: 100 }, (_, i) => ({ id: i + 1, score: 100 - i })),
    });
    const tiers = buildTiers(dump);
    expect(tiers.coldStart).toBe(true);
    expect(tiers.clusters.length).toBeGreaterThan(0);
    for (const c of tiers.clusters) {
      expect(c.coldStart).toBe(true);
      expect(c.tier).toBe("cluster");
    }
  });

  it("caps decisions at 20, clusters at 80, loose at 50", () => {
    const dump = makeDump({
      decisions: Array.from({ length: 100 }, (_, i) => ({ id: i + 1 })),
      clusters: Array.from({ length: 200 }, (_, i) => ({ id: i + 1, label: `c${i}`, member_count: 4 })),
      memories: Array.from({ length: 200 }, (_, i) => ({ id: i + 1, score: i })),
    });
    const tiers = buildTiers(dump);
    expect(tiers.decisions.length).toBe(20);
    expect(tiers.clusters.length).toBe(80);
    expect(tiers.looseMemories.length).toBe(50);
  });

  it("cluster body radius scales with member_count", () => {
    const dump = makeDump({
      clusters: [
        { id: 1, label: "small", member_count: 1 },
        { id: 2, label: "mid", member_count: 7 },
        { id: 3, label: "large", member_count: 256 },
      ],
    });
    const tiers = buildTiers(dump);
    const sizes = tiers.clusters.map(c => c.bodyRadius);
    expect(sizes[0]).toBeCloseTo(1.4, 4);
    expect(sizes[1]).toBeGreaterThan(sizes[0]);
    expect(sizes[2]).toBeCloseTo(4.0, 4);
  });
});
