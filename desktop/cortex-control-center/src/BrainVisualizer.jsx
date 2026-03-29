import { useCallback, useEffect, useRef, useState } from "react";
import ForceGraph3D from "react-force-graph-3d";
import * as THREE from "three";

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

export function BrainVisualizer() {
  const graphRef = useRef(null);
  const rotationRef = useRef(null);
  const [graphData, setGraphData] = useState({ nodes: [], links: [] });
  const [loading, setLoading] = useState(true);
  const [hoverNode, setHoverNode] = useState(null);
  const [selectedNode, setSelectedNode] = useState(null);
  const [autoRotate, setAutoRotate] = useState(true);
  const [stats, setStats] = useState({ nodes: 0, links: 0 });
  const [dimensions, setDimensions] = useState({
    width: window.innerWidth - 240,
    height: window.innerHeight,
  });

  // Track window resize
  useEffect(() => {
    function onResize() {
      setDimensions({
        width: window.innerWidth - 240,
        height: window.innerHeight,
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
      angle += 0.0008; // Slow, gentle rotation
      const cam = graphRef.current.camera();
      if (!cam) { rotationRef.current = requestAnimationFrame(rotate); return; }
      // Rotate at whatever distance the user is currently at
      const pos = cam.position;
      const dist = Math.sqrt(pos.x * pos.x + pos.z * pos.z) || 400;
      graphRef.current.cameraPosition({
        x: dist * Math.sin(angle),
        z: dist * Math.cos(angle),
        y: pos.y, // Preserve vertical position
      });
      rotationRef.current = requestAnimationFrame(rotate);
    }

    rotationRef.current = requestAnimationFrame(rotate);
    return () => {
      if (rotationRef.current) cancelAnimationFrame(rotationRef.current);
    };
  }, [autoRotate, graphData]);

  // Stop auto-rotate on user interaction
  const handleInteraction = useCallback(() => {
    if (autoRotate) setAutoRotate(false);
  }, [autoRotate]);

  const fetchBrainData = useCallback(async () => {
    try {
      const dumpRes = await fetch(`${CORTEX_BASE}/dump`).then(r => r.json()).catch(() => null);

      if (!dumpRes) {
        setLoading(false);
        return;
      }

      const nodes = [];
      const links = [];
      const linkSet = new Set();

      const memories = dumpRes.memories || [];
      for (const mem of memories) {
        nodes.push({
          id: `mem-${mem.id}`,
          label: truncate(mem.source || mem.text || `Memory ${mem.id}`, 50),
          fullText: mem.text || "",
          type: mem.type || "memory",
          agent: mem.source_agent || "system",
          score: mem.score || 1,
          group: "memory",
          size: 3 + Math.min((mem.score || 1) * 2, 6),
        });
      }

      const decisions = dumpRes.decisions || [];
      for (const dec of decisions) {
        nodes.push({
          id: `dec-${dec.id}`,
          label: truncate(dec.decision || `Decision ${dec.id}`, 50),
          fullText: dec.decision || "",
          context: dec.context || "",
          type: "decision",
          agent: dec.source_agent || "system",
          score: dec.score || 1,
          group: "decision",
          status: dec.status || "active",
          size: 4 + Math.min((dec.score || 1) * 2, 6),
        });

        if (dec.disputes_id) {
          links.push({
            source: `dec-${dec.id}`,
            target: `dec-${dec.disputes_id}`,
            type: "conflict",
            color: "#ff1744",
          });
        }
      }

      // Keyword-based semantic links
      const keywordMap = new Map();
      for (const node of nodes) {
        const text = (node.label + " " + (node.fullText || "")).toLowerCase();
        const words = [...new Set(text.split(/\W+/).filter(w => w.length > 4))];
        for (const word of words) {
          if (!keywordMap.has(word)) keywordMap.set(word, []);
          keywordMap.get(word).push(node.id);
        }
      }

      for (const [, ids] of keywordMap) {
        if (ids.length >= 2 && ids.length <= 5) {
          for (let i = 0; i < ids.length - 1; i++) {
            for (let j = i + 1; j < ids.length; j++) {
              const key = [ids[i], ids[j]].sort().join("|");
              if (!linkSet.has(key)) {
                linkSet.add(key);
                links.push({
                  source: ids[i],
                  target: ids[j],
                  type: "semantic",
                  color: "rgba(0, 212, 255, 0.06)",
                });
              }
            }
          }
        }
      }

      setGraphData({ nodes, links });
      setStats({ nodes: nodes.length, links: links.length });
      setLoading(false);
    } catch (err) {
      console.error("Brain fetch failed:", err);
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchBrainData();
  }, [fetchBrainData]);

  // Scene setup
  useEffect(() => {
    if (!graphRef.current) return;
    const scene = graphRef.current.scene();
    if (!scene) return;

    scene.background = new THREE.Color("#060a12");

    if (!scene.children.some(c => c.isAmbientLight)) {
      scene.add(new THREE.AmbientLight(0x404040, 0.5));
      const light = new THREE.PointLight(0x00d4ff, 0.3, 1000);
      light.position.set(0, 200, 0);
      scene.add(light);
    }
  }, [graphData]);

  const nodeThreeObject = useCallback((node) => {
    const color = getAgentColor(node.agent);
    const size = node.size || 4;
    const isSelected = selectedNode?.id === node.id;

    const geo = new THREE.SphereGeometry(size, 16, 12);
    const mat = new THREE.MeshStandardMaterial({
      color: new THREE.Color(color),
      emissive: new THREE.Color(color),
      emissiveIntensity: isSelected ? 1.0 : node.group === "decision" ? 0.6 : 0.4,
      roughness: 0.3,
      metalness: 0.1,
      transparent: true,
      opacity: isSelected ? 1.0 : 0.85,
    });

    const mesh = new THREE.Mesh(geo, mat);

    if (node.group === "decision") {
      const ringGeo = new THREE.RingGeometry(size * 1.3, size * 1.5, 24);
      const ringMat = new THREE.MeshBasicMaterial({
        color: new THREE.Color(color),
        transparent: true,
        opacity: isSelected ? 0.4 : 0.15,
        side: THREE.DoubleSide,
      });
      const ring = new THREE.Mesh(ringGeo, ringMat);
      ring.lookAt(0, 0, 1);
      mesh.add(ring);
    }

    return mesh;
  }, [selectedNode]);

  function handleNodeClick(node) {
    if (!node) return;
    setSelectedNode(prev => prev?.id === node.id ? null : node);
    setAutoRotate(false);

    if (graphRef.current) {
      const distance = 60;
      const distRatio = 1 + distance / Math.hypot(node.x, node.y, node.z);
      graphRef.current.cameraPosition(
        { x: node.x * distRatio, y: node.y * distRatio, z: node.z * distRatio },
        node,
        1200
      );
    }
  }

  if (loading) {
    return (
      <div className="brain-loading">
        <div className="coming-icon" style={{ fontSize: 48 }}>◬</div>
        <p>Loading brain topology...</p>
      </div>
    );
  }

  if (graphData.nodes.length === 0) {
    return (
      <div className="brain-loading">
        <div className="coming-icon" style={{ fontSize: 48 }}>◬</div>
        <p>No data from /dump endpoint.</p>
      </div>
    );
  }

  const memoryCt = graphData.nodes.filter(n => n.group === "memory").length;
  const decisionCt = graphData.nodes.filter(n => n.group === "decision").length;

  return (
    <div className="brain-container" onMouseDown={handleInteraction} onWheel={handleInteraction}>
      <div className="brain-hud">
        <span className="brain-stat"><span className="brain-label">NODES</span> {stats.nodes}</span>
        <span className="brain-stat"><span className="brain-label">LINKS</span> {stats.links}</span>
        <span className="brain-stat"><span className="brain-label">MEM</span> {memoryCt}</span>
        <span className="brain-stat"><span className="brain-label">DEC</span> {decisionCt}</span>
        <button
          className={`brain-toggle ${autoRotate ? "active" : ""}`}
          onClick={() => setAutoRotate(r => !r)}
          title="Toggle auto-rotation"
        >
          {autoRotate ? "⟳ AUTO" : "⊘ MANUAL"}
        </button>
      </div>

      {/* Detail Panel — shows on node click */}
      {selectedNode && (
        <div className="brain-detail">
          <button className="brain-detail-close" onClick={() => setSelectedNode(null)}>✕</button>
          <div className="brain-detail-type">
            <span className="memory-method">{selectedNode.group}</span>
            <span className="memory-method">{selectedNode.type}</span>
            {selectedNode.status === "disputed" && <span className="memory-method" style={{ background: "var(--red-dim)", color: "var(--red)", borderColor: "rgba(255,23,68,0.2)" }}>DISPUTED</span>}
          </div>
          <div className="brain-detail-label">{selectedNode.label}</div>
          <div className="brain-detail-agent" style={{ color: getAgentColor(selectedNode.agent) }}>
            {selectedNode.agent}
          </div>
          {selectedNode.fullText && (
            <div className="brain-detail-text">{selectedNode.fullText}</div>
          )}
          {selectedNode.context && (
            <div className="brain-detail-ctx">
              <span className="brain-detail-ctx-label">CONTEXT</span>
              {selectedNode.context}
            </div>
          )}
          <div className="brain-detail-meta">
            <span>Score: {selectedNode.score?.toFixed(2)}</span>
            <span>ID: {selectedNode.id}</span>
          </div>
        </div>
      )}

      {/* Hover tooltip (smaller, follows cursor area) */}
      {hoverNode && !selectedNode && (
        <div className="brain-tooltip">
          <div className="brain-tooltip-type">{hoverNode.group} · {hoverNode.type}</div>
          <div className="brain-tooltip-label">{hoverNode.label}</div>
          <div className="brain-tooltip-agent" style={{ color: getAgentColor(hoverNode.agent) }}>
            {hoverNode.agent}
          </div>
        </div>
      )}

      <ForceGraph3D
        ref={graphRef}
        graphData={graphData}
        nodeThreeObject={nodeThreeObject}
        nodeLabel=""
        linkColor={link => link.color || "rgba(0, 212, 255, 0.06)"}
        linkWidth={link => link.type === "conflict" ? 1.5 : 0.3}
        linkOpacity={0.15}
        linkDirectionalParticles={link => link.type === "conflict" ? 4 : 0}
        linkDirectionalParticleWidth={1.5}
        linkDirectionalParticleColor={() => "#ff1744"}
        backgroundColor="#060a12"
        width={dimensions.width}
        height={dimensions.height}
        d3AlphaDecay={0.03}
        d3VelocityDecay={0.4}
        warmupTicks={60}
        cooldownTime={3000}
        onNodeHover={node => setHoverNode(node || null)}
        onNodeClick={handleNodeClick}
      />

      <div className="brain-legend">
        {Object.entries(AGENT_COLORS).slice(0, 5).map(([agent, color]) => (
          <span key={agent} className="brain-legend-item">
            <span className="brain-legend-dot" style={{ background: color, boxShadow: `0 0 6px ${color}` }} />
            {agent}
          </span>
        ))}
        <span className="brain-legend-item">
          <span className="brain-legend-dot" style={{ background: "#546580" }} />
          ○ memory
        </span>
        <span className="brain-legend-item">
          <span className="brain-legend-dot" style={{ background: "#00d4ff", boxShadow: "0 0 6px #00d4ff" }} />
          ◇ decision
        </span>
      </div>
    </div>
  );
}
