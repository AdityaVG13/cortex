import * as THREE from "three";
import { createActivationTexture, createPulseMaterial } from "./PulseShader.js";

const SEGMENTS_PER_EDGE = 16;
const ARC_HEIGHT = 0.18;

function endpointId(endpoint) {
  if (endpoint && typeof endpoint === "object") return endpoint.id;
  return endpoint;
}

function arcPoint(start, end, t) {
  const x = start.x + (end.x - start.x) * t;
  const y = start.y + (end.y - start.y) * t;
  const z = start.z + (end.z - start.z) * t;
  const arch = Math.sin(t * Math.PI) * ARC_HEIGHT;
  const mid = new THREE.Vector3(
    (start.x + end.x) * 0.5,
    (start.y + end.y) * 0.5,
    (start.z + end.z) * 0.5,
  );
  const lift = mid.clone().normalize().multiplyScalar(arch * mid.length());
  return new THREE.Vector3(x, y, z).addScaledVector(lift, 1);
}

export function buildEdgeMesh(links, nodesById, options = {}) {
  if (!links?.length) return null;

  const positions = [];
  const progresses = [];
  const edgeIds = [];
  const indices = [];
  const edgeIndex = new Map();
  let vertexOffset = 0;
  let edgeCount = 0;

  for (let i = 0; i < links.length; i += 1) {
    const link = links[i];
    const sourceId = endpointId(link.source);
    const targetId = endpointId(link.target);
    const source = nodesById.get(sourceId);
    const target = nodesById.get(targetId);
    if (!source || !target) continue;
    if (!Number.isFinite(source.x) || !Number.isFinite(target.x)) continue;

    const edgeId = edgeCount;
    edgeIndex.set(`${sourceId}>${targetId}>${link.type || "semantic"}`, edgeId);

    for (let s = 0; s <= SEGMENTS_PER_EDGE; s += 1) {
      const t = s / SEGMENTS_PER_EDGE;
      const point = arcPoint(source, target, t);
      positions.push(point.x, point.y, point.z);
      progresses.push(t);
      edgeIds.push(edgeId);
    }

    for (let s = 0; s < SEGMENTS_PER_EDGE; s += 1) {
      indices.push(vertexOffset + s, vertexOffset + s + 1);
    }

    vertexOffset += SEGMENTS_PER_EDGE + 1;
    edgeCount += 1;
  }

  if (edgeCount === 0) return null;

  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geometry.setAttribute("aProgress", new THREE.Float32BufferAttribute(progresses, 1));
  geometry.setAttribute("aEdgeId", new THREE.Float32BufferAttribute(edgeIds, 1));
  geometry.setIndex(indices);

  const { texture: activationTexture, data: activationData } = createActivationTexture(edgeCount);
  const material = createPulseMaterial({
    activationTexture,
    activationCount: edgeCount,
    baseColor: options.baseColor,
    pulseColor: options.pulseColor,
  });

  const mesh = new THREE.LineSegments(geometry, material);
  mesh.frustumCulled = false;
  mesh.renderOrder = 2;
  mesh.userData = {
    brainEdgeMesh: true,
    edgeIndex,
    edgeCount,
    activationTexture,
    activationData,
  };

  return mesh;
}

export function disposeEdgeMesh(mesh) {
  if (!mesh) return;
  if (mesh.parent) mesh.parent.remove(mesh);
  mesh.geometry?.dispose();
  if (mesh.material) {
    if (Array.isArray(mesh.material)) mesh.material.forEach(m => m.dispose());
    else mesh.material.dispose();
  }
  mesh.userData.activationTexture?.dispose?.();
  mesh.userData.edgeIndex?.clear?.();
}

export function tickEdgeMaterialTime(mesh, time) {
  if (!mesh?.material?.uniforms?.uTime) return;
  mesh.material.uniforms.uTime.value = time;
}
