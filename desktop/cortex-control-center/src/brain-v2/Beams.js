import * as THREE from "three";
import { bezierArcPoints } from "./util/bezierArc.js";
import { createActivationTexture, createPulseMaterial } from "./PulseShader.js";
import { riseDecay } from "./util/easing.js";

const POOL_SIZE = 64;
const SEGMENTS = 16;
const VERTS_PER_BEAM = SEGMENTS + 1;
const RISE_MS = 80;
const TAU_MS = 280;
const DEFAULT_LIFE_MS = 600;

const _from = new THREE.Vector3();
const _to = new THREE.Vector3();

export function createBeams({ scene }) {
  const totalVerts = POOL_SIZE * VERTS_PER_BEAM;
  const positions = new Float32Array(totalVerts * 3);
  const progresses = new Float32Array(totalVerts);
  const beamIds = new Float32Array(totalVerts);
  const colors = new Float32Array(totalVerts * 3);
  const indices = new Uint16Array(POOL_SIZE * SEGMENTS * 2);

  for (let beam = 0; beam < POOL_SIZE; beam += 1) {
    for (let s = 0; s <= SEGMENTS; s += 1) {
      const v = beam * VERTS_PER_BEAM + s;
      progresses[v] = s / SEGMENTS;
      beamIds[v] = beam;
    }
    for (let s = 0; s < SEGMENTS; s += 1) {
      const i = (beam * SEGMENTS + s) * 2;
      indices[i] = beam * VERTS_PER_BEAM + s;
      indices[i + 1] = beam * VERTS_PER_BEAM + s + 1;
    }
  }

  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  geometry.setAttribute("aProgress", new THREE.BufferAttribute(progresses, 1));
  geometry.setAttribute("aBeamId", new THREE.BufferAttribute(beamIds, 1));
  geometry.setAttribute("aColor", new THREE.BufferAttribute(colors, 3));
  geometry.setIndex(new THREE.BufferAttribute(indices, 1));

  const { texture, data } = createActivationTexture(POOL_SIZE);
  const material = createPulseMaterial({
    activationTexture: texture,
    beamCount: POOL_SIZE,
  });

  const mesh = new THREE.LineSegments(geometry, material);
  mesh.frustumCulled = false;
  mesh.renderOrder = 1;
  mesh.name = "brain-v2-beams";
  scene.add(mesh);

  const slots = Array.from({ length: POOL_SIZE }, () => ({
    active: false,
    startTime: 0,
    lifeMs: DEFAULT_LIFE_MS,
  }));

  function findSlot(now) {
    for (let i = 0; i < POOL_SIZE; i += 1) {
      if (!slots[i].active) return i;
    }
    let oldestIdx = 0;
    let oldestTime = Infinity;
    for (let i = 0; i < POOL_SIZE; i += 1) {
      if (slots[i].startTime < oldestTime) {
        oldestTime = slots[i].startTime;
        oldestIdx = i;
      }
    }
    return oldestIdx;
  }

  function fire({ from, to, color = "#22d3ee", life = DEFAULT_LIFE_MS, now = performance.now() }) {
    if (!from || !to) return -1;
    _from.set(from.x, from.y, from.z);
    _to.set(to.x, to.y, to.z);
    const arc = bezierArcPoints(_from, _to, SEGMENTS, 0.18);
    const slot = findSlot(now);
    const baseVert = slot * VERTS_PER_BEAM;
    const c = new THREE.Color(color);
    for (let i = 0; i < arc.length; i += 1) {
      const v = baseVert + i;
      positions[v * 3 + 0] = arc[i].x;
      positions[v * 3 + 1] = arc[i].y;
      positions[v * 3 + 2] = arc[i].z;
      colors[v * 3 + 0] = c.r;
      colors[v * 3 + 1] = c.g;
      colors[v * 3 + 2] = c.b;
    }
    geometry.attributes.position.needsUpdate = true;
    geometry.attributes.aColor.needsUpdate = true;
    slots[slot].active = true;
    slots[slot].startTime = now;
    slots[slot].lifeMs = life;
    data[slot] = 0;
    texture.needsUpdate = true;
    return slot;
  }

  function tick(now = performance.now()) {
    let dirty = false;
    for (let i = 0; i < POOL_SIZE; i += 1) {
      const slot = slots[i];
      if (!slot.active) {
        if (data[i] !== 0) {
          data[i] = 0;
          dirty = true;
        }
        continue;
      }
      const t = now - slot.startTime;
      if (t >= slot.lifeMs) {
        slot.active = false;
        data[i] = 0;
        dirty = true;
        continue;
      }
      const value = riseDecay(t, RISE_MS, TAU_MS);
      data[i] = Math.min(1, value);
      dirty = true;
    }
    if (dirty) texture.needsUpdate = true;
    material.uniforms.uTime.value = (now * 0.001) % 1000;
  }

  function activeCount() {
    return slots.reduce((n, s) => n + (s.active ? 1 : 0), 0);
  }

  function dispose() {
    scene.remove(mesh);
    geometry.dispose();
    material.dispose();
    texture.dispose();
  }

  return { mesh, fire, tick, activeCount, dispose };
}
