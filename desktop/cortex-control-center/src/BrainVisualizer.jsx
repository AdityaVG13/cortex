import { Component, memo, startTransition, useCallback, useEffect, useMemo, useRef, useState } from "react";
import ForceGraph3D from "react-force-graph-3d";
import * as THREE from "three";
import { getAgentColor, truncate } from "./constants.js";
import { AppIcon } from "./ui-icons.jsx";

const BRAIN_NODE_COLORS = Object.freeze({
  memory: "#22d3ee",
  decision: "#f6b34b",
  selected: "#f8fbff",
  neighborMemory: "#67e8f9",
  neighborDecision: "#ffd166",
  dimmed: "#1b3348",
});

const BRAIN_LEGEND = Object.freeze([
  { key: "memory", label: "Memory", color: BRAIN_NODE_COLORS.memory },
  { key: "decision", label: "Decision", color: BRAIN_NODE_COLORS.decision },
  { key: "flow", label: "Recall flow", color: BRAIN_NODE_COLORS.neighborMemory },
  { key: "selected", label: "Selected", color: BRAIN_NODE_COLORS.selected },
]);

const BRAIN_FOCUS_DISTANCE = 136;
const BRAIN_FOCUS_TRANSITION_MS = 1550;
const BRAIN_OVERVIEW_LINK_CAP = 96;
const BRAIN_SHAPE_FORCE = 0.42;
const BRAIN_JARVIS_SHELL_NAME = "cortex-jarvis-brain-shell";

const BRAIN_REGIONS = Object.freeze([
  { key: "frontal", x: 0.88, y: 0.34, z: 0.12 },
  { key: "parietal", x: 0.55, y: 0.74, z: -0.18 },
  { key: "temporal", x: 0.90, y: -0.30, z: 0.22 },
  { key: "occipital", x: 0.36, y: -0.10, z: -0.34 },
  { key: "limbic", x: 0.30, y: 0.06, z: 0.48 },
]);

// Error boundary to catch Three.js/WebGL crashes
class GraphErrorBoundary extends Component {
  constructor(props) {
    super(props);
    this.state = { hasError: false, error: null };
  }
  static getDerivedStateFromError(error) {
    return { hasError: true, error: error.message };
  }
  render() {
    if (this.state.hasError) {
      return this.props.fallback || (
        <div className="brain-loading">
          <div className="coming-icon"><AppIcon name="brain" size={48} /></div>
          <p>3D renderer crashed: {this.state.error}</p>
          <p className="brain-fallback-reason">Showing 2D fallback instead.</p>
        </div>
      );
    }
    return this.props.children;
  }
}

function hasWebGLSupport() {
  if (typeof document === "undefined") return false;
  try {
    const canvas = document.createElement("canvas");
    return Boolean(
      canvas.getContext("webgl2")
        || canvas.getContext("webgl")
        || canvas.getContext("experimental-webgl")
    );
  } catch {
    return false;
  }
}

function graphEndpointId(endpoint) {
  if (endpoint && typeof endpoint === "object") return endpoint.id;
  return endpoint;
}

function graphLinkKey(link) {
  return `${graphEndpointId(link.source)}>${graphEndpointId(link.target)}>${link.type || "semantic"}`;
}

function graphNodePosition(node) {
  return {
    x: Number.isFinite(node?.x) ? node.x : 0,
    y: Number.isFinite(node?.y) ? node.y : 0,
    z: Number.isFinite(node?.z) ? node.z : 0,
  };
}

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

function brainOverviewThreshold(nodeCount) {
  return Math.max(480, Math.min(1080, Math.sqrt(Math.max(nodeCount, 1)) * 72));
}

function brainLinkOverviewScore(link) {
  const type = String(link.type || "semantic");
  const weight = Number.isFinite(Number(link.weight)) ? Number(link.weight) : 1;
  const sourceId = String(graphEndpointId(link.source) || "");
  const targetId = String(graphEndpointId(link.target) || "");
  const typeScore = type === "conflict"
    ? 8
    : type === "persisted"
      ? 5
      : type.includes("decision")
        ? 4
        : type.includes("semantic")
          ? 1
          : 3;
  const decisionEndpointScore = (sourceId.startsWith("dec-") ? 0.8 : 0) + (targetId.startsWith("dec-") ? 0.8 : 0);
  return typeScore + (weight * 2) + decisionEndpointScore;
}

function formatFlowType(type) {
  return String(type || "semantic").replace(/[_-]+/g, " ");
}

function isDecisionNode(node) {
  return node?.group === "decision" || node?.type === "decision";
}

function brainRegionForNode(node, seed) {
  if (isDecisionNode(node)) return seed % 2 === 0 ? BRAIN_REGIONS[0] : BRAIN_REGIONS[4];
  const type = String(node?.type || "").toLowerCase();
  if (type.includes("episodic") || type.includes("conversation")) return BRAIN_REGIONS[2];
  if (type.includes("summary") || type.includes("semantic")) return BRAIN_REGIONS[1];
  if (type.includes("goal") || type.includes("plan")) return BRAIN_REGIONS[0];
  return BRAIN_REGIONS[seed % BRAIN_REGIONS.length];
}

