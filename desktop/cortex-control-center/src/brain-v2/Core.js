import * as THREE from "three";
import { getHaloTexture } from "./Halo.js";
const CORE_RADIUS = 25,
  HALO_RADIUS = 80,
  HALO_COLOR = "#40e0ff",
  OUTER_COLOR = "#40e0ff",
  INNER_COLOR = "#ffd166",
  OUTER_ROT_RATE = 0.18,
  INNER_ROT_RATE = -0.32,
  HALO_BREATH_HZ = (2 * Math.PI) / 1.5,
  HALO_BREATH_AMPLITUDE = 0.08;
function wireframeIcosahedron(radius, color, opacity) {
  const base = new THREE.IcosahedronGeometry(radius, 1),
    wire = new THREE.WireframeGeometry(base);
  base.dispose();
  const material = new THREE.LineBasicMaterial({
    color,
    transparent: !0,
    opacity,
    blending: THREE.AdditiveBlending,
    depthWrite: !1,
  });
  return new THREE.LineSegments(wire, material);
}
function createCore() {
  const group = new THREE.Group();
  group.name = "brain-v2-core";
  const outer = wireframeIcosahedron(CORE_RADIUS, OUTER_COLOR, 0.85),
    inner = wireframeIcosahedron(CORE_RADIUS * 0.65, INNER_COLOR, 0.55);
  ((outer.name = "core-outer"), (inner.name = "core-inner"));
  const haloMaterial = new THREE.SpriteMaterial({
      map: getHaloTexture(),
      color: new THREE.Color(HALO_COLOR),
      transparent: !0,
      blending: THREE.AdditiveBlending,
      depthWrite: !1,
    }),
    halo = new THREE.Sprite(haloMaterial);
  return (
    (halo.name = "core-halo"),
    halo.scale.set(HALO_RADIUS * 2, HALO_RADIUS * 2, 1),
    group.add(halo),
    group.add(outer),
    group.add(inner),
    (group.userData = { haloIntensityBase: 1, haloPulseUntil: 0 }),
    group
  );
}
function tickCore(group, t, now = performance.now()) {
  if (!group) return;
  const outer = group.getObjectByName("core-outer"),
    inner = group.getObjectByName("core-inner"),
    halo = group.getObjectByName("core-halo");
  if (
    (outer && (outer.rotation.y = t * OUTER_ROT_RATE),
    inner &&
      ((inner.rotation.x = -t * INNER_ROT_RATE),
      (inner.rotation.y = t * INNER_ROT_RATE)),
    halo)
  ) {
    const breath = 1 + Math.sin(t * HALO_BREATH_HZ) * HALO_BREATH_AMPLITUDE;
    let pulse = 1;
    const remaining = group.userData.haloPulseUntil - now;
    if (remaining > 0) {
      const progress = 1 - remaining / 800;
      pulse = 1 + Math.sin(progress * Math.PI) * 0.2;
    }
    const intensity = group.userData.haloIntensityBase * breath * pulse;
    (halo.material.color.set(HALO_COLOR),
      halo.material.color.multiplyScalar(intensity),
      (halo.material.needsUpdate = !0));
  }
}
function pulseCoreHalo(group, now = performance.now()) {
  group && (group.userData.haloPulseUntil = now + 800);
}
function disposeCore(group) {
  group &&
    group.traverse((obj) => {
      (obj.geometry && obj.geometry.dispose(),
        obj.material &&
          (Array.isArray(obj.material)
            ? obj.material.forEach((m) => m.dispose())
            : obj.material.dispose()));
    });
}
export { createCore, disposeCore, pulseCoreHalo, tickCore };
