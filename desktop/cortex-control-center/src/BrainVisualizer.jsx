import { Component, useCallback, useEffect, useMemo, useRef, useState } from "react";
import ForceGraph3D from "react-force-graph-3d";

const CORTEX_BASE = "http://127.0.0.1:7437";

const AGENT_COLORS = {
  claude: "#4a9eff",
  droid: "#ff9800",
  "factory-droid": "#ff9800",
  gemini: "#a855f7",
  ollama: "#22c55e",
  mcp: "#00d4ff",
  system: "#546580",
};

function getAgentColor(agent) {
  if (!agent) return "#00d4ff";
  const key = agent.toLowerCase();
  for (const [k, v] of Object.entries(AGENT_COLORS)) {
    if (key.includes(k)) return v;
  }
  return "#00d4ff";
}

function truncate(str, len) {
  if (!str) return "";
  return str.length > len ? str.slice(0, len) + "..." : str;
}

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
          <div className="coming-icon" style={{ fontSize: 48 }}>◬</div>
          <p>3D renderer crashed: {this.state.error}</p>
          <p style={{ fontSize: 12, color: "#546580", marginTop: 8 }}>Showing 2D fallback instead.</p>
        </div>
      );
    }
    return this.props.children;
  }
}

export function BrainVisualizer() {
  const graphRef = useRef(null);
  const rotationRef = useRef(null);
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

  useEffect(() => {
    function onResize() {
      setDimensions({
        width: Math.max(window.innerWidth - 260, 400),
        height: Math.max(window.innerHeight - 20, 300),
      });
    }
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  // Auto-rotation
  useEffect(() => {
    if (!graphRef.current || !autoRotate) {
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
  }, [autoRotate, graphData]);

  const fetchBrainData = useCallback(async () => {
    try {
      const dumpRes = await fetch(`${CORTEX_BASE}/dump`)
        .then(r => r.ok ? r.json() : null)
        .catch(() => null);

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

        if (dec.disputes_id && nodeIds.has(`dec-${dec.disputes_id}`)) {
          links.push({ source: id, target: `dec-${dec.disputes_id}`, type: "conflict" });
        }
      }

      // Keyword links (capped)
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
              const key = [ids[i], ids[j]].sort().join("|");
              if (!linkSet.has(key)) {
                linkSet.add(key);
                links.push({ source: ids[i], target: ids[j], type: "semantic" });
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
  }, []);

  useEffect(() => { fetchBrainData(); }, [fetchBrainData]);

  const memoryCt = useMemo(() => graphData.nodes.filter(n => n.group === "memory").length, [graphData]);
  const decisionCt = useMemo(() => graphData.nodes.filter(n => n.group === "decision").length, [graphData]);

  // Error / loading states
  if (error) {
    return (
      <div className="brain-loading">
        <div className="coming-icon" style={{ fontSize: 48 }}>◬</div>
        <p>Error: {error}</p>
        <button className="btn-sm btn-primary" onClick={() => { setError(null); setLoading(true); fetchBrainData(); }} style={{ marginTop: 12 }}>Retry</button>
      </div>
    );
  }

  if (loading) {
    return (
      <div className="brain-loading">
        <div className="coming-icon" style={{ fontSize: 48 }}>◬</div>
        <p>Loading brain topology... ({graphData.nodes.length} nodes)</p>
      </div>
    );
  }

  if (graphData.nodes.length === 0) {
    return (
      <div className="brain-loading">
        <div className="coming-icon" style={{ fontSize: 48 }}>◬</div>
        <p>No memories found in brain.</p>
      </div>
    );
  }

  // 2D fallback — shown if ForceGraph3D is missing (shouldn't happen with static import)
  if (!ForceGraph3D) {
    return (
      <div className="brain-container" style={{ padding: 24, overflowY: "auto" }}>
        <div className="brain-hud" style={{ position: "relative", marginBottom: 16 }}>
          <span className="brain-stat"><span className="brain-label">NODES</span> {graphData.nodes.length}</span>
          <span className="brain-stat"><span className="brain-label">LINKS</span> {graphData.links.length}</span>
          <span className="brain-stat"><span className="brain-label">MEM</span> {memoryCt}</span>
          <span className="brain-stat"><span className="brain-label">DEC</span> {decisionCt}</span>
          <span style={{ color: "#ff9800", fontSize: 11, marginLeft: "auto" }}>2D Fallback — WebGL unavailable</span>
        </div>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
          {graphData.nodes.map(node => (
            <div key={node.id} onClick={() => setSelectedNode(prev => prev?.id === node.id ? null : node)}
              style={{
                padding: "8px 12px",
                background: selectedNode?.id === node.id ? "rgba(0,212,255,0.1)" : "#0f1520",
                border: `1px solid ${selectedNode?.id === node.id ? "rgba(0,212,255,0.3)" : "#1e2d42"}`,
                borderRadius: 6,
                cursor: "pointer",
                borderLeft: `3px solid ${getAgentColor(node.agent)}`,
                maxWidth: 280,
                fontSize: 12,
              }}
            >
              <div style={{ fontWeight: 600, color: "#e8edf5" }}>{node.label}</div>
              <div style={{ color: "#546580", fontSize: 11 }}>{node.group} · {node.agent}</div>
            </div>
          ))}
        </div>
        {selectedNode && (
          <div className="brain-detail" style={{ position: "fixed" }}>
            <button className="brain-detail-close" onClick={() => setSelectedNode(null)}>✕</button>
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
      <div className="brain-hud">
        <span className="brain-stat"><span className="brain-label">NODES</span> {graphData.nodes.length}</span>
        <span className="brain-stat"><span className="brain-label">LINKS</span> {graphData.links.length}</span>
        <span className="brain-stat"><span className="brain-label">MEM</span> {memoryCt}</span>
        <span className="brain-stat"><span className="brain-label">DEC</span> {decisionCt}</span>
        <button className={`brain-toggle ${autoRotate ? "active" : ""}`} onClick={() => setAutoRotate(r => !r)}>
          {autoRotate ? "⟳ AUTO" : "⊘ MANUAL"}
        </button>
      </div>

      {selectedNode && (
        <div className="brain-detail">
          <button className="brain-detail-close" onClick={() => setSelectedNode(null)}>✕</button>
          <div className="brain-detail-type">
            <span className="memory-method">{selectedNode.group}</span>
            <span className="memory-method">{selectedNode.type}</span>
          </div>
          <div className="brain-detail-label">{selectedNode.label}</div>
          <div className="brain-detail-agent" style={{ color: getAgentColor(selectedNode.agent) }}>{selectedNode.agent}</div>
          {selectedNode.fullText && <div className="brain-detail-text">{selectedNode.fullText}</div>}
          {selectedNode.context && <div className="brain-detail-ctx"><span className="brain-detail-ctx-label">CONTEXT</span>{selectedNode.context}</div>}
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

      <ForceGraph3D
        ref={graphRef}
        graphData={graphData}
        nodeColor={node => getAgentColor(node.agent)}
        nodeVal={node => node.val || 3}
        nodeLabel={node => `${node.label} (${node.agent})`}
        linkColor={link => link.type === "conflict" ? "#ff1744" : "rgba(0, 212, 255, 0.06)"}
        linkWidth={link => link.type === "conflict" ? 1.5 : 0.3}
        linkOpacity={0.15}
        linkDirectionalParticles={link => link.type === "conflict" ? 3 : 0}
        linkDirectionalParticleWidth={1.5}
        linkDirectionalParticleColor={() => "#ff1744"}
        backgroundColor="#060a12"
        width={dimensions.width}
        height={dimensions.height}
        d3AlphaDecay={0.06}
        d3VelocityDecay={0.5}
        warmupTicks={20}
        cooldownTime={1500}
        onNodeHover={node => setHoverNode(node || null)}
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

      <div className="brain-legend">
        {Object.entries(AGENT_COLORS).slice(0, 5).map(([agent, color]) => (
          <span key={agent} className="brain-legend-item">
            <span className="brain-legend-dot" style={{ background: color, boxShadow: `0 0 6px ${color}` }} />
            {agent}
          </span>
        ))}
      </div>
    </div>
  );
}