function brainLayoutPoint(node, index, total) {
  const seed = hashString(`${node.id}:${node.agent}:${node.type}:${index}`);
  const region = brainRegionForNode(node, seed);
  const hemisphere = seed % 2 === 0 ? -1 : 1;
  const angle = (index * 2.399963229728653) + (seededUnit(seed, 2) * Math.PI * 2);
  const density = 0.28 + (seededUnit(seed, 3) * 0.72);
  const layer = total > 1 ? index / (total - 1) : 0.5;
  const outerX = 120;
  const outerY = 92;
  const outerZ = 74;
  const centralGap = 22;
  const x = hemisphere * (centralGap + (region.x * outerX * 0.66))
    + (hemisphere * Math.cos(angle) * outerX * 0.20 * density);
  const y = ((region.y - 0.18) * outerY)
    + (Math.sin(angle) * outerY * 0.24 * density)
    + (Math.sin(layer * Math.PI) * 16);
  const z = (region.z * outerZ)
    + (Math.sin(angle * 1.7) * outerZ * 0.32 * density);

  return {
    brainRegion: region.key,
    brainHemisphere: hemisphere < 0 ? "left" : "right",
    brainX: x,
    brainY: y,
    brainZ: z,
    x,
    y,
    z,
  };
}

function applyBrainLayout(nodes) {
  const total = Math.max(nodes.length, 1);
  return nodes.map((node, index) => ({
    ...node,
    ...brainLayoutPoint(node, index, total),
  }));
}

