import { riseDecay } from "./easing.js";

const DEPTH_CAP = 2;
const STEP_MS = 110;
const RISE_MS = 80;
const TAU_MS = 280;
const DEPTH_ATTENUATION = 0.55;
const RIPPLE_VISIBLE_TAIL_MS = 500;
const RIPPLE_LIFE_MS = DEPTH_CAP * STEP_MS + RISE_MS + RIPPLE_VISIBLE_TAIL_MS;

function endpointId(endpoint) {
  if (endpoint && typeof endpoint === "object") return endpoint.id;
  return endpoint;
}

function edgeKey(sourceId, targetId, type = "semantic") {
  return `${sourceId}>${targetId}>${type}`;
}

export class RippleEngine {
  constructor() {
    this.adjacency = new Map();
    this.edgeIndex = new Map();
    this.activationData = null;
    this.activationTexture = null;
    this.edgeCount = 0;
    this.ripples = [];
    this.observers = { fire: new Set(), reach: new Set() };
  }

  attachMesh(mesh) {
    if (!mesh?.userData) return;
    this.edgeIndex = mesh.userData.edgeIndex || new Map();
    this.activationData = mesh.userData.activationData || null;
    this.activationTexture = mesh.userData.activationTexture || null;
    this.edgeCount = mesh.userData.edgeCount || 0;
  }

  buildAdjacency(links) {
    const adjacency = new Map();
    for (const link of links || []) {
      const sourceId = endpointId(link.source);
      const targetId = endpointId(link.target);
      const type = link.type || "semantic";
      const key = edgeKey(sourceId, targetId, type);
      const edgeId = this.edgeIndex.get(key);
      if (edgeId === undefined) continue;

      if (!adjacency.has(sourceId)) adjacency.set(sourceId, []);
      if (!adjacency.has(targetId)) adjacency.set(targetId, []);
      adjacency.get(sourceId).push({ neighborId: targetId, edgeId });
      adjacency.get(targetId).push({ neighborId: sourceId, edgeId });
    }
    this.adjacency = adjacency;
  }

  bfs(sourceId) {
    const visitedNodes = new Map();
    const visitedEdges = new Map();
    const reachOrder = [];
    visitedNodes.set(sourceId, 0);

    let frontier = [sourceId];
    for (let depth = 1; depth <= DEPTH_CAP; depth += 1) {
      const next = [];
      for (const nodeId of frontier) {
        const neighbors = this.adjacency.get(nodeId) || [];
        for (const { neighborId, edgeId } of neighbors) {
          if (!visitedEdges.has(edgeId)) {
            visitedEdges.set(edgeId, depth - 1);
          }
          if (!visitedNodes.has(neighborId)) {
            visitedNodes.set(neighborId, depth);
            reachOrder.push({ neighborId, depth });
            next.push(neighborId);
          }
        }
      }
      frontier = next;
      if (!frontier.length) break;
    }

    return { visitedNodes, visitedEdges, reachOrder };
  }

  fire(sourceId, now = performance.now()) {
    if (!this.adjacency.has(sourceId)) return null;
    const result = this.bfs(sourceId);
    const ripple = {
      sourceId,
      startTime: now,
      visitedNodes: result.visitedNodes,
      visitedEdges: result.visitedEdges,
      reachOrder: result.reachOrder,
      reached: new Set(),
    };
    this.ripples.push(ripple);
    this.notify("fire", { sourceId, time: now });
    return ripple;
  }

  fireAmbient(now = performance.now()) {
    const ids = [...this.adjacency.keys()];
    if (!ids.length) return null;
    const id = ids[Math.floor(Math.random() * ids.length)];
    return this.fire(id, now);
  }

  on(event, callback) {
    this.observers[event]?.add(callback);
    return () => this.observers[event]?.delete(callback);
  }

  notify(event, payload) {
    const set = this.observers[event];
    if (!set) return;
    for (const cb of set) {
      try { cb(payload); } catch { /* observer errors must not break ticks */ }
    }
  }

  tick(now = performance.now()) {
    if (!this.activationData || !this.activationTexture) return;

    this.activationData.fill(0);
    const next = [];

    for (const ripple of this.ripples) {
      const elapsed = now - ripple.startTime;
      if (elapsed > RIPPLE_LIFE_MS) continue;

      for (const [edgeId, depth] of ripple.visitedEdges) {
        const t = elapsed - depth * STEP_MS;
        if (t < 0) continue;
        const value = riseDecay(t, RISE_MS, TAU_MS) * Math.pow(DEPTH_ATTENUATION, depth);
        const current = this.activationData[edgeId] || 0;
        this.activationData[edgeId] = Math.min(1, current + value);
      }

      for (const reach of ripple.reachOrder) {
        if (ripple.reached.has(reach.neighborId)) continue;
        const arrival = reach.depth * STEP_MS;
        if (elapsed >= arrival) {
          ripple.reached.add(reach.neighborId);
          this.notify("reach", { nodeId: reach.neighborId, depth: reach.depth });
        }
      }

      next.push(ripple);
    }

    this.ripples = next;
    this.activationTexture.needsUpdate = true;
  }

  reset() {
    this.ripples = [];
    if (this.activationData) this.activationData.fill(0);
    if (this.activationTexture) this.activationTexture.needsUpdate = true;
  }

  static get DEPTH_CAP() { return DEPTH_CAP; }
  static get STEP_MS() { return STEP_MS; }
  static get RISE_MS() { return RISE_MS; }
  static get TAU_MS() { return TAU_MS; }
  static get DEPTH_ATTENUATION() { return DEPTH_ATTENUATION; }
  static get RIPPLE_LIFE_MS() { return RIPPLE_LIFE_MS; }
}
