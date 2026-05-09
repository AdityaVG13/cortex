import * as THREE from "three";
import { getHaloTexture } from "./Halo.js";

const CORE_RADIUS = 25;
const HALO_RADIUS = 80;
const HALO_COLOR = "#40e0ff";
const OUTER_COLOR = "#40e0ff";
const INNER_COLOR = "#ffd166";

const OUTER_ROT_RATE = 0.18;
const INNER_ROT_RATE = -0.32;
const HALO_BREATH_HZ = 2 * Math.PI / 1.5;
const HALO_BREATH_AMPLITUDE = 0.08;

function wireframeIcosahedron(radius, color, opacity) {
  const base = new THREE.IcosahedronGeometry(radius, 1);
  const wire = new THREE.WireframeGeometry(base);
  base.dispose();
  const material = new THREE.LineBasicMaterial({
    color,
    transparent: true,
    opacity,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
  });
  return new THREE.LineSegments(wire, material);
}

export function createCore() {
  const group = new THREE.Group();
  group.name = "brain-v2-core";

  const outer = wireframeIcosahedron(CORE_RADIUS, OUTER_COLOR, 0.85);
  const inner = wireframeIcosahedron(CORE_RADIUS * 0.65, INNER_COLOR, 0.55);
  outer.name = "core-outer";
  inner.name = "core-inner";

  const haloMaterial = new THREE.SpriteMaterial({
    map: getHaloTexture(),
    color: new THREE.Color(HALO_COLOR),
    transparent: true,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
  });
  const halo = new THREE.Sprite(haloMaterial);
  halo.name = "core-halo";
  halo.scale.set(HALO_RADIUS * 2, HALO_RADIUS * 2, 1);

  group.add(halo);
  group.add(outer);
  group.add(inner);

  group.userData = {
    haloIntensityBase: 1,
    haloPulseUntil: 0,
  };

  return group;
}

export function tickCore(group, t, now = performance.now()) {
  if (!group) return;
  const outer = group.getObjectByName("core-outer");
  const inner = group.getObjectByName("core-inner");
  const halo = group.getObjectByName("core-halo");

  if (outer) outer.rotation.y = t * OUTER_ROT_RATE;
  if (inner) {
    inner.rotation.x = -t * INNER_ROT_RATE;
    inner.rotation.y = t * INNER_ROT_RATE;
  }

  if (halo) {
    const breath = 1 + Math.sin(t * HALO_BREATH_HZ) * HALO_BREATH_AMPLITUDE;
    let pulse = 1;
    const remaining = group.userData.haloPulseUntil - now;
    if (remaining > 0) {
      const progress = 1 - remaining / 800;
      const peak = 0.2;
      pulse = 1 + Math.sin(progress * Math.PI) * peak;
    }
    const intensity = group.userData.haloIntensityBase * breath * pulse;
    halo.material.color.set(HALO_COLOR);
    halo.material.color.multiplyScalar(intensity);
    halo.material.needsUpdate = true;
  }
}

export function pulseCoreHalo(group, now = performance.now()) {
  if (!group) return;
  group.userData.haloPulseUntil = now + 800;
}

export function disposeCore(group) {
  if (!group) return;
  group.traverse(obj => {
    if (obj.geometry) obj.geometry.dispose();
    if (obj.material) {
      if (Array.isArray(obj.material)) obj.material.forEach(m => m.dispose());
      else obj.material.dispose();
    }
  });
}
