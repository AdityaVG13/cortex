import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const wrapper = readFileSync(new URL("./BrainVisualizer.jsx", import.meta.url), "utf8");
const v2Index = readFileSync(new URL("./brain-v2/index.jsx", import.meta.url), "utf8");
const scene = readFileSync(new URL("./brain-v2/Scene.js", import.meta.url), "utf8");
const core = readFileSync(new URL("./brain-v2/Core.js", import.meta.url), "utf8");
const halo = readFileSync(new URL("./brain-v2/Halo.js", import.meta.url), "utf8");
const easing = readFileSync(new URL("./brain-v2/util/easing.js", import.meta.url), "utf8");
const tiers = readFileSync(new URL("./brain-v2/Tiers.js", import.meta.url), "utf8");
const satellites = readFileSync(new URL("./brain-v2/Satellites.js", import.meta.url), "utf8");
const palette = readFileSync(new URL("./brain-v2/ClusterPalette.js", import.meta.url), "utf8");
const fnv1a = readFileSync(new URL("./brain-v2/util/fnv1a.js", import.meta.url), "utf8");

describe("Brain v2 wrapper", () => {
  it("v1 force-graph and brain/* modules are gone", () => {
    expect(wrapper).not.toContain("react-force-graph-3d");
    expect(wrapper).not.toContain("postprocessing");
    expect(wrapper).not.toContain("BRAIN_REGIONS");
    expect(wrapper).not.toContain("createJarvisBrainShell");
    expect(wrapper).not.toContain("createConstellationShells");
    expect(wrapper).not.toContain("RippleEngine");
    expect(wrapper).not.toContain("EdgeMesh");
  });

  it("BrainVisualizer wraps BrainV2 with WebGL detection + error boundary", () => {
    expect(wrapper).toContain("import { BrainV2 } from \"./brain-v2/index.jsx\"");
    expect(wrapper).toContain("hasWebGLSupport");
    expect(wrapper).toContain("GraphErrorBoundary");
    expect(wrapper).toContain("<BrainV2");
  });
});

describe("Brain v2 scene scaffolding", () => {
  it("BrainV2 mounts the core and registers a tick", () => {
    expect(v2Index).toContain("createScene");
    expect(v2Index).toContain("createCore()");
    expect(v2Index).toContain("tickCore(core, t, now)");
    expect(v2Index).toContain("disposeCore");
  });

  it("Scene uses LinearToneMapping and origin-locked OrbitControls", () => {
    expect(scene).toContain("THREE.LinearToneMapping");
    expect(scene).not.toContain("ACESFilmicToneMapping");
    expect(scene).toContain("OrbitControls");
    expect(scene).toContain("controls.target.set(0, 0, 0)");
    expect(scene).toContain("controls.zoomSpeed = 0.7");
  });

  it("Core defines counter-rotation rates and breathing constants", () => {
    expect(core).toContain("OUTER_ROT_RATE = 0.18");
    expect(core).toContain("INNER_ROT_RATE = -0.32");
    expect(core).toContain("HALO_BREATH_AMPLITUDE = 0.08");
    expect(core).toContain("export function createCore");
    expect(core).toContain("export function tickCore");
    expect(core).toContain("export function pulseCoreHalo");
    expect(core).toContain("IcosahedronGeometry");
    expect(core).toContain("WireframeGeometry");
  });

  it("Halo exports a memoized canvas-built radial gradient texture", () => {
    expect(halo).toContain("export function getHaloTexture");
    expect(halo).toContain("createRadialGradient");
    expect(halo).toContain("CanvasTexture");
  });

  it("Easing helpers ported from v1", () => {
    expect(easing).toContain("export function clamp01");
    expect(easing).toContain("export function easeOutCubic");
    expect(easing).toContain("export function expDecay");
    expect(easing).toContain("export function riseDecay");
  });

  it("Tiers builds three layers with Fibonacci spacing and cold-start fallback", () => {
    expect(tiers).toContain("export function buildTiers");
    expect(tiers).toContain("TIER_DECISION_RADIUS = 80");
    expect(tiers).toContain("TIER_CLUSTER_RADIUS = 140");
    expect(tiers).toContain("TIER_LOOSE_RADIUS_MIN = 180");
    expect(tiers).toContain("TIER_LOOSE_RADIUS_MAX = 220");
    expect(tiers).toContain("TOTAL_BUDGET_MIN = 70");
    expect(tiers).toContain("TOTAL_BUDGET_MAX = 90");
    expect(tiers).toContain("DECISION_RATIO = 0.15");
    expect(tiers).toContain("CLUSTER_RATIO = 0.55");
    expect(tiers).toContain("LOOSE_RATIO = 0.30");
    expect(tiers).toContain("useColdStart");
  });

  it("Satellites uses InstancedMesh bodies + halos and exposes pulse + selection", () => {
    expect(satellites).toContain("export function createSatellites");
    expect(satellites).toContain("InstancedMesh");
    expect(satellites).toContain("AdditiveBlending");
    expect(satellites).toContain("setData");
    expect(satellites).toContain("pulseSlot");
    expect(satellites).toContain("setSelected");
  });

  it("ClusterPalette derives hue from FNV-1a hash via golden-angle stride", () => {
    expect(palette).toContain("export function paletteForCluster");
    expect(palette).toContain("GOLDEN_ANGLE = 137.508");
    expect(palette).toContain("SATURATION = 0.70");
    expect(palette).toContain("LIGHTNESS = 0.58");
    expect(palette).toContain("fnv1a32");
  });

  it("FNV-1a hash exports a 32-bit unsigned function", () => {
    expect(fnv1a).toContain("export function fnv1a32");
    expect(fnv1a).toContain("2166136261");
    expect(fnv1a).toContain("16777619");
  });
});

