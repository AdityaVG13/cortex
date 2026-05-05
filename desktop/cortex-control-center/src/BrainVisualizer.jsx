import { Component, memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import ForceGraph3D from "react-force-graph-3d";
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

function formatFlowType(type) {
  return String(type || "semantic").replace(/[_-]+/g, " ");
}

function isDecisionNode(node) {
  return node?.group === "decision" || node?.type === "decision";
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
  const [graphData, setGraphData] = useState({ nodes: [], links: [] });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);
  const [hoverNode, setHoverNode] = useState(null);
  const [selectedNode, setSelectedNode] = useState(null);
  const [autoRotate, setAutoRotate] = useState(false);
  const [dimensions, setDimensions] = useState({
    width: Math.max(window.innerWidth - 260, 400),
    height: Math.max(window.innerHeight - 20, 300),
  });
  const [webglAvailable] = useState(() => hasWebGLSupport());

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

      setGraphData({ nodes, links: validLinks });
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
  const updateHoverNode = useCallback((node) => {
    const nodeId = node?.id || null;
    if (hoverNodeIdRef.current === nodeId) return;
    hoverNodeIdRef.current = nodeId;
    setHoverNode(node || null);
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
          linkColor={link => (
            link.type === "conflict"
              ? "#ff1744"
              : isSelectedFlowLink(link)
                ? "rgba(64, 224, 255, 0.72)"
                : selectedNode
                  ? "rgba(0, 212, 255, 0.035)"
                  : "rgba(0, 212, 255, 0.06)"
          )}
          linkWidth={link => link.type === "conflict" ? 1.5 : isSelectedFlowLink(link) ? 1.1 : selectedNode ? 0.18 : 0.3}
          linkOpacity={selectedNode ? 0.32 : 0.15}
          linkDirectionalParticles={link => link.type === "conflict" ? 3 : isSelectedFlowLink(link) ? 2 : 0}
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
          onNodeClick={node => {
            if (!node) return;
            setSelectedNode(prev => prev?.id === node.id ? null : node);
            setAutoRotate(false);
            if (graphRef.current) {
              const d = 60;
              const ratio = 1 + d / Math.hypot(node.x, node.y, node.z);
              graphRef.current.cameraPosition(
                { x: node.x * ratio, y: node.y * ratio, z: node.z * ratio },
                node,
                1200
              );
            }
          }}
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
