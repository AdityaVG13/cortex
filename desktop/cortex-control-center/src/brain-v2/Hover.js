import * as THREE from "three";

export function createHover({ camera, slotsRef, onHoverChange, hitRadiusScale = 2.0 }) {
  const raycaster = new THREE.Raycaster();
  const ndc = new THREE.Vector2();
  const _ox = { x: 0 };
  const _oy = { x: 0 };
  const _oz = { x: 0 };
  let pendingNDC = null;
  let lastHoveredId = null;

  function setCursor(clientX, clientY, rect) {
    if (!rect || rect.width <= 0 || rect.height <= 0) {
      pendingNDC = null;
      return;
    }
    pendingNDC = {
      x: ((clientX - rect.left) / rect.width) * 2 - 1,
      y: -((clientY - rect.top) / rect.height) * 2 + 1,
    };
  }

  function clearCursor() {
    pendingNDC = null;
    if (lastHoveredId != null) {
      lastHoveredId = null;
      onHoverChange?.(null);
    }
  }

  function tick() {
    if (!pendingNDC || !slotsRef?.current) return;
    ndc.set(pendingNDC.x, pendingNDC.y);
    pendingNDC = null;
    raycaster.setFromCamera(ndc, camera);
    const origin = raycaster.ray.origin;
    const dir = raycaster.ray.direction;
    _ox.x = origin.x;
    _oy.x = origin.y;
    _oz.x = origin.z;

    const slots = slotsRef.current;
    let bestSlot = null;
    let bestT = Infinity;
    for (let i = 0; i < slots.length; i += 1) {
      const slot = slots[i];
      const dx = slot.x - _ox.x;
      const dy = slot.y - _oy.x;
      const dz = slot.z - _oz.x;
      const proj = dir.x * dx + dir.y * dy + dir.z * dz;
      if (proj <= 0) continue;
      const px = _ox.x + proj * dir.x;
      const py = _oy.x + proj * dir.y;
      const pz = _oz.x + proj * dir.z;
      const ddx = slot.x - px;
      const ddy = slot.y - py;
      const ddz = slot.z - pz;
      const dist2 = ddx * ddx + ddy * ddy + ddz * ddz;
      const hitR = (slot.bodyRadius || 1) * hitRadiusScale;
      if (dist2 <= hitR * hitR && proj < bestT) {
        bestT = proj;
        bestSlot = slot;
      }
    }

    const id = bestSlot?.id ?? null;
    if (id !== lastHoveredId) {
      lastHoveredId = id;
      onHoverChange?.(bestSlot || null);
    }
  }

  return { setCursor, clearCursor, tick };
}
