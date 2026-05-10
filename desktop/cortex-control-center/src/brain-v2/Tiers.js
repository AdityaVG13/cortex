import { fnv1a32 } from "./util/fnv1a.js";

const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5));

const TIER_DECISION_RADIUS = 80;
const TIER_CLUSTER_RADIUS = 140;
const TIER_LOOSE_RADIUS_MIN = 180;
const TIER_LOOSE_RADIUS_MAX = 220;

const TOTAL_BUDGET_MIN = 70;
const TOTAL_BUDGET_MAX = 90;
const DECISION_RATIO = 0.15;
const CLUSTER_RATIO = 0.55;
const LOOSE_RATIO = 0.30;

function pickBudget() {
  const span = TOTAL_BUDGET_MAX - TOTAL_BUDGET_MIN + 1;
  const total = TOTAL_BUDGET_MIN + Math.floor(Math.random() * span);
  return {
    total,
    decisions: Math.max(3, Math.round(total * DECISION_RATIO)),
    clusters: Math.max(10, Math.round(total * CLUSTER_RATIO)),
    loose: Math.max(5, Math.round(total * LOOSE_RATIO)),
  };
}

function fibonacciOnSphere(index, total, seedOffset = 0) {
  const i = index + 0.5;
  const phi = Math.acos(1 - (2 * i) / Math.max(total, 1));
  const theta = GOLDEN_ANGLE * i + seedOffset;
  return {
    nx: Math.sin(phi) * Math.cos(theta),
    ny: Math.sin(phi) * Math.sin(theta),
    nz: Math.cos(phi),
  };
}

function clusterRadius(memberCount) {
  if (!memberCount || memberCount < 1) return 1.4;
  return Math.min(4.0, Math.max(1.4, Math.log2(memberCount + 1) * 1.4));
}

function looseRadius(seed) {
  const f = ((seed >>> 0) % 1024) / 1024;
  return TIER_LOOSE_RADIUS_MIN + (TIER_LOOSE_RADIUS_MAX - TIER_LOOSE_RADIUS_MIN) * f;
}

export function buildTiers(dump, options = {}) {
  const budget = options.budget || pickBudget();
  const decisions = (dump?.decisions || []).slice(0, budget.decisions);
  const rawClusters = (dump?.clusters || dump?.crystals || []).slice(0, budget.clusters);
  const memories = (dump?.memories || []).slice();
  memories.sort((a, b) => (b?.score || 0) - (a?.score || 0));
  // Backfill: compute how many memories we actually need to hit the budget.
  const desiredTotal = budget.decisions + budget.clusters + budget.loose;
  const usedSoFar = decisions.length + rawClusters.length;
  const looseTargetEarly = Math.max(budget.loose, desiredTotal - usedSoFar);
  const looseMemories = memories.slice(0, looseTargetEarly);

  const decisionsLayout = decisions.map((node, index) => {
    const id = `decision-${node.id}`;
    const seed = fnv1a32(id);
    const { nx, ny, nz } = fibonacciOnSphere(index, decisions.length, (seed % 1024) / 1024);
    return {
      id,
      sourceId: node.id,
      tier: "decision",
      label: node.decision || `Decision ${node.id}`,
      agent: node.source_agent || "system",
      type: "decision",
      bodyRadius: 2.0,
      x: nx * TIER_DECISION_RADIUS,
      y: ny * TIER_DECISION_RADIUS,
      z: nz * TIER_DECISION_RADIUS,
      orbitRadius: TIER_DECISION_RADIUS,
      memberCount: 1,
    };
  });

  const clusterCount = rawClusters.length;
  const useColdStart = clusterCount === 0 && looseMemories.length > 0;
  const clusterSourceCount = useColdStart
    ? Math.min(budget.clusters, looseMemories.length)
    : clusterCount;

  const clustersLayout = useColdStart
    ? looseMemories.slice(0, clusterSourceCount).map((mem, index) => {
        const id = `cold-cluster-${mem.id}`;
        const seed = fnv1a32(id);
        const { nx, ny, nz } = fibonacciOnSphere(index, clusterSourceCount, (seed % 1024) / 1024);
        return {
          id,
          sourceId: mem.id,
          tier: "cluster",
          coldStart: true,
          label: mem.source || mem.text || `Memory ${mem.id}`,
          agent: mem.source_agent || "system",
          type: "memory",
          bodyRadius: 1.4,
          x: nx * TIER_CLUSTER_RADIUS,
          y: ny * TIER_CLUSTER_RADIUS,
          z: nz * TIER_CLUSTER_RADIUS,
          orbitRadius: TIER_CLUSTER_RADIUS,
          memberCount: 1,
          centroidKey: `cold-${mem.id}`,
        };
      })
    : rawClusters.map((cluster, index) => {
        const id = `cluster-${cluster.id}`;
        const seed = fnv1a32(id);
        const { nx, ny, nz } = fibonacciOnSphere(index, clusterCount, (seed % 1024) / 1024);
        const memberCount = Number(cluster.member_count || 1);
        const bodyRadius = clusterRadius(memberCount);
        return {
          id,
          sourceId: cluster.id,
          tier: "cluster",
          coldStart: false,
          label: cluster.label || `Cluster ${cluster.id}`,
          agent: "consolidation",
          type: "cluster",
          bodyRadius,
          x: nx * TIER_CLUSTER_RADIUS,
          y: ny * TIER_CLUSTER_RADIUS,
          z: nz * TIER_CLUSTER_RADIUS,
          orbitRadius: TIER_CLUSTER_RADIUS,
          memberCount,
          centroidKey: cluster.centroid_key || `centroid-${cluster.id}`,
        };
      });

  const usedDecisions = decisionsLayout.length;
  const usedClusters = clustersLayout.length;
  const looseTarget = Math.max(budget.loose, desiredTotal - usedDecisions - usedClusters);

  const loosePool = useColdStart ? looseMemories.slice(clusterSourceCount) : looseMemories;
  const looseLayout = loosePool.slice(0, looseTarget).map((mem, index) => {
    const id = `loose-${mem.id}`;
    const seed = fnv1a32(id);
    const { nx, ny, nz } = fibonacciOnSphere(index, loosePool.length, (seed % 1024) / 1024);
    const r = looseRadius(seed);
    return {
      id,
      sourceId: mem.id,
      tier: "loose",
      label: mem.source || mem.text || `Memory ${mem.id}`,
      agent: mem.source_agent || "system",
      type: "memory",
      bodyRadius: 1.0,
      x: nx * r,
      y: ny * r,
      z: nz * r,
      orbitRadius: r,
      memberCount: 1,
    };
  });

  return {
    decisions: decisionsLayout,
    clusters: clustersLayout,
    looseMemories: looseLayout,
    coldStart: useColdStart,
    budget,
  };
}

export { pickBudget };

export const TIER_RADII = Object.freeze({
  decision: TIER_DECISION_RADIUS,
  cluster: TIER_CLUSTER_RADIUS,
  looseMin: TIER_LOOSE_RADIUS_MIN,
  looseMax: TIER_LOOSE_RADIUS_MAX,
});
