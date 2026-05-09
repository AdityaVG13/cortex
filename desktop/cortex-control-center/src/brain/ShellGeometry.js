import * as THREE from "three";

export const CONSTELLATION_SHELL_NAME = "cortex-constellation-shell";

export const SHELL_OUTER_RADIUS = 220;
export const SHELL_INNER_RADIUS = 130;

const OUTER_LINE_COLOR = "#40e0ff";
const INNER_LINE_COLOR = "#ffd166";
const RETICLE_COLOR = "#40e0ff";
const CROSSHAIR_COLOR = "#f8fbff";

function makeLine(points, color, opacity, blending = THREE.AdditiveBlending) {
  const material = new THREE.LineBasicMaterial({
    color,
    transparent: true,
    opacity,
    depthWrite: false,
    blending,
  });
  const geometry = new THREE.BufferGeometry().setFromPoints(points);
  return new THREE.Line(geometry, material);
}

function makeLineSegments(geometry, color, opacity) {
  const material = new THREE.LineBasicMaterial({
    color,
    transparent: true,
    opacity,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
  });
  const segments = new THREE.LineSegments(geometry, material);
  return segments;
}

function icosphereWireframe(radius, detail, color, opacity) {
  const base = new THREE.IcosahedronGeometry(radius, detail);
  const wire = new THREE.WireframeGeometry(base);
  base.dispose();
  return makeLineSegments(wire, color, opacity);
}

function ellipseRingPoints(radiusX, radiusY, z, rotationX = 0, rotationY = 0) {
  const points = [];
  const segments = 128;
  const euler = new THREE.Euler(rotationX, rotationY, 0);
  for (let index = 0; index <= segments; index += 1) {
    const theta = (index / segments) * Math.PI * 2;
    const point = new THREE.Vector3(Math.cos(theta) * radiusX, Math.sin(theta) * radiusY, z);
    point.applyEuler(euler);
    points.push(point);
  }
  return points;
}

function reticleRing(radius, segments = 96, tickEvery = 8, tickLength = 6) {
  const ringPoints = [];
  for (let index = 0; index <= segments; index += 1) {
    const theta = (index / segments) * Math.PI * 2;
    ringPoints.push(new THREE.Vector3(Math.cos(theta) * radius, Math.sin(theta) * radius, 0));
  }
  const ring = makeLine(ringPoints, RETICLE_COLOR, 0.32);

  const tickGroup = new THREE.Group();
  for (let index = 0; index < segments; index += tickEvery) {
    const theta = (index / segments) * Math.PI * 2;
    const inner = new THREE.Vector3(Math.cos(theta) * radius, Math.sin(theta) * radius, 0);
    const outer = new THREE.Vector3(
      Math.cos(theta) * (radius + tickLength),
      Math.sin(theta) * (radius + tickLength),
      0,
    );
    tickGroup.add(makeLine([inner, outer], RETICLE_COLOR, 0.42));
  }

  const arc = new THREE.Group();
  arc.add(ring);
  arc.add(tickGroup);
  arc.rotation.x = Math.PI / 2;
  return arc;
}

function centerCrosshair(size = 14) {
  const group = new THREE.Group();
  const arms = [
    [new THREE.Vector3(-size, 0, 0), new THREE.Vector3(-size * 0.3, 0, 0)],
    [new THREE.Vector3(size * 0.3, 0, 0), new THREE.Vector3(size, 0, 0)],
    [new THREE.Vector3(0, -size, 0), new THREE.Vector3(0, -size * 0.3, 0)],
    [new THREE.Vector3(0, size * 0.3, 0), new THREE.Vector3(0, size, 0)],
  ];
  for (const [a, b] of arms) {
    group.add(makeLine([a, b], CROSSHAIR_COLOR, 0.55));
  }
  return group;
}

export function createConstellationShells() {
  const group = new THREE.Group();
  group.name = CONSTELLATION_SHELL_NAME;

  const outerShell = icosphereWireframe(SHELL_OUTER_RADIUS, 2, OUTER_LINE_COLOR, 0.28);
  const innerShell = icosphereWireframe(SHELL_INNER_RADIUS, 2, INNER_LINE_COLOR, 0.18);
  group.add(outerShell);
  group.add(innerShell);

  const rings = [
    makeLine(ellipseRingPoints(270, 170, -16, Math.PI * 0.20, 0), OUTER_LINE_COLOR, 0.14),
    makeLine(ellipseRingPoints(215, 130, 38, Math.PI * 0.34, Math.PI * 0.12), INNER_LINE_COLOR, 0.10),
    makeLine(ellipseRingPoints(185, 105, -50, Math.PI * 0.48, -Math.PI * 0.10), OUTER_LINE_COLOR, 0.11),
  ];
  for (const ring of rings) group.add(ring);

  group.add(reticleRing(290, 96, 8, 8));
  group.add(centerCrosshair(20));

  for (const child of group.children) {
    child.renderOrder = 1;
  }

  return group;
}

export function disposeConstellationShells(group) {
  if (!group) return;
  group.traverse(obj => {
    if (obj.geometry) obj.geometry.dispose();
    if (obj.material) {
      if (Array.isArray(obj.material)) obj.material.forEach(m => m.dispose());
      else obj.material.dispose();
    }
  });
}