describe("Brain v2 Beams + PulseShader", () => {
  const beams = readFileSync(new URL("./brain-v2/Beams.js", import.meta.url), "utf8");
  const pulseShader = readFileSync(new URL("./brain-v2/PulseShader.js", import.meta.url), "utf8");
  const bezier = readFileSync(new URL("./brain-v2/util/bezierArc.js", import.meta.url), "utf8");

  it("Beams pool 64 slots, single merged LineSegments, GLSL pulse material", () => {
    expect(beams).toContain("POOL_SIZE = 64");
    expect(beams).toContain("SEGMENTS = 16");
    expect(beams).toContain("LineSegments");
    expect(beams).toContain("createPulseMaterial");
    expect(beams).toContain("createActivationTexture");
    expect(beams).toContain("export function createBeams");
    expect(beams).toContain("riseDecay");
  });

  it("PulseShader uses RedFormat FloatType DataTexture with NearestFilter", () => {
    expect(pulseShader).toContain("export function createActivationTexture");
    expect(pulseShader).toContain("export function createPulseMaterial");
    expect(pulseShader).toContain("RedFormat");
    expect(pulseShader).toContain("FloatType");
    expect(pulseShader).toContain("NearestFilter");
    expect(pulseShader).toContain("AdditiveBlending");
    expect(pulseShader).toContain("uTime");
    expect(pulseShader).toContain("vProgress");
    expect(pulseShader).toContain("aColor");
  });

  it("bezierArcPoints returns segments+1 control-lifted points", () => {
    expect(bezier).toContain("export function bezierArcPoints");
    expect(bezier).toContain("normalize().multiplyScalar");
  });

  it("BrainV2 mounts Beams alongside Satellites and exposes window.__brainFire", () => {
    expect(v2Index).toContain("createBeams");
    expect(v2Index).toContain("beamsRef.current?.fire");
    expect(v2Index).toContain("window.__brainFire");
  });
});

describe("Brain v2 firing pipeline (P5)", () => {
  const firingClient = readFileSync(new URL("./brain-v2/FiringClient.js", import.meta.url), "utf8");
  const idleSim = readFileSync(new URL("./brain-v2/IdleSimulator.js", import.meta.url), "utf8");
  const dispatcher = readFileSync(new URL("./brain-v2/EventDispatcher.js", import.meta.url), "utf8");
  const mulberry = readFileSync(new URL("./brain-v2/util/mulberry32.js", import.meta.url), "utf8");

  it("FiringClient uses native EventSource with ?token= and listens for brain_batch", () => {
    expect(firingClient).toContain("new EventSource(url)");
    expect(firingClient).toContain("token=");
    expect(firingClient).toContain("brain_batch");
    expect(firingClient).toContain("encodeURIComponent(token)");
    expect(firingClient).not.toContain("setTimeout");
  });

  it("IdleSimulator threshold is 6s, fake interval 0.9-2.4s, burst 2-4 with mulberry32", () => {
    expect(idleSim).toContain("IDLE_THRESHOLD_MS = 6_000");
    expect(idleSim).toContain("FAKE_INTERVAL_MIN_MS = 900");
    expect(idleSim).toContain("FAKE_INTERVAL_MAX_MS = 2_400");
    expect(idleSim).toContain("BURST_MIN = 2");
    expect(idleSim).toContain("BURST_MAX = 4");
    expect(idleSim).toContain("mulberry32");
    expect(idleSim).toContain("noteRealEvent");
  });

  it("EventDispatcher routes the five firing event types", () => {
    expect(dispatcher).toContain("consolidation_started");
    expect(dispatcher).toContain("member_added");
    expect(dispatcher).toContain("cluster_finalized");
    expect(dispatcher).toContain("link_inferred");
    expect(dispatcher).toContain("recall");
    expect(dispatcher).toContain("dispatchFake");
  });

  it("mulberry32 PRNG is reproducible from a seed", () => {
    expect(mulberry).toContain("export function mulberry32");
    expect(mulberry).toContain("0x6D2B79F5");
  });

  it("BrainV2 wires FiringClient + IdleSimulator + EventDispatcher", () => {
    expect(v2Index).toContain("createFiringClient");
    expect(v2Index).toContain("createIdleSimulator");
    expect(v2Index).toContain("createEventDispatcher");
    expect(v2Index).toContain("idleSim.noteRealEvent()");
    expect(v2Index).toContain("dispatcher.dispatch(event)");
  });
});