function createBrainShapeForce(strength = BRAIN_SHAPE_FORCE) {
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

function createLine(points, color = "#40e0ff", opacity = 0.36) {
  const material = new THREE.LineBasicMaterial({
    color,
    transparent: true,
    opacity,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
  });
  const geometry = new THREE.BufferGeometry().setFromPoints(points);
  return new THREE.Line(geometry, material);
}

function hemisphereOutline(centerX, radiusX, radiusY, zBias, side) {
  const points = [];
  const segments = 112;
  for (let index = 0; index <= segments; index += 1) {
    const theta = (index / segments) * Math.PI * 2;
    const lobe = 1 + (Math.sin(theta * 3 + side) * 0.035);
    const x = centerX + (Math.cos(theta) * radiusX * lobe);
    const y = Math.sin(theta) * radiusY * (1 - (Math.cos(theta) * 0.05));
    const z = zBias + (Math.sin(theta * 2) * 12) + (Math.cos(theta) * side * 8);
    points.push(new THREE.Vector3(x, y, z));
  }
  return points;
}

function cortexPath(centerX, y, width, z, side, phase) {
  const points = [];
  const segments = 72;
  for (let index = 0; index <= segments; index += 1) {
    const t = index / segments;
    const x = centerX - (width / 2) + (width * t);
    const arch = Math.sin(t * Math.PI) * 18;
    const wave = Math.sin((t * Math.PI * 5) + phase) * 5;
    points.push(new THREE.Vector3(x, y + arch + wave, z + (Math.cos(t * Math.PI) * side * 7)));
  }
  return points;
}

function ellipseRing(radiusX, radiusY, z, rotationX = 0, rotationY = 0) {
  const points = [];
  const segments = 128;
  const euler = new THREE.Euler(rotationX, rotationY, 0);
  for (let index = 0; index <= segments; index += 1) {
    const theta = (index / segments) * Math.PI * 2;
    const point = new THREE.Vector3(Math.cos(theta) * radiusX, Math.sin(theta) * radiusY, z);
    point.applyEuler(euler);
    points.push(point);
  }
  return points;
}

function createJarvisBrainShell() {
  const group = new THREE.Group();
  group.name = BRAIN_JARVIS_SHELL_NAME;

  const lines = [
    createLine(hemisphereOutline(-76, 72, 82, -4, -1), "#40e0ff", 0.42),
    createLine(hemisphereOutline(76, 72, 82, -4, 1), "#40e0ff", 0.42),
    createLine(hemisphereOutline(-76, 54, 62, 18, -1), "#40e0ff", 0.18),
    createLine(hemisphereOutline(76, 54, 62, 18, 1), "#40e0ff", 0.18),
    createLine(cortexPath(-76, 36, 92, 20, -1, 0.2), "#40e0ff", 0.28),
    createLine(cortexPath(-76, 4, 106, 10, -1, 1.7), "#40e0ff", 0.24),
    createLine(cortexPath(-76, -32, 82, -2, -1, 2.9), "#ffd166", 0.16),
    createLine(cortexPath(76, 36, 92, 20, 1, 1.1), "#40e0ff", 0.28),
    createLine(cortexPath(76, 4, 106, 10, 1, 2.4), "#40e0ff", 0.24),
    createLine(cortexPath(76, -32, 82, -2, 1, 3.2), "#ffd166", 0.16),
    createLine([
      new THREE.Vector3(0, -72, 12),
      new THREE.Vector3(-4, -40, 20),
      new THREE.Vector3(5, -8, 24),
      new THREE.Vector3(-3, 30, 18),
      new THREE.Vector3(0, 74, 10),
    ], "#f8fbff", 0.22),
    createLine(ellipseRing(172, 110, -10, Math.PI * 0.20, 0), "#40e0ff", 0.14),
    createLine(ellipseRing(138, 84, 24, Math.PI * 0.34, Math.PI * 0.12), "#ffd166", 0.10),
    createLine(ellipseRing(118, 68, -32, Math.PI * 0.48, -Math.PI * 0.10), "#40e0ff", 0.11),
  ];

  for (const line of lines) {
    line.renderOrder = 1;
    group.add(line);
  }

  return group;
}

function brainNodeBaseColor(node) {
  return isDecisionNode(node) ? BRAIN_NODE_COLORS.decision : BRAIN_NODE_COLORS.memory;
}

function brainNodeColor(node, selectedNode, selectedFlow) {
  if (selectedNode?.id === node.id) return BRAIN_NODE_COLORS.selected;
  const neighbor = selectedFlow.neighborIds.has(node.id);
  if (neighbor && isDecisionNode(node)) return BRAIN_NODE_COLORS.neighborDecision;
  if (neighbor) return BRAIN_NODE_COLORS.neighborMemory;
  if (selectedNode) return BRAIN_NODE_COLORS.dimmed;
  return brainNodeBaseColor(node);
}

function brainNodeValue(node, selectedNode, selectedFlow) {
  const base = Math.max(2.2, node.val || 3);
  if (selectedNode?.id === node.id) return base * 1.55;
  if (selectedFlow.neighborIds.has(node.id)) return base * 1.18;
  if (selectedNode) return Math.max(1.4, base * 0.76);
  return base;
}

function focusGraphNode(graph, node) {
  if (!graph || !node) return;
  const target = graphNodePosition(node);
  const camera = typeof graph.camera === "function" ? graph.camera() : null;
  const position = camera?.position || { x: 0, y: 0, z: BRAIN_FOCUS_DISTANCE };

  let dx = position.x - target.x;
  let dy = position.y - target.y;
  let dz = position.z - target.z;
  let distance = Math.hypot(dx, dy, dz);

  if (!Number.isFinite(distance) || distance < 1) {
    dx = target.x || 0;
    dy = target.y || 0;
    dz = target.z || 1;
    distance = Math.hypot(dx, dy, dz) || 1;
  }

  const scale = BRAIN_FOCUS_DISTANCE / distance;
  graph.cameraPosition(
    {
      x: target.x + (dx * scale),
      y: target.y + (dy * scale),
      z: target.z + (dz * scale),
    },
    target,
    BRAIN_FOCUS_TRANSITION_MS,
  );
}

function BrainFallbackGraph({
  graphData,
  memoryCt,
  decisionCt,
  selectedNode,
  setSelectedNode,
  reason = "2D fallback: WebGL unavailable",
}) {
  return (
    <div className="brain-container brain-fallback-container">
      <div className="brain-hud brain-hud-fallback">
        <span className="brain-stat"><span className="brain-label">NODES</span> {graphData.nodes.length}</span>
        <span className="brain-stat"><span className="brain-label">LINKS</span> {graphData.links.length}</span>
        <span className="brain-stat"><span className="brain-label">MEM</span> {memoryCt}</span>
        <span className="brain-stat"><span className="brain-label">DEC</span> {decisionCt}</span>
        <span className="brain-fallback-reason">{reason}</span>
      </div>
      <div className="brain-node-fallback-grid">
        {graphData.nodes.map(node => (
          <button
            key={node.id}
            type="button"
            className="brain-node-fallback"
            aria-pressed={selectedNode?.id === node.id}
            onClick={() => setSelectedNode(prev => prev?.id === node.id ? null : node)}
            style={{ "--brain-node-agent-color": getAgentColor(node.agent) }}
          >
            <div className="brain-node-fallback-label">{node.label}</div>
            <div className="brain-node-fallback-meta">{node.group} - {node.agent}</div>
          </button>
        ))}
      </div>
      {selectedNode && (
        <div className="brain-detail brain-detail-fixed">
          <button className="brain-detail-close" onClick={() => setSelectedNode(null)}><AppIcon name="close" size={12} /></button>
          <div className="brain-detail-type">
            <span className="memory-method">{selectedNode.group}</span>
            <span className="memory-method">{selectedNode.type}</span>
          </div>
          <div className="brain-detail-label">{selectedNode.label}</div>
          <div className="brain-detail-agent" style={{ color: getAgentColor(selectedNode.agent) }}>{selectedNode.agent}</div>
          {selectedNode.fullText && <div className="brain-detail-text">{selectedNode.fullText}</div>}
          {selectedNode.context && <div className="brain-detail-ctx"><span className="brain-detail-ctx-label">CONTEXT</span>{selectedNode.context}</div>}
        </div>
      )}
    </div>
  );
}

function BrainVisualizerComponent({ api = null, cortexBase = "http://127.0.0.1:7437", authToken = "", active = true }) {
  const graphRef = useRef(null);
  const rotationRef = useRef(null);
  const hoverNodeIdRef = useRef(null);
  const jarvisShellRef = useRef(null);
  const selectedNodeRef = useRef(null);
  const selectionFrameRef = useRef(null);
  const zoomFrameRef = useRef(null);
  const viewDepthRef = useRef("detail");
  const [graphData, setGraphData] = useState({ nodes: [], links: [] });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);
  const [hoverNode, setHoverNode] = useState(null);
  const [selectedNode, setSelectedNode] = useState(null);
  const [autoRotate, setAutoRotate] = useState(false);
  const [viewDepth, setViewDepth] = useState("detail");
  const [dimensions, setDimensions] = useState({
    width: Math.max(window.innerWidth - 260, 400),
    height: Math.max(window.innerHeight - 20, 300),
  });
  const [webglAvailable] = useState(() => hasWebGLSupport());

  useEffect(() => {
    selectedNodeRef.current = selectedNode;
  }, [selectedNode]);

  useEffect(() => () => {
    if (selectionFrameRef.current) cancelAnimationFrame(selectionFrameRef.current);
    if (zoomFrameRef.current) cancelAnimationFrame(zoomFrameRef.current);
  }, []);

  useEffect(() => {
    if (!active) return undefined;
    function onResize() {
      setDimensions({
        width: Math.max(window.innerWidth - 260, 400),
        height: Math.max(window.innerHeight - 20, 300),
      });
    }
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [active]);

  useEffect(() => {
    const graph = graphRef.current;
    if (!graph) return;
    if (active) {
      if (typeof graph.resumeAnimation === "function") graph.resumeAnimation();
      if (typeof graph.enablePointerInteraction === "function") graph.enablePointerInteraction(true);
      return;
    }
    if (typeof graph.pauseAnimation === "function") graph.pauseAnimation();
    if (typeof graph.enablePointerInteraction === "function") graph.enablePointerInteraction(false);
  }, [active, graphData.nodes.length, graphData.links.length]);

  useEffect(() => {
    if (!active || !webglAvailable || !graphRef.current) return undefined;
    const graph = graphRef.current;
    const scene = typeof graph.scene === "function" ? graph.scene() : null;
    if (!scene) return undefined;

    if (!jarvisShellRef.current) {
      jarvisShellRef.current = createJarvisBrainShell();
    }

    const shell = jarvisShellRef.current;
    if (!scene.getObjectByName(BRAIN_JARVIS_SHELL_NAME)) {
      scene.add(shell);
    }

    return () => {
      if (shell.parent) shell.parent.remove(shell);
    };
  }, [active, graphData.nodes.length, webglAvailable]);

  const syncViewDepth = useCallback(() => {
    const graph = graphRef.current;
    const camera = graph && typeof graph.camera === "function" ? graph.camera() : null;
    const position = camera?.position;
    if (!position) return;

    const distance = Math.hypot(position.x, position.y, position.z);
    const threshold = brainOverviewThreshold(graphData.nodes.length);
    const nextDepth = distance > threshold
      ? "overview"
      : distance < threshold * 0.82
        ? "detail"
        : viewDepthRef.current;

    if (nextDepth !== viewDepthRef.current) {
      viewDepthRef.current = nextDepth;
      setViewDepth(nextDepth);
    }
  }, [graphData.nodes.length]);

  useEffect(() => {
    if (!active || !graphRef.current) return undefined;
    const controls = typeof graphRef.current.controls === "function" ? graphRef.current.controls() : null;
    if (!controls || typeof controls.addEventListener !== "function") return undefined;

    const handleControlsChange = () => {
      if (zoomFrameRef.current) return;
      zoomFrameRef.current = requestAnimationFrame(() => {
        zoomFrameRef.current = null;
        syncViewDepth();
      });
    };

    controls.addEventListener("change", handleControlsChange);
    syncViewDepth();

    return () => {
      controls.removeEventListener("change", handleControlsChange);
      if (zoomFrameRef.current) {
        cancelAnimationFrame(zoomFrameRef.current);
        zoomFrameRef.current = null;
      }
    };
  }, [active, graphData.nodes.length, graphData.links.length, syncViewDepth]);

  useEffect(() => {
    const graph = graphRef.current;
    if (!active || !graph || !graphData.nodes.length) return;

    try {
      const chargeForce = typeof graph.d3Force === "function" ? graph.d3Force("charge") : null;
      if (chargeForce && typeof chargeForce.strength === "function") chargeForce.strength(-32);
      if (chargeForce && typeof chargeForce.distanceMax === "function") chargeForce.distanceMax(240);

      const linkForce = typeof graph.d3Force === "function" ? graph.d3Force("link") : null;
      if (linkForce && typeof linkForce.distance === "function") {
        linkForce.distance(link => {
          const type = String(link.type || "semantic");
          if (type === "conflict") return 64;
          if (String(graphEndpointId(link.source) || "").startsWith("dec-") || String(graphEndpointId(link.target) || "").startsWith("dec-")) return 48;
          return 38;
        });
      }
      if (linkForce && typeof linkForce.strength === "function") {
        linkForce.strength(link => Math.min(0.32, Math.max(0.04, Number(link.weight || 1) * 0.08)));
      }
      if (typeof graph.d3Force === "function") graph.d3Force("brainShape", createBrainShapeForce());
      if (typeof graph.d3ReheatSimulation === "function") graph.d3ReheatSimulation();
    } catch {
      // Force tuning is best-effort; the graph should still render with library defaults.
    }
  }, [active, graphData.nodes.length, graphData.links.length]);

  // Auto-rotation
  useEffect(() => {
    if (!active || !graphRef.current || !autoRotate) {
      if (rotationRef.current) {
        cancelAnimationFrame(rotationRef.current);
        rotationRef.current = null;
      }
      return;
    }

    let angle = 0;
    function rotate() {
      if (!graphRef.current) return;
      angle += 0.0008;
      try {
        const cam = graphRef.current.camera();
        if (!cam) { rotationRef.current = requestAnimationFrame(rotate); return; }
        const pos = cam.position;
        const dist = Math.sqrt(pos.x * pos.x + pos.z * pos.z) || 400;
        graphRef.current.cameraPosition({
          x: dist * Math.sin(angle),
          z: dist * Math.cos(angle),
          y: pos.y,
        });
      } catch { /* ignore rotation errors */ }
      rotationRef.current = requestAnimationFrame(rotate);
    }

    rotationRef.current = requestAnimationFrame(rotate);
    return () => { if (rotationRef.current) cancelAnimationFrame(rotationRef.current); };
  }, [active, autoRotate, graphData]);

  const fetchBrainData = useCallback(async () => {
    try {
      if (typeof api !== "function") {
        setError(`API bridge unavailable for ${cortexBase}`);
        setLoading(false);
        return;
      }

      const dumpRes = await api("/dump", true);

      if (!dumpRes) {
        setError("Could not fetch /dump endpoint");
        setLoading(false);
        return;
      }

      const nodes = [];
      const links = [];
      const nodeIds = new Set();
      const linkSet = new Set();
      const MAX_LINKS = 300;
      const backendLinks = Array.isArray(dumpRes.graph?.links) ? dumpRes.graph.links : null;

      for (const mem of (dumpRes.memories || [])) {
        const id = `mem-${mem.id}`;
        nodeIds.add(id);
        nodes.push({
          id,
          label: truncate(mem.source || mem.text || `Memory ${mem.id}`, 50),
          fullText: mem.text || "",
          type: mem.type || "memory",
          agent: mem.source_agent || "system",
          score: mem.score || 1,
          group: "memory",
          val: 2 + Math.min((mem.score || 1) * 2, 6),
        });
      }

      for (const dec of (dumpRes.decisions || [])) {
        const id = `dec-${dec.id}`;
        nodeIds.add(id);
        nodes.push({
          id,
          label: truncate(dec.decision || `Decision ${dec.id}`, 50),
          fullText: dec.decision || "",
          context: dec.context || "",
          type: "decision",
          agent: dec.source_agent || "system",
          score: dec.score || 1,
          group: "decision",
          status: dec.status || "active",
          val: 3 + Math.min((dec.score || 1) * 2, 6),
        });
      }

      if (backendLinks) {
        for (const link of backendLinks) {
          if (links.length >= MAX_LINKS) break;
          if (!nodeIds.has(link.source) || !nodeIds.has(link.target)) continue;
          const key = [link.source, link.target, link.type || "persisted"].sort().join("|");
          if (linkSet.has(key)) continue;
          linkSet.add(key);
          links.push({
            source: link.source,
            target: link.target,
            type: link.type || "persisted",
            weight: link.weight || 1,
          });
        }
      } else {
        // Legacy fallback for older daemons that do not emit persisted graph links yet.
        for (const dec of (dumpRes.decisions || [])) {
          const source = `dec-${dec.id}`;
          if (dec.disputes_id && nodeIds.has(`dec-${dec.disputes_id}`)) {
            const target = `dec-${dec.disputes_id}`;
            const key = [source, target, "conflict"].sort().join("|");
            if (!linkSet.has(key)) {
              linkSet.add(key);
              links.push({ source, target, type: "conflict", weight: 1 });
            }
          }
        }

        const keywordMap = new Map();
        for (const node of nodes) {
          const words = [...new Set(
            (node.label + " " + (node.fullText || "")).toLowerCase()
              .split(/\W+/)
              .filter(w => w.length > 5)
          )];
          for (const word of words) {
            if (!keywordMap.has(word)) keywordMap.set(word, []);
            keywordMap.get(word).push(node.id);
          }
        }

        for (const [, ids] of keywordMap) {
          if (links.length >= MAX_LINKS) break;
          if (ids.length >= 2 && ids.length <= 4) {
            for (let i = 0; i < ids.length - 1 && links.length < MAX_LINKS; i++) {
              for (let j = i + 1; j < ids.length && links.length < MAX_LINKS; j++) {
                const key = [ids[i], ids[j], "semantic"].sort().join("|");
                if (!linkSet.has(key)) {
                  linkSet.add(key);
                  links.push({ source: ids[i], target: ids[j], type: "semantic", weight: 1 });
                }
              }
            }
          }
        }
      }

      // Final validation — ensure all link targets exist
      const validLinks = links.filter(l => nodeIds.has(l.source) && nodeIds.has(l.target));

      setGraphData({ nodes: applyBrainLayout(nodes), links: validLinks });
      setLoading(false);
    } catch (err) {
      setError(err.message);
      setLoading(false);
    }
  }, [api, cortexBase, authToken]);

  useEffect(() => { fetchBrainData(); }, [fetchBrainData]);

  const memoryCt = useMemo(() => graphData.nodes.filter(n => n.group === "memory").length, [graphData]);
  const decisionCt = useMemo(() => graphData.nodes.filter(n => n.group === "decision").length, [graphData]);
  const selectedFlow = useMemo(() => {
    const selectedId = selectedNode?.id;
    const neighborIds = new Set();
    const typeCounts = new Map();
    const flowLinks = [];

    if (!selectedId) {
      return {
        neighborIds,
        flowLinks,
        connectionCount: 0,
        primaryType: "idle",
      };
    }

    for (const link of graphData.links) {
      const sourceId = graphEndpointId(link.source);
      const targetId = graphEndpointId(link.target);
      if (sourceId !== selectedId && targetId !== selectedId) continue;

      const neighborId = sourceId === selectedId ? targetId : sourceId;
      const type = link.type || "semantic";
      neighborIds.add(neighborId);
      typeCounts.set(type, (typeCounts.get(type) || 0) + 1);
      flowLinks.push({
        neighborId,
        type,
        direction: sourceId === selectedId ? "outbound" : "inbound",
        weight: link.weight || 1,
      });
    }

    const [primaryType = "isolated"] = [...typeCounts.entries()]
      .sort((a, b) => b[1] - a[1])
      .map(([type]) => type);

    return {
      neighborIds,
      flowLinks: flowLinks.slice(0, 5),
      connectionCount: flowLinks.length,
      primaryType,
    };
  }, [graphData.links, selectedNode?.id]);

  const overviewLinkKeys = useMemo(() => {
    const limit = Math.max(28, Math.min(BRAIN_OVERVIEW_LINK_CAP, Math.ceil(graphData.nodes.length * 1.12)));
    if (graphData.links.length <= limit) {
      return new Set(graphData.links.map(graphLinkKey));
    }

    return new Set(
      graphData.links
        .map((link, index) => ({ key: graphLinkKey(link), index, score: brainLinkOverviewScore(link) }))
        .sort((a, b) => b.score - a.score || a.index - b.index)
        .slice(0, limit)
        .map(link => link.key)
    );
  }, [graphData.links, graphData.nodes.length]);

  const isSelectedFlowLink = useCallback((link) => {
    const selectedId = selectedNode?.id;
    if (!selectedId) return false;
    const sourceId = graphEndpointId(link.source);
    const targetId = graphEndpointId(link.target);
    return sourceId === selectedId || targetId === selectedId;
  }, [selectedNode?.id]);

  const resolveNodeColor = useCallback(
    (node) => brainNodeColor(node, selectedNode, selectedFlow),
    [selectedFlow, selectedNode],
  );
  const resolveNodeValue = useCallback(
    (node) => brainNodeValue(node, selectedNode, selectedFlow),
    [selectedFlow, selectedNode],
  );
  const resolveLinkVisibility = useCallback((link) => {
    if (selectedNode) return isSelectedFlowLink(link) || link.type === "conflict";
    if (viewDepth !== "overview") return true;
    return overviewLinkKeys.has(graphLinkKey(link));
  }, [isSelectedFlowLink, overviewLinkKeys, selectedNode, viewDepth]);
  const overviewActive = viewDepth === "overview" && !selectedNode;
  const resolveLinkColor = useCallback((link) => {
    if (link.type === "conflict") return "#ff1744";
    if (isSelectedFlowLink(link)) return "rgba(64, 224, 255, 0.72)";
    if (selectedNode) return "rgba(0, 212, 255, 0.035)";
    return overviewActive ? "rgba(64, 224, 255, 0.22)" : "rgba(0, 212, 255, 0.06)";
  }, [isSelectedFlowLink, overviewActive, selectedNode]);
  const resolveLinkWidth = useCallback((link) => {
    if (link.type === "conflict") return 1.5;
    if (isSelectedFlowLink(link)) return 1.1;
    if (selectedNode) return 0.18;
    return overviewActive ? 0.22 : 0.3;
  }, [isSelectedFlowLink, overviewActive, selectedNode]);
  const resolveLinkParticles = useCallback((link) => {
    if (link.type === "conflict") return selectedNode ? 2 : 0;
    return isSelectedFlowLink(link) ? 2 : 0;
  }, [isSelectedFlowLink, selectedNode]);
  const updateHoverNode = useCallback((node) => {
    const nodeId = node?.id || null;
    if (hoverNodeIdRef.current === nodeId) return;
    hoverNodeIdRef.current = nodeId;
    setHoverNode(node || null);
  }, []);
  const selectGraphNode = useCallback((node) => {
    if (!node) return;
    const current = selectedNodeRef.current;
    const nextNode = current?.id === node.id ? null : node;

    setAutoRotate(false);
    if (nextNode && graphRef.current) focusGraphNode(graphRef.current, nextNode);

    selectedNodeRef.current = nextNode;
    if (selectionFrameRef.current) cancelAnimationFrame(selectionFrameRef.current);
    selectionFrameRef.current = requestAnimationFrame(() => {
      selectionFrameRef.current = null;
      startTransition(() => setSelectedNode(nextNode));
    });
  }, []);

  // Error / loading states
  if (error) {
    return (
      <div className="brain-loading">
        <div className="coming-icon"><AppIcon name="brain" size={48} /></div>
        <p>Error: {error}</p>
        <button className="btn-sm btn-primary" onClick={() => { setError(null); setLoading(true); fetchBrainData(); }} style={{ marginTop: 12 }}>Retry</button>
      </div>
    );
  }

  if (loading) {
    return (
      <div className="brain-loading">
        <div className="coming-icon"><AppIcon name="brain" size={48} /></div>
        <p>Loading brain topology... ({graphData.nodes.length} nodes)</p>
      </div>
    );
  }

  if (graphData.nodes.length === 0) {
    return (
      <div className="brain-loading">
        <div className="coming-icon"><AppIcon name="brain" size={48} /></div>
        <p>No memories found in brain.</p>
      </div>
    );
  }

  // 2D fallback — shown if ForceGraph3D is missing (shouldn't happen with static import)
  if (!ForceGraph3D || !webglAvailable) {
    return (
      <div className="brain-container brain-fallback-container">
        <div className="brain-hud brain-hud-fallback">
          <span className="brain-stat"><span className="brain-label">NODES</span> {graphData.nodes.length}</span>
          <span className="brain-stat"><span className="brain-label">LINKS</span> {graphData.links.length}</span>
          <span className="brain-stat"><span className="brain-label">MEM</span> {memoryCt}</span>
          <span className="brain-stat"><span className="brain-label">DEC</span> {decisionCt}</span>
          <span className="brain-fallback-reason">2D fallback: WebGL unavailable</span>
        </div>
        <div className="brain-node-fallback-grid">
          {graphData.nodes.map(node => (
            <button
              key={node.id}
              type="button"
              className="brain-node-fallback"
              aria-pressed={selectedNode?.id === node.id}
              onClick={() => setSelectedNode(prev => prev?.id === node.id ? null : node)}
              style={{ "--brain-node-agent-color": getAgentColor(node.agent) }}
            >
              <div className="brain-node-fallback-label">{node.label}</div>
              <div className="brain-node-fallback-meta">{node.group} - {node.agent}</div>
            </button>
          ))}
        </div>
        {selectedNode && (
          <div className="brain-detail brain-detail-fixed">
            <button className="brain-detail-close" onClick={() => setSelectedNode(null)}><AppIcon name="close" size={12} /></button>
            <div className="brain-detail-type">
              <span className="memory-method">{selectedNode.group}</span>
              <span className="memory-method">{selectedNode.type}</span>
            </div>
            <div className="brain-detail-label">{selectedNode.label}</div>
            <div className="brain-detail-agent" style={{ color: getAgentColor(selectedNode.agent) }}>{selectedNode.agent}</div>
            {selectedNode.fullText && <div className="brain-detail-text">{selectedNode.fullText}</div>}
            {selectedNode.context && <div className="brain-detail-ctx"><span className="brain-detail-ctx-label">CONTEXT</span>{selectedNode.context}</div>}
          </div>
        )}
      </div>
    );
  }

  // 3D Graph
  return (
    <div className="brain-container" onMouseDown={() => autoRotate && setAutoRotate(false)} onWheel={() => autoRotate && setAutoRotate(false)}>
      <div className="brain-orbital-ring brain-orbital-ring-a" aria-hidden="true" />
      <div className="brain-orbital-ring brain-orbital-ring-b" aria-hidden="true" />

      <div className="brain-hud brain-hud-primary">
        <div className="brain-hud-copy">
          <span className="brain-mode">Neural topology</span>
          <strong className="brain-title">Cortex Brain Map</strong>
          <p>Click any node to pin details. Drag to inspect clusters. Auto-rotate is display mode only.</p>
        </div>
      </div>

      <div className="brain-hud brain-hud-secondary">
        <span className="brain-stat"><span className="brain-label">NODES</span> {graphData.nodes.length}</span>
        <span className="brain-stat"><span className="brain-label">LINKS</span> {graphData.links.length}</span>
        <span className="brain-stat"><span className="brain-label">MEM</span> {memoryCt}</span>
        <span className="brain-stat"><span className="brain-label">DEC</span> {decisionCt}</span>
        {selectedNode ? (
          <span className="brain-stat brain-stat-flow">
            <span className="brain-label">FLOW</span> {selectedFlow.connectionCount}
          </span>
        ) : null}
        <button className={`brain-toggle ${autoRotate ? "active" : ""}`} onClick={() => setAutoRotate(r => !r)} style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
          {autoRotate ? <AppIcon name="refresh" size={14} /> : <AppIcon name="activity" size={14} />}
          <span>{autoRotate ? "AUTO" : "MANUAL"}</span>
        </button>
      </div>

      {selectedNode && (
        <div className="brain-detail">
          <button className="brain-detail-close" onClick={() => setSelectedNode(null)}><AppIcon name="close" size={12} /></button>
          <div className="brain-detail-type">
            <span className="memory-method">{selectedNode.group}</span>
            <span className="memory-method">{selectedNode.type}</span>
          </div>
          <div className="brain-detail-label">{selectedNode.label}</div>
          <div className="brain-detail-agent" style={{ color: getAgentColor(selectedNode.agent) }}>{selectedNode.agent}</div>
          {selectedNode.fullText && <div className="brain-detail-text">{selectedNode.fullText}</div>}
          {selectedNode.context && <div className="brain-detail-ctx"><span className="brain-detail-ctx-label">CONTEXT</span>{selectedNode.context}</div>}
          <div className="brain-flow-panel">
            <div className="brain-flow-head">
              <span>Recall Flow</span>
              <strong>{formatFlowType(selectedFlow.primaryType)}</strong>
            </div>
            {selectedFlow.flowLinks.length ? (
              <div className="brain-flow-list">
                {selectedFlow.flowLinks.map((link) => (
                  <div key={`${link.direction}-${link.neighborId}-${link.type}`} className="brain-flow-row">
                    <span className={`brain-flow-direction ${link.direction}`}>{link.direction}</span>
                    <span className="brain-flow-node">{link.neighborId}</span>
                    <span className="brain-flow-type">{formatFlowType(link.type)}</span>
                  </div>
                ))}
              </div>
            ) : (
              <p className="brain-flow-empty">No immediate graph paths for this node.</p>
            )}
          </div>
          <div className="brain-detail-meta">
            <span>Score: {selectedNode.score?.toFixed(2)}</span>
            <span>ID: {selectedNode.id}</span>
          </div>
        </div>
      )}

      {hoverNode && !selectedNode && (
        <div className="brain-tooltip">
          <div className="brain-tooltip-type">{hoverNode.group} · {hoverNode.type}</div>
          <div className="brain-tooltip-label">{hoverNode.label}</div>
          <div className="brain-tooltip-agent" style={{ color: getAgentColor(hoverNode.agent) }}>{hoverNode.agent}</div>
        </div>
      )}

      <GraphErrorBoundary
        fallback={(
          <BrainFallbackGraph
            graphData={graphData}
            memoryCt={memoryCt}
            decisionCt={decisionCt}
            selectedNode={selectedNode}
            setSelectedNode={setSelectedNode}
            reason="2D fallback: 3D renderer unavailable"
          />
        )}
      >
        <ForceGraph3D
          ref={graphRef}
          graphData={graphData}
          nodeColor={resolveNodeColor}
          nodeVal={resolveNodeValue}
          nodeResolution={8}
          nodeOpacity={0.94}
          nodeRelSize={3.6}
          nodeLabel={node => `${node.label} (${node.agent})`}
          controlType="orbit"
          enableNavigationControls={true}
          showNavInfo={false}
          linkVisibility={resolveLinkVisibility}
          linkColor={resolveLinkColor}
          linkWidth={resolveLinkWidth}
          linkOpacity={overviewActive ? 0.12 : selectedNode ? 0.32 : 0.15}
          linkDirectionalParticles={resolveLinkParticles}
          linkDirectionalParticleWidth={link => isSelectedFlowLink(link) ? 1.8 : 1.5}
          linkDirectionalParticleColor={link => isSelectedFlowLink(link) ? "#40e0ff" : "#ff1744"}
          backgroundColor="#040812"
          width={dimensions.width}
          height={dimensions.height}
          d3AlphaDecay={0.07}
          d3VelocityDecay={0.46}
          warmupTicks={45}
          cooldownTime={1200}
          onNodeHover={updateHoverNode}
          onNodeClick={selectGraphNode}
        />
      </GraphErrorBoundary>

      <div className="brain-legend">
        {BRAIN_LEGEND.map(({ key, label, color }) => (
          <span key={key} className="brain-legend-item">
            <span className="brain-legend-dot" style={{ background: color, boxShadow: `0 0 6px ${color}` }} />
            {label}
          </span>
        ))}
      </div>
    </div>
  );
}

BrainVisualizerComponent.displayName = "BrainVisualizer";
export const BrainVisualizer = memo(BrainVisualizerComponent);
