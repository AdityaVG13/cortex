import { SHELL_OUTER_RADIUS, SHELL_INNER_RADIUS } from "./ShellGeometry.js";

const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5));

function hashString(value) {
  let hash = 2166136261;
  const text = String(value || "");
  for (let index = 0; index < text.length; index += 1) {
    hash ^= text.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

function seededUnit(seed, salt = 0) {
  const value = Math.sin((seed + (salt * 1013)) * 12.9898) * 43758.5453;
  return value - Math.floor(value);
}

function isDecisionNode(node) {
  return node?.group === "decision" || node?.type === "decision";
}

function fibonacciSphere(index, total, seed) {
  const i = index + 0.5;
  const phi = Math.acos(1 - (2 * i) / total);
  const theta = GOLDEN_ANGLE * i + seededUnit(seed, 1) * 2 * Math.PI;
  return {
    nx: Math.sin(phi) * Math.cos(theta),
    ny: Math.sin(phi) * Math.sin(theta),
    nz: Math.cos(phi),
  };
}

function shellPoint(node, indexInGroup, totalInGroup, radius, jitter = 0.06) {
  const seed = hashString(`${node.id}:${node.agent}:${node.type}`);
  const { nx, ny, nz } = fibonacciSphere(indexInGroup, Math.max(totalInGroup, 1), seed);
  const wobble = 1 + (seededUnit(seed, 2) - 0.5) * jitter;
  const r = radius * wobble;
  const x = nx * r;
  const y = ny * r;
  const z = nz * r;
  return {
    brainRegion: radius === SHELL_OUTER_RADIUS ? "outer" : "inner",
    brainHemisphere: nx >= 0 ? "right" : "left",
    brainX: x,
    brainY: y,
    brainZ: z,
    shellRadius: radius,
    x,
    y,
    z,
  };
}

export function applyShellLayout(nodes, options = {}) {
  const { useShellSplit = true } = options;
  const memoryNodes = [];
  const decisionNodes = [];
  for (const node of nodes) {
    if (isDecisionNode(node)) decisionNodes.push(node);
    else memoryNodes.push(node);
  }

  const outerGroup = useShellSplit ? memoryNodes : nodes;
  const innerGroup = useShellSplit ? decisionNodes : [];

  const positioned = new Map();

  outerGroup.forEach((node, index) => {
    positioned.set(node.id, shellPoint(node, index, outerGroup.length, SHELL_OUTER_RADIUS));
  });

  innerGroup.forEach((node, index) => {
    positioned.set(node.id, shellPoint(node, index, innerGroup.length, SHELL_INNER_RADIUS));
  });

  return nodes.map(node => ({ ...node, ...positioned.get(node.id) }));
}

export function createShellProjectionForce(strength = 0.42) {
  let nodes = [];
  function force(alpha) {
    const pull = Math.min(0.24, strength * alpha);
    for (const node of nodes) {
      if (!Number.isFinite(node.brainX) || !Number.isFinite(node.brainY) || !Number.isFinite(node.brainZ)) continue;
      node.vx += (node.brainX - (node.x || 0)) * pull;
      node.vy += (node.brainY - (node.y || 0)) * pull;
      node.vz += (node.brainZ - (node.z || 0)) * pull;
    }
  }
  force.initialize = assignedNodes => { nodes = assignedNodes || []; };
  return force;
}