describe("Brain v2 interaction (P6)", () => {
  const hover = readFileSync(new URL("./brain-v2/Hover.js", import.meta.url), "utf8");
  const cameraSrc = readFileSync(new URL("./brain-v2/Camera.js", import.meta.url), "utf8");
  const hud = readFileSync(new URL("./brain-v2/Hud.jsx", import.meta.url), "utf8");
  const css = readFileSync(new URL("./styles.css", import.meta.url), "utf8");

  it("Hover uses ray-vs-sphere against slot positions, rAF-throttled", () => {
    expect(hover).toContain("export function createHover");
    expect(hover).toContain("setCursor");
    expect(hover).toContain("clearCursor");
    expect(hover).toContain("function tick");
    expect(hover).toContain("Raycaster");
    expect(hover).toContain("hitRadiusScale");
  });

  it("Camera defines auto-rotate + single-phase spotlight envelope", () => {
    expect(cameraSrc).toContain("AUTO_ROTATE_RATE = 0.04");
    expect(cameraSrc).toContain("AUTO_RESUME_MS = 8_000");
    expect(cameraSrc).toContain("SPOTLIGHT_PULL = 0.15");
    expect(cameraSrc).toContain("SPOTLIGHT_DURATION_MS = 1_200");
    expect(cameraSrc).toContain("easeInOutCubic");
    expect(cameraSrc).toContain("export function createCamera");
    expect(cameraSrc).toContain("pauseAutoRotate");
    expect(cameraSrc).toContain("spotlight(satelliteWorldPos)");
    expect(cameraSrc).toContain("cameraStart.copy(camera.position)");
    expect(cameraSrc).toContain("cameraEnd.copy(");
  });

  it("Hud renders tooltip + detail panel via React props", () => {
    expect(hud).toContain("brain-v2-tooltip");
    expect(hud).toContain("brain-v2-detail");
    expect(hud).toContain("brain-v2-detail-grid");
    expect(hud).toContain("brain-v2-detail-row");
    expect(hud).toContain("hover && !selected");
    expect(hud).toContain("function tierLabel");
  });

  it("Stats + ticker render via direct DOM refs in BrainV2 (no React reconciliation)", () => {
    expect(v2Index).toContain("brain-v2-hud-strip");
    expect(v2Index).toContain("brain-v2-ticker");
    expect(v2Index).toContain("statRefs");
    expect(v2Index).toContain("function writeStats");
    expect(v2Index).toContain("function renderTicker");
    expect(v2Index).toContain("now - lastStatsAtRef.current >= 1000");
  });

  it("BrainV2 wires hover + camera spotlight + click-pin + right-click deselect", () => {
    expect(v2Index).toContain("createHover");
    expect(v2Index).toContain("createCamera");
    expect(v2Index).toContain("hoveredSlotRef");
    expect(v2Index).toContain("selectedSlotRef");
    expect(v2Index).toContain("cameraHandleRef.current.spotlight(slot)");
    expect(v2Index).toContain("e.preventDefault()");
    expect(v2Index).toContain("onContextMenu={handleContextMenu}");
  });

  it("CSS includes Brain v2 HUD layout rules", () => {
    expect(css).toContain(".brain-v2-hud-strip");
    expect(css).toContain(".brain-v2-ticker");
    expect(css).toContain(".brain-v2-tooltip");
    expect(css).toContain(".brain-v2-detail");
  });
});
