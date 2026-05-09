import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("./BrainVisualizer.jsx", import.meta.url), "utf8");
const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
const shellGeometry = readFileSync(new URL("./brain/ShellGeometry.js", import.meta.url), "utf8");
const shellLayout = readFileSync(new URL("./brain/ShellLayout.js", import.meta.url), "utf8");
const renderLayers = readFileSync(new URL("./brain/RenderLayers.js", import.meta.url), "utf8");
const postFx = readFileSync(new URL("./brain/PostFx.js", import.meta.url), "utf8");
const pulseShader = readFileSync(new URL("./brain/PulseShader.js", import.meta.url), "utf8");
const edgeMesh = readFileSync(new URL("./brain/EdgeMesh.js", import.meta.url), "utf8");

function readBlock(text, needle) {
  const start = text.indexOf(needle);
  expect(start, `missing block ${needle}`).toBeGreaterThanOrEqual(0);

  const bodyStart = text.indexOf("{", start);
  expect(bodyStart, `missing body for ${needle}`).toBeGreaterThanOrEqual(0);

  let depth = 1;
  for (let index = bodyStart + 1; index < text.length; index += 1) {
    if (text[index] === "{") {
      depth += 1;
    } else if (text[index] === "}") {
      depth -= 1;
    }

    if (depth === 0) {
      return text.slice(bodyStart + 1, index);
    }
  }

  throw new Error(`unterminated block ${needle}`);
}

