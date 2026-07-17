import { fnv1a32 } from "./util/fnv1a.js";
const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5)), LABEL_MAX_LEN = 56;
function truncate(text, max = LABEL_MAX_LEN) { if (!text) return "";
  const flat = String(text).replace(/\s+/g, " ").trim();
  return flat.length <= max ? flat : `${flat.slice(0, max - 1)}\u2026`;
}
function memoryLabel(mem) { const text = mem?.text, source = mem?.source;
  return text && text.trim().length > 0
    ? truncate(text)
    : source && source.trim().length > 0
      ? truncate(source)
      : `Memory ${mem?.id ?? "?"}`;
}
function clusterLabelOf(cluster) { const label = cluster?.label, consolidated = cluster?.consolidated_text;
  return label && label.trim().length > 0
    ? truncate(label)
    : consolidated && consolidated.trim().length > 0
      ? truncate(consolidated)
      : `Cluster ${cluster?.id ?? "?"}`;
}
function decisionLabelOf(dec) { const decision = dec?.decision;
  return decision && decision.trim().length > 0 ? truncate(decision) : `Decision ${dec?.id ?? "?"}`;
}
const TIER_DECISION_RADIUS = 80, TIER_CLUSTER_RADIUS = 140, TIER_LOOSE_RADIUS_MIN = 180, TIER_LOOSE_RADIUS_MAX = 220,
  TOTAL_BUDGET_MIN = 70, TOTAL_BUDGET_MAX = 90, DECISION_RATIO = 0.15, CLUSTER_RATIO = 0.55, LOOSE_RATIO = 0.3;
function pickBudget() { const span = TOTAL_BUDGET_MAX - TOTAL_BUDGET_MIN + 1,
    total = TOTAL_BUDGET_MIN + Math.floor(Math.random() * span), decisions = Math.max(3, Math.round(total * DECISION_RATIO)),
    clusters = Math.max(10, Math.round(total * CLUSTER_RATIO)), looseTarget = Math.max(5, Math.round(total * LOOSE_RATIO)),
    roundedTotal = decisions + clusters + looseTarget, loose = Math.max(5, looseTarget + total - roundedTotal);
  return { total, decisions, clusters, loose };
}
function fibonacciOnSphere(index, total, seedOffset = 0) { const i = index + 0.5,
    phi = Math.acos(1 - (2 * i) / Math.max(total, 1)), theta = GOLDEN_ANGLE * i + seedOffset;
  return { nx: Math.sin(phi) * Math.cos(theta), ny: Math.sin(phi) * Math.sin(theta), nz: Math.cos(phi), };
}
function clusterRadius(memberCount) { return !memberCount || memberCount < 1 ? 1.4 : Math.min(4, Math.max(1.4, Math.log2(memberCount + 1) * 1.4));
}
function looseRadius(seed) { const f = ((seed >>> 0) % 1024) / 1024;
  return TIER_LOOSE_RADIUS_MIN + (TIER_LOOSE_RADIUS_MAX - TIER_LOOSE_RADIUS_MIN) * f;
}
function buildTiers(dump, options = {}) { const budget = options.budget || pickBudget(),
    decisions = (dump?.decisions || []).slice(0, budget.decisions), rawClusters = (dump?.clusters || dump?.crystals || []).slice(0, budget.clusters),
    memories = (dump?.memories || []).slice();
  memories.sort((a, b) => (b?.score || 0) - (a?.score || 0));
  const desiredTotal = budget.decisions + budget.clusters + budget.loose, usedSoFar = decisions.length + rawClusters.length,
    looseTargetEarly = Math.max(budget.loose, desiredTotal - usedSoFar), looseMemories = memories.slice(0, looseTargetEarly),
    decisionsLayout = decisions.map((node, index) => { const id = `decision-${node.id}`,
        seed = fnv1a32(id), { nx, ny, nz } = fibonacciOnSphere(index, decisions.length, (seed % 1024) / 1024);
      return { id, sourceId: node.id, tier: "decision", label: decisionLabelOf(node), agent: node.source_agent || "system", type: "decision", bodyRadius: 2,
        x: nx * TIER_DECISION_RADIUS, y: ny * TIER_DECISION_RADIUS, z: nz * TIER_DECISION_RADIUS, orbitRadius: TIER_DECISION_RADIUS, memberCount: 1, };
    }), clusterCount = rawClusters.length, useColdStart = clusterCount === 0 && looseMemories.length > 0,
    clusterSourceCount = useColdStart ? Math.min(budget.clusters, looseMemories.length) : clusterCount, clustersLayout = useColdStart
      ? looseMemories.slice(0, clusterSourceCount).map((mem, index) => { const id = `cold-cluster-${mem.id}`,
            seed = fnv1a32(id), { nx, ny, nz } = fibonacciOnSphere(index, clusterSourceCount, (seed % 1024) / 1024);
          return { id, sourceId: mem.id, tier: "cluster", coldStart: !0, label: memoryLabel(mem), agent: mem.source_agent || "system", type: "memory",
            bodyRadius: 1.4, x: nx * TIER_CLUSTER_RADIUS, y: ny * TIER_CLUSTER_RADIUS, z: nz * TIER_CLUSTER_RADIUS,
            orbitRadius: TIER_CLUSTER_RADIUS, memberCount: 1, centroidKey: `cold-${mem.id}`, };
        })
      : rawClusters.map((cluster, index) => { const id = `cluster-${cluster.id}`,
            seed = fnv1a32(id), { nx, ny, nz } = fibonacciOnSphere(index, clusterCount, (seed % 1024) / 1024),
            memberCount = Number(cluster.member_count || 1), bodyRadius = clusterRadius(memberCount);
          return { id, sourceId: cluster.id, tier: "cluster", coldStart: !1, label: clusterLabelOf(cluster), agent: "consolidation", type: "cluster",
            bodyRadius, x: nx * TIER_CLUSTER_RADIUS, y: ny * TIER_CLUSTER_RADIUS, z: nz * TIER_CLUSTER_RADIUS,
            orbitRadius: TIER_CLUSTER_RADIUS, memberCount, centroidKey: cluster.centroid_key || `centroid-${cluster.id}`, };
        }), usedDecisions = decisionsLayout.length,
    usedClusters = clustersLayout.length, looseTarget = Math.max(budget.loose, desiredTotal - usedDecisions - usedClusters),
    loosePool = useColdStart ? looseMemories.slice(clusterSourceCount) : looseMemories, looseLayout = loosePool.slice(0, looseTarget).map((mem, index) => {
      const id = `loose-${mem.id}`, seed = fnv1a32(id),
        { nx, ny, nz } = fibonacciOnSphere(index, loosePool.length, (seed % 1024) / 1024), r = looseRadius(seed);
      return { id, sourceId: mem.id, tier: "loose", label: memoryLabel(mem), agent: mem.source_agent || "system", type: "memory", bodyRadius: 1,
        x: nx * r, y: ny * r, z: nz * r, orbitRadius: r, memberCount: 1, };
    });
  return { decisions: decisionsLayout, clusters: clustersLayout, looseMemories: looseLayout, coldStart: useColdStart, budget, };
}
const TIER_RADII = Object.freeze({ decision: TIER_DECISION_RADIUS, cluster: TIER_CLUSTER_RADIUS, looseMin: TIER_LOOSE_RADIUS_MIN,
  looseMax: TIER_LOOSE_RADIUS_MAX, });
export { buildTiers };
