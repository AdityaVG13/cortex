# Technical Evaluation: Real-Time Brain/Neural Network Graph Visualizer

**Purpose:** Evaluate libraries and techniques for building a desktop app that visualizes knowledge nodes forming and connecting in real-time, with a "brain/neural network" aesthetic.

**Date:** 2026-03-29

---

## 1. Force-Directed Graph Libraries Comparison

### Tier 1: Recommended for This Use Case

#### 3d-force-graph + react-force-graph (vasturiano)
- **GitHub:** [vasturiano/3d-force-graph](https://github.com/vasturiano/3d-force-graph) (5.9k stars) | [react-force-graph](https://github.com/vasturiano/react-force-graph) (3k stars)
- **Rendering:** Three.js/WebGL (3D), HTML5 Canvas (2D)
- **Force Engines:** d3-force-3d (default, highly configurable) or ngraph (faster for large graphs)
- **Modes:** 2D, 3D, VR, AR via separate React components
- **Performance:** Demonstrated with ~4k elements; ngraph engine handles larger graphs faster
- **Key Strengths for Our Use Case:**
  - Built-in `postProcessingComposer()` with **UnrealBloomPass support** (official example exists)
  - `nodeThreeObject` allows custom Three.js Object3D per node (spheres with emissive glow, custom shaders)
  - `linkDirectionalParticles` animates particles flowing along edges (built-in "electrical impulse" effect)
  - Incremental `graphData()` updates without full re-render
  - Smooth camera transitions via `cameraPosition()` with duration control
  - Pause/resume animation for performance control
- **Bloom Example Code:**
  ```javascript
  import { UnrealBloomPass } from 'three/examples/jsm/postprocessing/UnrealBloomPass.js';
  const bloomPass = new UnrealBloomPass();
  bloomPass.strength = 4;
  bloomPass.radius = 1;
  bloomPass.threshold = 0;
  fgRef.current.postProcessingComposer().addPass(bloomPass);
  ```
- **Verdict:** **PRIMARY RECOMMENDATION.** Best balance of 3D aesthetics, built-in bloom/particles, React integration, and reasonable performance. The "brain" aesthetic comes almost free.

#### cosmos.gl (Cosmograph)
- **GitHub:** [cosmosgl/graph](https://github.com/cosmosgl/graph) (1.1k stars)
- **Rendering:** WebGL 2 via luma.gl
- **Performance:** **Handles hundreds of thousands to 1 million nodes** in real-time. All computation on GPU via fragment/vertex shaders. Joined OpenJS Foundation.
- **Key Strengths:** Unmatched raw performance; GPU-resident force simulation
- **Limitations:** No built-in bloom/glow effects; less visual customization than Three.js-based solutions; no 3D mode (2D only); limited node styling
- **Verdict:** **Best for massive graphs (100k+ nodes)** but lacks the visual richness needed for a "brain" aesthetic. Consider as a fallback if node counts exceed 10k.

### Tier 2: Strong Alternatives

#### Sigma.js + Graphology
- **GitHub:** [jacomyal/sigma.js](https://github.com/jacomyal/sigma.js) (12k stars)
- **Rendering:** WebGL (nodes/edges) + Canvas (labels)
- **Performance:** Renders 100k edges easily with default styles; struggles at 5k nodes with icons; ForceAtlas2 layout degrades beyond 50k edges
- **Key Strengths:** Graphology ecosystem provides Louvain community detection, ForceAtlas2 layout, graph metrics. Excellent for 2D network analysis.
- **Limitations:** 2D only; no built-in 3D, bloom, or particle effects; v4 (rewriting renderer API) planned for 2025-2026
- **Verdict:** Best pure-2D graph library. Not suitable for the "brain" aesthetic but excellent data layer (Graphology) could feed into a Three.js renderer.

#### AntV G6
- **GitHub:** [antvis/G6](https://github.com/antvis/G6) (12k stars)
- **Rendering:** Canvas, SVG, WebGL, server-side Node.js
- **Performance:** Rust+WASM parallel computing for layouts; WebGPU acceleration for some layouts
- **Key Strengths:** 10+ built-in layouts, rich interactions, themes, 3D plugin packages, comprehensive Chinese ecosystem (Ant Group)
- **Limitations:** Primarily 2D-focused; 3D is a plugin, not core; documentation heavily Chinese-first
- **Verdict:** Feature-rich but 3D/aesthetic capabilities trail behind Three.js-based solutions.

#### Reagraph
- **GitHub:** [reaviz/reagraph](https://github.com/reaviz/reagraph) (1k+ stars)
- **Rendering:** WebGL
- **Key Strengths:** React-native API, 15+ layout algorithms, clustering, path finding, edge bundling, light/dark themes
- **Limitations:** Less mature ecosystem; no built-in bloom/particle effects
- **Verdict:** Simpler React API than react-force-graph but fewer visual customization options.

### Tier 3: Considered but Not Recommended

| Library | Stars | Reason Not Recommended |
|---------|-------|----------------------|
| **Cytoscape.js** | 10.9k | Canvas-based, WebGL renderer still experimental (3 FPS to 10 FPS improvement on 3.2k nodes); compound nodes expensive; no 3D |
| **vis-network** | ~3k | Canvas only; smooth up to "a few thousand nodes"; no WebGL, no 3D, no bloom |
| **D3-force (raw)** | N/A | SVG-based by default; requires manual Canvas/WebGL integration; maximum flexibility but maximum work |

---

## 2. Neural Network / Brain Visual Aesthetic

### Making Nodes Look Like Neurons

**Technique: Emissive Sphere + Selective Bloom**
```javascript
// Custom node as glowing sphere
nodeThreeObject: (node) => {
  const geometry = new THREE.SphereGeometry(node.size || 4);
  const material = new THREE.MeshStandardMaterial({
    color: node.color,
    emissive: node.color,
    emissiveIntensity: 0.8,
    toneMapped: false  // Critical: allows HDR values for bloom
  });
  return new THREE.Mesh(geometry, material);
}
```

**Selective Bloom** isolates glowing objects into a separate Three.js Layer, renders them with UnrealBloomPass, then composites back. This prevents the entire scene from blooming. [Official Three.js example](https://threejs.org/examples/webgl_postprocessing_unreal_bloom_selective.html).

### Dendrite-Like Edges
- Use `linkCurvature` in 3d-force-graph for organic curved connections
- `linkDirectionalParticles` with low speed creates "synaptic pulse" effect along edges
- Custom `linkThreeObject` can render edges as tubes with varying thickness

### Real-Time Node Addition Animation
1. **Fade-in:** New node spawns with `opacity: 0`, transitions to `1` over 500ms
2. **Position settling:** Node appears at a random position near its cluster; force simulation naturally pulls it into place
3. **Connection drawing:** Links animate from source to new node using `linkDirectionalParticles`
4. **Layout stability:** Use incremental layout updates -- existing nodes shift minimally to accommodate new ones ("mental map preservation")

### Decay Visualization
- **Fading:** Reduce `emissiveIntensity` proportional to decay score
- **Shrinking:** Scale node geometry based on access frequency
- **Dimming:** Shift color toward gray/transparent as relevance decreases
- **Death animation:** Node implodes (scale to 0) then removed from graph data

### Cluster Detection and Zoom Levels

**Galaxy View (zoomed out):**
- Clusters appear as colored nebulae/clouds
- Individual nodes not visible; replaced by cluster centroids
- Color = agent source or topic category

**Cluster View (mid zoom):**
- Individual nodes visible as small glowing dots
- Edges visible within cluster
- Inter-cluster edges shown as thick bundled lines

**Node View (zoomed in):**
- Full node detail: label, metadata, timestamps
- Edge labels visible
- Neighboring nodes highlighted

**Implementation:** Use D3's semantic zoom or Three.js LOD (Level of Detail) system. Switch between SimplifiedClusterMesh, PointCloudMesh, and DetailedNodeMesh based on camera distance.

---

## 3. Real-Time Data Flow Visualization

### Animating Data Along Edges (Electrical Impulses)

**Built-in with 3d-force-graph:**
```javascript
linkDirectionalParticles: 4,           // number of particles per link
linkDirectionalParticleWidth: 2,       // particle size
linkDirectionalParticleSpeed: 0.005,   // speed along edge
linkDirectionalParticleColor: '#00ffff' // cyan "electrical" color
```

**Custom "Recall" Visualization:**
When a memory is recalled, highlight the traversal path:
1. Query node pulses bright
2. Particles trace from query through intermediate nodes to matched memory
3. Matched memory node flares with increased bloom
4. Path edges temporarily thicken and glow

**Reference:** Netflix's [Vizceral](https://github.com/Netflix/vizceral) (4.1k stars) animated traffic volume between nodes at three drill-down levels (global, regional, service). No longer maintained but excellent design reference.

### SSE/WebSocket-Driven Updates

**Architecture:**
```
Cortex Backend (Python)
  ---> SSE/WebSocket stream
    ---> React Frontend
      ---> Buffered state updates
        ---> react-force-graph re-render
```

**Key Techniques to Avoid Layout Thrashing:**
1. **Buffer incoming events** in a mutable ref; flush on `requestAnimationFrame` cadence
2. **Batch node additions** -- add up to N nodes per frame, not one per event
3. **Pin existing nodes** temporarily when many new nodes arrive (freeze positions)
4. **Use `d3AlphaDecay`** set high (0.05-0.1) so simulation settles quickly after changes
5. **Incremental layout algorithms** place new nodes near neighbors without repositioning existing ones

### Color-Coding by Source/Agent
| Agent/Source | Color | Hex |
|-------------|-------|-----|
| Claude (Cortex) | Electric Blue | `#00BFFF` |
| Local LLM (Ollama) | Amber/Orange | `#FF8C00` |
| User Input | Green | `#00FF88` |
| System/Auto | Purple | `#9B59B6` |
| Decaying | Gray (fading) | `#555555` |

---

## 4. Rendering Technology Decision

### Benchmark Summary (2025 Data)

| Technology | Element Threshold | Init Time | Frame Time (interaction) | Best For |
|-----------|------------------|-----------|--------------------------|----------|
| **SVG** | Up to ~3k elements | ~5ms | ~16ms | Small graphs, accessibility |
| **Canvas** | Up to ~10k elements | ~15ms | ~1.2ms | Mid-size 2D graphs |
| **WebGL** | 10k-100k+ elements | ~40ms | ~0.01ms | Large graphs, 3D, effects |
| **WebGPU** | 100k+ elements | ~50ms | <0.01ms | Massive particle systems |

**Decision: WebGL (Three.js) with WebGPU fallback path.**

Rationale:
- Our target is 1,000-5,000 nodes -- well within WebGL's sweet spot
- Three.js UnrealBloomPass requires WebGL
- WebGPU is production-ready since Three.js r171 (Sep 2025) with `import * as THREE from 'three/webgpu'` and automatic WebGL 2 fallback
- WebGPU compute shaders offer 150x improvement for particle systems (10k particles: 30ms/frame on CPU vs <2ms on WebGPU for 100k particles)

### GPU-Accelerated Particle Systems

**three.quarks** ([GitHub](https://github.com/Alchemist0823/three.quarks), 777 stars):
- General-purpose VFX engine for Three.js
- Billboard sprites, stretched billboards, mesh particles, ribbon trails
- Color/size/rotation over lifetime, force fields, orbital motion
- React Three Fiber integration via `quarks.r3f`
- Visual editor at quarks.art
- Batch rendering minimizes draw calls

**GPGPU Particles** (Codrops tutorial):
- GPU-resident particle positions updated via custom GLSL shaders
- Curl-noise motion for organic particle drift
- Ideal for ambient "floating dust" or "neural energy" background effects

### Bloom/Glow Post-Processing

**Three.js UnrealBloomPass** parameters:
- `resolution`: Higher = sharper bloom (performance cost)
- `strength`: Glow intensity (0-10, typical: 2-4)
- `radius`: Spread distance (0-2, typical: 0.5-1)
- `threshold`: Brightness cutoff (0 = everything blooms, 1 = only brightest)

**Selective Bloom** (only specific objects glow):
1. Assign glowing objects to Three.js Layer 1
2. Render Layer 1 only with BloomComposer
3. Render full scene with MainComposer
4. Merge via custom shader
5. [Three.js official selective bloom example](https://threejs.org/examples/webgl_postprocessing_unreal_bloom_selective.html)

---

## 5. Desktop Shell Decision

### Electron vs Tauri

| Factor | Electron | Tauri v2 |
|--------|----------|----------|
| **Bundle Size** | 150-300 MB | 5-10 MB |
| **RAM Idle** | 200-300 MB | 30-40 MB |
| **Startup** | 1-2 seconds | < 0.5 seconds |
| **WebGL Support** | Full Chromium (excellent) | WebView2/WKWebView (good, some edge cases) |
| **Three.js Compat** | Excellent, same as Chrome | **Reported issues**: context loss, performance degradation in production builds |
| **WebGPU** | Supported (Chromium flags) | Depends on system webview version |
| **GPU Acceleration** | Full, configurable | System-dependent |
| **Security** | Broad Node.js access (lockdown required) | Rust backend, opt-in narrow API |

**Decision: Electron for v1, evaluate Tauri for v2.**

Rationale:
- Three.js + WebGL + bloom post-processing has **known issues in Tauri** (WebView2 context loss, performance drops in production builds)
- Electron ships Chromium, guaranteeing identical rendering to Chrome dev testing
- The 200MB size penalty is acceptable for a desktop visualization tool
- WebGPU migration later benefits from Electron's Chromium engine
- If Tauri's WebView2 WebGL issues are resolved, migrate to Tauri v2 for the massive size/memory improvements

---

## 6. Reference Projects and Design Inspiration

### Direct References

| Project | Stars | Tech Stack | Relevance |
|---------|-------|-----------|-----------|
| **[Vestige](https://github.com/samvallad33/vestige)** | 455 | SvelteKit + Three.js + WebSocket | **Closest match.** AI cognitive memory with 3D neural visualization, bloom post-processing, force-directed layout. Maintains 60fps at 1000+ nodes. Purple "dream mode" during consolidation. |
| **[Graphiti](https://github.com/getzep/graphiti)** | 24.3k | Python + Neo4j | Temporal knowledge graph engine. No built-in visualizer but the data model (bi-temporal facts, validity windows, provenance) is the ideal backend for our visualization. |
| **[Netflix Vizceral](https://github.com/Netflix/vizceral)** | 4.1k | WebGL | Animated traffic flow at three zoom levels (global/regional/service). Not maintained but excellent UX reference for drill-down navigation and flow animation. |
| **Obsidian Graph View** | N/A | Proprietary (Electron + Canvas/WebGL) | Well-known knowledge graph UX. Force-directed layout with semantic zoom. Open-source alternatives: Logseq, AnyType. |
| **Neo4j Bloom** | N/A | Commercial | Natural-language graph exploration. Scene-based visualization. Design reference for search-driven graph exploration UX. |

### Vestige Deep Dive (Closest Analog)

Vestige is the most relevant reference project. Key implementation details:
- **SvelteKit 2 + Svelte 5** frontend (we'd use React, but architecture is transferable)
- **Three.js** for WebGL rendering with bloom post-processing
- **WebSocket** for real-time cognitive operation broadcasts
- **Force-directed layout** organizes memories spatially by cognitive relationships
- **60 FPS at 1,000+ nodes** -- validates our target performance
- **Visual states:** Memory creation (pulse), search queries (path highlight), decay (fade), consolidation ("dream mode" purple shift)
- **29 brain modules** including FSRS-6 spaced repetition
- **Single 22MB Rust binary** backend

---

## 7. Recommended Stack

### Primary Stack

```
Desktop Shell:     Electron (Chromium WebGL guarantee)
Frontend:          React 19 + TypeScript
Graph Engine:      react-force-graph-3d (vasturiano)
3D Rendering:      Three.js (WebGL, WebGPU future path)
Post-Processing:   UnrealBloomPass (selective bloom)
Particle Effects:  three.quarks (VFX) + linkDirectionalParticles (built-in)
Data Layer:        Graphology (community detection, metrics, algorithms)
Layout:            d3-force-3d (default) or ngraph (if performance needed)
Real-Time:         WebSocket from Cortex Python backend
State Management:  Zustand (lightweight, works with refs for animation state)
```

### Architecture

```
┌─────────────────────────────────────────────────┐
│  Electron Main Process                          │
│  ┌───────────────────────────────────────────┐  │
│  │  Renderer Process (Chromium)              │  │
│  │  ┌─────────────────────────────────────┐  │  │
│  │  │  React App                          │  │  │
│  │  │  ┌───────────┐  ┌───────────────┐  │  │  │
│  │  │  │  Zustand   │  │  WebSocket    │  │  │  │
│  │  │  │  Store     │◄─┤  Client       │  │  │  │
│  │  │  └─────┬─────┘  └───────┬───────┘  │  │  │
│  │  │        │                │           │  │  │
│  │  │  ┌─────▼─────────────────▼───────┐  │  │  │
│  │  │  │  react-force-graph-3d         │  │  │  │
│  │  │  │  ┌─────────────────────────┐  │  │  │  │
│  │  │  │  │  Three.js Scene         │  │  │  │  │
│  │  │  │  │  • Custom node meshes   │  │  │  │  │
│  │  │  │  │  • Emissive materials   │  │  │  │  │
│  │  │  │  │  • Bloom composer       │  │  │  │  │
│  │  │  │  │  • three.quarks VFX     │  │  │  │  │
│  │  │  │  └─────────────────────────┘  │  │  │  │
│  │  │  └───────────────────────────────┘  │  │  │
│  │  └─────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────┘  │
└─────────────────────────┬───────────────────────┘
                          │ WebSocket
                          ▼
              ┌───────────────────────┐
              │  Cortex Backend       │
              │  (Python, port 7437)  │
              │  • Memory CRUD events │
              │  • Recall queries     │
              │  • Decay scores       │
              │  • Cluster updates    │
              └───────────────────────┘
```

### Implementation Priority

| Phase | Feature | Effort |
|-------|---------|--------|
| **P0** | react-force-graph-3d with bloom + basic nodes/edges | 2 days |
| **P1** | WebSocket integration with Cortex for live memory events | 1 day |
| **P2** | Custom neuron-style nodes (emissive spheres, size by importance) | 1 day |
| **P3** | Particle flow along edges (linkDirectionalParticles) | 0.5 day |
| **P4** | Decay visualization (fade, shrink, dim) | 1 day |
| **P5** | Semantic zoom (galaxy/cluster/node detail levels) | 2 days |
| **P6** | Recall path highlighting (query -> matched memories) | 1 day |
| **P7** | Agent color coding + legend | 0.5 day |
| **P8** | three.quarks ambient particle effects | 1 day |
| **P9** | Electron packaging + GPU optimization | 1 day |

**Total estimated: ~10 days to full "brain visualizer" experience.**

---

## 8. Key Technical Risks and Mitigations

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| Performance degrades beyond 5k nodes | Medium | Switch force engine to ngraph; implement LOD; use cosmos.gl for 2D fallback |
| Bloom post-processing causes frame drops | Low | Reduce bloom resolution; use selective bloom on fewer objects; throttle to 30fps |
| WebSocket event flood causes layout thrashing | Medium | Buffer events in mutable ref; batch updates on rAF; pin existing nodes during bursts |
| Electron bundle too large | Low | Acceptable for desktop tool; tree-shake Three.js imports; lazy-load VFX |
| Three.js WebGPU migration breaks bloom | Low | WebGPU has automatic WebGL 2 fallback; test bloom in WebGPU renderer before migrating |

---

## Sources

### Libraries
- [3d-force-graph](https://github.com/vasturiano/3d-force-graph)
- [react-force-graph](https://github.com/vasturiano/react-force-graph)
- [Sigma.js](https://github.com/jacomyal/sigma.js)
- [Cytoscape.js](https://github.com/cytoscape/cytoscape.js)
- [AntV G6](https://github.com/antvis/G6)
- [cosmos.gl](https://github.com/cosmosgl/graph)
- [Reagraph](https://github.com/reaviz/reagraph)
- [three.quarks](https://github.com/Alchemist0823/three.quarks)
- [Graphology](https://graphology.github.io/standard-library/communities-louvain.html)

### Reference Projects
- [Vestige - Cognitive Memory for AI Agents](https://github.com/samvallad33/vestige)
- [Graphiti - Temporal Knowledge Graphs](https://github.com/getzep/graphiti)
- [Netflix Vizceral - Traffic Visualization](https://github.com/Netflix/vizceral)
- [Three.js Neural Network Visualizer](https://github.com/marcusbuffett/Three.js-Neural-Network-Visualizer)

### Techniques and Tutorials
- [Three.js Unreal Bloom (official)](https://threejs.org/examples/webgl_postprocessing_unreal_bloom.html)
- [Three.js Selective Bloom (official)](https://threejs.org/examples/webgl_postprocessing_unreal_bloom_selective.html)
- [GPGPU Particle Effects with Three.js (Codrops)](https://tympanus.net/codrops/2024/12/19/crafting-a-dreamy-particle-effect-with-three-js-and-gpgpu/)
- [SVG vs Canvas vs WebGL Benchmarks 2025](https://www.svggenie.com/blog/svg-vs-canvas-vs-webgl-performance-2025)
- [WebGPU Three.js Migration Guide 2026](https://www.utsubo.com/blog/webgpu-threejs-migration-guide)
- [Force-Directed Graph Layout Stability (yWorks)](https://www.yworks.com/pages/force-directed-graph-layout)
- [Graph Visualization Efficiency (PMC)](https://pmc.ncbi.nlm.nih.gov/articles/PMC12061801/)

### Design References
- [Neo4j Bloom](https://neo4j.com/product/bloom/)
- [Cosmograph](https://cosmograph.app/docs-general/)
- [Electron vs Tauri Comparison](https://www.dolthub.com/blog/2025-11-13-electron-vs-tauri/)
- [Unreal Bloom Selective Tutorial](https://waelyasmina.net/articles/unreal-bloom-selective-threejs-post-processing/)
