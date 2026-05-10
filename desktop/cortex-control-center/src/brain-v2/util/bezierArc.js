import * as THREE from "three";

const _mid = new THREE.Vector3();
const _control = new THREE.Vector3();

export function bezierArcPoints(from, to, segments = 16, lift = 0.18) {
  const points = [];
  _mid.set(
    (from.x + to.x) * 0.5,
    (from.y + to.y) * 0.5,
    (from.z + to.z) * 0.5,
  );
  const midLength = _mid.length();
  if (midLength < 1e-3) {
    _control.set(0, 1, 0).multiplyScalar(midLength * lift + 1);
  } else {
    _control.copy(_mid).normalize().multiplyScalar(midLength * (1 + lift));
  }
  for (let i = 0; i <= segments; i += 1) {
    const t = i / segments;
    const omt = 1 - t;
    const x = omt * omt * from.x + 2 * omt * t * _control.x + t * t * to.x;
    const y = omt * omt * from.y + 2 * omt * t * _control.y + t * t * to.y;
    const z = omt * omt * from.z + 2 * omt * t * _control.z + t * t * to.z;
    points.push(new THREE.Vector3(x, y, z));
  }
  return points;
}
