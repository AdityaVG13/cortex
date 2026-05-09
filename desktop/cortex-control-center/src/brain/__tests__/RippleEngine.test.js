import { describe, expect, it } from "vitest";
import { RippleEngine } from "../RippleEngine.js";

function makeMesh(edgeIndex, edgeCount) {
  return {
    userData: {
      edgeIndex,
      edgeCount,
      activationData: new Float32Array(edgeCount),
      activationTexture: { needsUpdate: false },
    },
  };
}

function buildLineGraph(n) {
  const links = [];
  const edgeIndex = new Map();
  for (let i = 0; i < n - 1; i += 1) {
    const link = { source: `n${i}`, target: `n${i + 1}`, type: "semantic" };
    edgeIndex.set(`n${i}>n${i + 1}>semantic`, i);
    links.push(link);
  }
  return { links, edgeIndex, edgeCount: n - 1 };
}

describe("RippleEngine", () => {
  it("respects depth cap of 2 hops on a long line graph", () => {
    const graph = buildLineGraph(8);
    const mesh = makeMesh(graph.edgeIndex, graph.edgeCount);
    const engine = new RippleEngine();
    engine.attachMesh(mesh);
    engine.buildAdjacency(graph.links);

    const ripple = engine.fire("n0", 0);
    const visitedDepths = [...ripple.visitedNodes.values()];
    expect(Math.max(...visitedDepths)).toBe(RippleEngine.DEPTH_CAP);
    expect(ripple.visitedNodes.has("n4")).toBe(false);
    expect(ripple.visitedNodes.get("n2")).toBe(2);
  });

  it("activations are additive across simultaneous ripples and clamp at 1.0", () => {
    const graph = buildLineGraph(4);
    const mesh = makeMesh(graph.edgeIndex, graph.edgeCount);
    const engine = new RippleEngine();
    engine.attachMesh(mesh);
    engine.buildAdjacency(graph.links);

    engine.fire("n0", 0);
    engine.fire("n2", 0);
    engine.tick(60);
    const buffer = mesh.userData.activationData;
    for (const value of buffer) {
      expect(value).toBeGreaterThanOrEqual(0);
      expect(value).toBeLessThanOrEqual(1);
    }
    expect(buffer[1]).toBeGreaterThan(0);
  });

  it("decays back to zero after RIPPLE_LIFE_MS", () => {
    const graph = buildLineGraph(3);
    const mesh = makeMesh(graph.edgeIndex, graph.edgeCount);
    const engine = new RippleEngine();
    engine.attachMesh(mesh);
    engine.buildAdjacency(graph.links);
    engine.fire("n0", 0);
    engine.tick(RippleEngine.RIPPLE_LIFE_MS + 50);
    for (const value of mesh.userData.activationData) {
      expect(value).toBe(0);
    }
    expect(engine.ripples.length).toBe(0);
  });

  it("notifies fire and reach observers", () => {
    const graph = buildLineGraph(3);
    const mesh = makeMesh(graph.edgeIndex, graph.edgeCount);
    const engine = new RippleEngine();
    engine.attachMesh(mesh);
    engine.buildAdjacency(graph.links);
    const fired = [];
    const reached = [];
    engine.on("fire", payload => fired.push(payload));
    engine.on("reach", payload => reached.push(payload));
    engine.fire("n0", 0);
    engine.tick(120);
    expect(fired.length).toBe(1);
    expect(fired[0].sourceId).toBe("n0");
    expect(reached.some(r => r.nodeId === "n1" && r.depth === 1)).toBe(true);
  });

  it("fireAmbient picks a random node and produces a ripple", () => {
    const graph = buildLineGraph(5);
    const mesh = makeMesh(graph.edgeIndex, graph.edgeCount);
    const engine = new RippleEngine();
    engine.attachMesh(mesh);
    engine.buildAdjacency(graph.links);
    const ripple = engine.fireAmbient(0);
    expect(ripple).not.toBeNull();
    expect(ripple.visitedNodes.size).toBeGreaterThan(0);
  });
});
