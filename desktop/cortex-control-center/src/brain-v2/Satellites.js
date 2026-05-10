import * as THREE from "three";
import { getHaloTexture } from "./Halo.js";
import { paletteForCluster, DECISION_COLOR, LOOSE_COLOR, SELECTED_COLOR } from "./ClusterPalette.js";

const SLOT_BUDGET = 200;
const HALO_TO_BODY = 3.0;
const BOB_FREQ = 2 * Math.PI / 4.0;
const BOB_AMPLITUDE = 0.02;

const _matrix = new THREE.Matrix4();
const _color = new THREE.Color();
const _quat = new THREE.Quaternion();
const _scale = new THREE.Vector3();
const _pos = new THREE.Vector3();

function colorForSlot(slot) {
  if (slot.selected) return SELECTED_COLOR;
  if (slot.tier === "decision") return DECISION_COLOR;
  if (slot.tier === "loose") return LOOSE_COLOR;
  if (slot.coldStart) return LOOSE_COLOR;
  if (slot.centroidKey) return paletteForCluster(slot.centroidKey).color;
  return LOOSE_COLOR;
}

export function createSatellites({ scene }) {
  const bodyGeometry = new THREE.SphereGeometry(1, 12, 12);
  const bodyMaterial = new THREE.MeshBasicMaterial({
    color: 0xffffff,
    transparent: true,
    opacity: 0.95,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
  });
  const bodies = new THREE.InstancedMesh(bodyGeometry, bodyMaterial, SLOT_BUDGET);
  bodies.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
  bodies.count = 0;
  bodies.name = "brain-v2-satellite-bodies";

  const haloMap = getHaloTexture();
  const haloGeometry = new THREE.PlaneGeometry(1, 1);
  const haloMaterial = new THREE.MeshBasicMaterial({
    map: haloMap,
    color: 0xffffff,
    transparent: true,
    opacity: 0.85,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
    side: THREE.DoubleSide,
  });
  const halos = new THREE.InstancedMesh(haloGeometry, haloMaterial, SLOT_BUDGET);
  halos.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
  halos.count = 0;
  halos.name = "brain-v2-satellite-halos";

  scene.add(halos);
  scene.add(bodies);

  let slots = [];
  let selectedId = null;

  function setData(payload) {
    const flat = [];
    for (const d of payload.decisions || []) flat.push({ ...d });
    for (const c of payload.clusters || []) flat.push({ ...c });
    for (const m of payload.looseMemories || []) flat.push({ ...m });

    const next = flat.slice(0, SLOT_BUDGET).map((entry) => ({
      ...entry,
      phase: Math.random() * Math.PI * 2,
      pulseUntil: 0,
      selected: selectedId != null && selectedId === entry.id,
    }));

    slots = next;
    bodies.count = slots.length;
    halos.count = slots.length;
    writeAll();
    bodies.computeBoundingSphere();
    halos.computeBoundingSphere();
  }

  function writeAll(now = performance.now()) {
    for (let i = 0; i < slots.length; i += 1) writeSlot(i, now);
    bodies.instanceMatrix.needsUpdate = true;
    halos.instanceMatrix.needsUpdate = true;
    if (bodies.instanceColor) bodies.instanceColor.needsUpdate = true;
    if (halos.instanceColor) halos.instanceColor.needsUpdate = true;
  }

  function writeSlot(index, now) {
    const slot = slots[index];
    if (!slot) return;
    const t = now * 0.001 + slot.phase;
    const bob = 1 + Math.sin(t * BOB_FREQ) * BOB_AMPLITUDE;

    _pos.set(slot.x * bob, slot.y * bob, slot.z * bob);

    const pulseScale = slot.pulseUntil > now
      ? 1 + ((slot.pulseUntil - now) / 600) * 0.4
      : 1;
    const bodySize = slot.bodyRadius * pulseScale * (slot.selected ? 1.4 : 1);
    _scale.set(bodySize, bodySize, bodySize);
    _quat.identity();
    _matrix.compose(_pos, _quat, _scale);
    bodies.setMatrixAt(index, _matrix);

    const haloSize = bodySize * HALO_TO_BODY * (slot.selected ? 1.4 : 1);
    _scale.set(haloSize, haloSize, 1);
    _matrix.compose(_pos, _quat, _scale);
    halos.setMatrixAt(index, _matrix);

    _color.copy(colorForSlot(slot));
    bodies.setColorAt(index, _color);
    halos.setColorAt(index, _color);
  }

  function tick(t, now = performance.now()) {
    if (!slots.length) return;
    for (let i = 0; i < slots.length; i += 1) writeSlot(i, now);
    bodies.instanceMatrix.needsUpdate = true;
    halos.instanceMatrix.needsUpdate = true;
    if (bodies.instanceColor) bodies.instanceColor.needsUpdate = true;
    if (halos.instanceColor) halos.instanceColor.needsUpdate = true;

    const camera = scene._camera || null;
    if (camera) {
      halos.lookAt(camera.position);
    }
  }

  function pulseSlot(id, now = performance.now()) {
    const idx = slots.findIndex(s => s.id === id);
    if (idx < 0) return;
    slots[idx].pulseUntil = now + 600;
  }

  function setSelected(id) {
    selectedId = id;
    for (const slot of slots) slot.selected = slot.id === id;
  }

  function getSlotById(id) {
    return slots.find(s => s.id === id) || null;
  }

  function getSlotPositions() {
    return slots.map(s => ({ id: s.id, x: s.x, y: s.y, z: s.z }));
  }

  function getAllIds() {
    return slots.map(s => s.id);
  }

  function dispose() {
    scene.remove(bodies);
    scene.remove(halos);
    bodyGeometry.dispose();
    bodyMaterial.dispose();
    haloGeometry.dispose();
    haloMaterial.dispose();
  }

  // Initialize per-instance color attribute.
  bodies.instanceColor = new THREE.InstancedBufferAttribute(new Float32Array(SLOT_BUDGET * 3), 3);
  halos.instanceColor = new THREE.InstancedBufferAttribute(new Float32Array(SLOT_BUDGET * 3), 3);

  return {
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
  };
}
