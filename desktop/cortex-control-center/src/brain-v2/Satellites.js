import * as THREE from "three";
import { getHaloTexture } from "./Halo.js";
import {
  paletteForCluster,
  DECISION_COLOR,
  LOOSE_COLOR,
  SELECTED_COLOR,
} from "./ClusterPalette.js";
const DEFAULT_SLOT_BUDGET = 200,
  HALO_TO_BODY = 3,
  BOB_FREQ = (2 * Math.PI) / 4,
  BOB_AMPLITUDE = 0.02,
  _matrix = new THREE.Matrix4(),
  _color = new THREE.Color(),
  _quat = new THREE.Quaternion(),
  _scale = new THREE.Vector3(),
  _pos = new THREE.Vector3();
function colorForSlot(slot) {
  return slot.selected
    ? SELECTED_COLOR
    : slot.tier === "decision"
      ? DECISION_COLOR
      : slot.tier === "loose"
        ? LOOSE_COLOR
        : slot.coldStart
          ? LOOSE_COLOR
          : slot.centroidKey
            ? paletteForCluster(slot.centroidKey).color
            : LOOSE_COLOR;
}
function createSatellites({ scene, slotBudget = DEFAULT_SLOT_BUDGET }) {
  const capacity = Math.max(1, Math.floor(slotBudget)),
    bodyGeometry = new THREE.SphereGeometry(1, 12, 12),
    bodyMaterial = new THREE.MeshBasicMaterial({
      color: 16777215,
      transparent: !0,
      opacity: 0.95,
      blending: THREE.AdditiveBlending,
      depthWrite: !1,
    }),
    bodies = new THREE.InstancedMesh(bodyGeometry, bodyMaterial, capacity);
  (bodies.instanceMatrix.setUsage(THREE.DynamicDrawUsage),
    (bodies.count = 0),
    (bodies.name = "brain-v2-satellite-bodies"));
  const haloMap = getHaloTexture(),
    haloGeometry = new THREE.PlaneGeometry(1, 1),
    haloMaterial = new THREE.MeshBasicMaterial({
      map: haloMap,
      color: 16777215,
      transparent: !0,
      opacity: 0.85,
      blending: THREE.AdditiveBlending,
      depthWrite: !1,
      side: THREE.DoubleSide,
    }),
    halos = new THREE.InstancedMesh(haloGeometry, haloMaterial, capacity);
  (halos.instanceMatrix.setUsage(THREE.DynamicDrawUsage),
    (halos.count = 0),
    (halos.name = "brain-v2-satellite-halos"),
    scene.add(halos),
    scene.add(bodies));
  let slots = [],
    selectedId = null;
  function setData(payload) {
    const flat = [];
    for (const d of payload.decisions || []) flat.push({ ...d });
    for (const c of payload.clusters || []) flat.push({ ...c });
    for (const m of payload.looseMemories || []) flat.push({ ...m });
    ((slots = flat
      .slice(0, capacity)
      .map((entry) => ({
        ...entry,
        phase: Math.random() * Math.PI * 2,
        pulseUntil: 0,
        selected: selectedId != null && selectedId === entry.id,
      }))),
      (bodies.count = slots.length),
      (halos.count = slots.length),
      writeAll(),
      bodies.computeBoundingSphere(),
      halos.computeBoundingSphere());
  }
  function writeAll(now = performance.now()) {
    for (let i = 0; i < slots.length; i += 1) writeSlot(i, now);
    ((bodies.instanceMatrix.needsUpdate = !0),
      (halos.instanceMatrix.needsUpdate = !0),
      bodies.instanceColor && (bodies.instanceColor.needsUpdate = !0),
      halos.instanceColor && (halos.instanceColor.needsUpdate = !0));
  }
  function writeSlot(index, now) {
    const slot = slots[index];
    if (!slot) return;
    const t = now * 0.001 + slot.phase,
      bob = 1 + Math.sin(t * BOB_FREQ) * BOB_AMPLITUDE;
    _pos.set(slot.x * bob, slot.y * bob, slot.z * bob);
    const pulseScale =
        slot.pulseUntil > now ? 1 + ((slot.pulseUntil - now) / 600) * 0.4 : 1,
      bodySize = slot.bodyRadius * pulseScale * (slot.selected ? 1.4 : 1);
    (_scale.set(bodySize, bodySize, bodySize),
      _quat.identity(),
      _matrix.compose(_pos, _quat, _scale),
      bodies.setMatrixAt(index, _matrix));
    const haloSize = bodySize * HALO_TO_BODY * (slot.selected ? 1.4 : 1);
    (_scale.set(haloSize, haloSize, 1),
      _matrix.compose(_pos, _quat, _scale),
      halos.setMatrixAt(index, _matrix),
      _color.copy(colorForSlot(slot)),
      bodies.setColorAt(index, _color),
      halos.setColorAt(index, _color));
  }
  function tick(t, now = performance.now()) {
    if (!slots.length) return;
    for (let i = 0; i < slots.length; i += 1) writeSlot(i, now);
    ((bodies.instanceMatrix.needsUpdate = !0),
      (halos.instanceMatrix.needsUpdate = !0),
      bodies.instanceColor && (bodies.instanceColor.needsUpdate = !0),
      halos.instanceColor && (halos.instanceColor.needsUpdate = !0));
    const camera = scene._camera || null;
    camera && halos.lookAt(camera.position);
  }
  function pulseSlot(id, now = performance.now()) {
    const idx = slots.findIndex((s) => s.id === id);
    idx < 0 || (slots[idx].pulseUntil = now + 600);
  }
  function setSelected(id) {
    selectedId = id;
    for (const slot of slots) slot.selected = slot.id === id;
    writeAll();
  }
  function getSlotById(id) {
    return slots.find((s) => s.id === id) || null;
  }
  function getSlotPositions() {
    return slots.map((s) => ({ id: s.id, x: s.x, y: s.y, z: s.z }));
  }
  function getAllIds() {
    return slots.map((s) => s.id);
  }
  function dispose() {
    (scene.remove(bodies),
      scene.remove(halos),
      bodyGeometry.dispose(),
      bodyMaterial.dispose(),
      haloGeometry.dispose(),
      haloMaterial.dispose());
  }
  return (
    (bodies.instanceColor = new THREE.InstancedBufferAttribute(
      new Float32Array(capacity * 3),
      3,
    )),
    (halos.instanceColor = new THREE.InstancedBufferAttribute(
      new Float32Array(capacity * 3),
      3,
    )),
    {
      bodies,
      halos,
      setData,
      tick,
      pulseSlot,
      setSelected,
      getSlotById,
      getSlotPositions,
      getAllIds,
      dispose,
    }
  );
}
export { createSatellites };