describe("Brain visualizer", () => {
  it("keeps selected-node recall flow visible in the details panel", () => {
    expect(source).toContain("const selectedFlow = useMemo");
    expect(source).toContain("brain-stat brain-stat-flow");
    expect(source).toContain("brain-flow-panel");
  });

  it("uses native graph nodes for smooth type-colored rendering", () => {
    expect(source).not.toContain("nodeThreeObject={");
    expect(source).toContain("const BRAIN_NODE_COLORS = Object.freeze");
    expect(source).toContain("nodeColor={resolveNodeColor}");
    expect(source).toContain("nodeVal={resolveNodeValue}");
    expect(source).toContain("nodeResolution={8}");
  });

  it("keeps the screen-space Brain overlay static and cheap", () => {
    expect(source).toContain("brain-orbital-ring brain-orbital-ring-a");
    expect(source).toContain("brain-orbital-ring brain-orbital-ring-b");
    expect(source).not.toContain("brain-hologram-shell");
    expect(source).not.toContain("brain-scanline");

    expect(readBlock(css, ".brain-orbital-ring {")).not.toContain("animation:");
    expect(css).not.toContain("brain-scanline");
    expect(css).not.toContain("brain-ring-drift");
    expect(css).not.toContain(".brain-container canvas");
  });

  it("smooths selected-node focus and disables built-in link rendering", () => {
    expect(source).toContain("const BRAIN_FOCUS_TRANSITION_MS = 1550");
    expect(source).toContain("function focusGraphNode");
    expect(source).toContain("startTransition(() => setSelectedNode(nextNode))");
    expect(source).toContain("linkVisibility={false}");
    expect(source).not.toContain("BRAIN_OVERVIEW_LINK_CAP");
    expect(source).not.toContain("linkDirectionalParticles=");
  });

  it("uses the constellation lattice layout instead of anatomical hemispheres", () => {
    expect(source).not.toContain("BRAIN_REGIONS");
    expect(source).not.toContain("brainRegionForNode");
    expect(source).not.toContain("applyBrainLayout");
    expect(source).not.toContain("createBrainShapeForce");
    expect(source).toContain("import { applyShellLayout, createShellProjectionForce }");
    expect(source).toContain("applyShellLayout(nodes, { useShellSplit: useShellSplitRef.current })");
    expect(source).toContain("graph.d3Force(\"shellProjection\", createShellProjectionForce())");
  });

  it("renders the constellation shell scaffolding inside the rotatable 3D scene", () => {
    expect(source).not.toContain("createJarvisBrainShell");
    expect(source).not.toContain("BRAIN_JARVIS_SHELL_NAME");
    expect(source).toContain("createConstellationShells()");
    expect(source).toContain("CONSTELLATION_SHELL_NAME");
    expect(source).toContain("scene.add(shell)");
    expect(source).toContain("controlType=\"orbit\"");
    expect(source).toContain("enableNavigationControls={true}");
    expect(source).toContain("showNavInfo={false}");
  });

  it("ShellGeometry exports the constellation builders", () => {
    expect(shellGeometry).toContain("export const CONSTELLATION_SHELL_NAME");
    expect(shellGeometry).toContain("export function createConstellationShells");
    expect(shellGeometry).toContain("export function disposeConstellationShells");
    expect(shellGeometry).toContain("IcosahedronGeometry");
    expect(shellGeometry).toContain("WireframeGeometry");
  });

  it("ShellLayout exposes deterministic shell projection", () => {
    expect(shellLayout).toContain("export function applyShellLayout");
    expect(shellLayout).toContain("export function createShellProjectionForce");
    expect(shellLayout).toContain("useShellSplit");
    expect(shellLayout).toContain("fibonacciSphere");
  });

  it("RenderLayers defines BASE/BLOOM layer constants and helpers", () => {
    expect(renderLayers).toContain("export const BRAIN_LAYERS");
    expect(renderLayers).toContain("BASE: 0");
    expect(renderLayers).toContain("BLOOM: 1");
    expect(renderLayers).toContain("export function assignLayer");
    expect(renderLayers).toContain("export function markBloom");
  });

  it("PostFx wires luminance-thresholded bloom + ACES tonemapping + auto-degrade thresholds", () => {
    expect(postFx).toContain("import { BloomEffect, EffectComposer, EffectPass, RenderPass } from \"postprocessing\"");
    expect(postFx).toContain("ACESFilmicToneMapping");
    expect(postFx).toContain("BLOOM_INTENSITY = 0.85");
    expect(postFx).toContain("BLOOM_THRESHOLD = 0.18");
    expect(postFx).toContain("BLOOM_SMOOTHING = 0.4");
    expect(postFx).toContain("DEGRADE_DISABLE_MS = 33.3");
    expect(postFx).toContain("DEGRADE_REENABLE_MS = 22");
    expect(postFx).toContain("DEGRADE_REENABLE_SUSTAIN_MS = 3000");
    expect(postFx).toContain("export function attachBloom");
    expect(postFx).toContain("export function refreshBloomSelection");
  });

  it("BrainVisualizer mounts post-fx, assigns layers, and exposes bloom state", () => {
    expect(source).toContain("import { BRAIN_LAYERS, assignLayer, markBloom }");
    expect(source).toContain("import { attachBloom }");
    expect(source).toContain("attachBloom(graph,");
    expect(source).toContain("assignLayer(jarvisShellRef.current, BRAIN_LAYERS.BASE)");
    expect(source).toContain("data-bloom={bloomActive ? \"on\" : \"off\"}");
    expect(source).toContain("data-shell-split={useShellSplit ? \"on\" : \"off\"}");
  });

  it("PulseShader exposes the GLSL traveling-pulse material and activation texture", () => {
    expect(pulseShader).toContain("export function createActivationTexture");
    expect(pulseShader).toContain("export function createPulseMaterial");
    expect(pulseShader).toContain("DataTexture");
    expect(pulseShader).toContain("RedFormat");
    expect(pulseShader).toContain("FloatType");
    expect(pulseShader).toContain("AdditiveBlending");
    expect(pulseShader).toContain("uTime");
    expect(pulseShader).toContain("uActivation");
    expect(pulseShader).toContain("vProgress");
  });

  it("EdgeMesh builds a single merged BufferGeometry with per-vertex aProgress + aEdgeId", () => {
    expect(edgeMesh).toContain("export function buildEdgeMesh");
    expect(edgeMesh).toContain("export function disposeEdgeMesh");
    expect(edgeMesh).toContain("export function tickEdgeMaterialTime");
    expect(edgeMesh).toContain("aProgress");
    expect(edgeMesh).toContain("aEdgeId");
    expect(edgeMesh).toContain("LineSegments");
    expect(edgeMesh).toContain("brainEdgeMesh: true");
  });

  it("BrainVisualizer mounts the merged edge mesh on the default layer and ticks pulse each frame", () => {
    expect(source).toContain("import { buildEdgeMesh, disposeEdgeMesh, tickEdgeMaterialTime }");
    expect(source).toContain("buildEdgeMesh(graphData.links, nodesById,");
    expect(source).not.toContain("assignLayer(mesh, BRAIN_LAYERS.BLOOM)");
    expect(source).toContain("markBloom(mesh, true)");
    expect(source).toContain("tickEdgeMaterialTime(mesh, elapsedSec)");
    expect(source).toContain("disposeEdgeMesh(edgeMeshRef.current)");
  });

  it("slows scroll-zoom while leaving orbit defaults intact", () => {
    expect(source).toContain("controls.zoomSpeed = 0.7");
    expect(source).not.toContain("controls.enableDamping = true");
  });

  it("right-click deselects the active node and suppresses the context menu", () => {
    expect(source).toContain("e.button === 2");
    expect(source).toContain("onContextMenu={(e) => e.preventDefault()}");
    expect(source).toContain("startTransition(() => setSelectedNode(null))");
  });

  it("BrainVisualizer instantiates RippleEngine, ticks per frame, and fires on click", () => {
    expect(source).toContain("import { RippleEngine }");
    expect(source).toContain("new RippleEngine()");
    expect(source).toContain("engine.attachMesh(mesh)");
    expect(source).toContain("engine.buildAdjacency(graphData.links)");
    expect(source).toContain("engine.tick(now)");
    expect(source).toContain("rippleEngineRef.current?.fire(nextNode.id, performance.now())");
  });
});
