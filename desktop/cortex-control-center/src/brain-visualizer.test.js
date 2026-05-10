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
    expect(tiers).toContain("MAX_DECISIONS = 20");
    expect(tiers).toContain("MAX_CLUSTERS = 80");
    expect(tiers).toContain("MAX_LOOSE = 50");
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
