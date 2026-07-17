import * as THREE from "three";
function createHover({ camera, slotsRef, onHoverChange, hitRadiusScale = 2 }) {
  const raycaster = new THREE.Raycaster(),
    ndc = new THREE.Vector2(),
    _ox = { x: 0 },
    _oy = { x: 0 },
    _oz = { x: 0 };
  let pendingNDC = null,
    lastHoveredId = null;
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
    ((pendingNDC = null),
      lastHoveredId != null && ((lastHoveredId = null), onHoverChange?.(null)));
  }
  function tick() {
    if (!pendingNDC || !slotsRef?.current) return;
    (ndc.set(pendingNDC.x, pendingNDC.y),
      (pendingNDC = null),
      raycaster.setFromCamera(ndc, camera));
    const origin = raycaster.ray.origin,
      dir = raycaster.ray.direction;
    ((_ox.x = origin.x), (_oy.x = origin.y), (_oz.x = origin.z));
    const slots = slotsRef.current;
    let bestSlot = null,
      bestT = 1 / 0;
    for (let i = 0; i < slots.length; i += 1) {
      const slot = slots[i],
        dx = slot.x - _ox.x,
        dy = slot.y - _oy.x,
        dz = slot.z - _oz.x,
        proj = dir.x * dx + dir.y * dy + dir.z * dz;
      if (proj <= 0) continue;
      const px = _ox.x + proj * dir.x,
        py = _oy.x + proj * dir.y,
        pz = _oz.x + proj * dir.z,
        ddx = slot.x - px,
        ddy = slot.y - py,
        ddz = slot.z - pz,
        dist2 = ddx * ddx + ddy * ddy + ddz * ddz,
        hitR = (slot.bodyRadius || 1) * hitRadiusScale;
      dist2 <= hitR * hitR &&
        proj < bestT &&
        ((bestT = proj), (bestSlot = slot));
    }
    const id = bestSlot?.id ?? null;
    id !== lastHoveredId &&
      ((lastHoveredId = id), onHoverChange?.(bestSlot || null));
  }
  return { setCursor, clearCursor, tick };
}
export { createHover };
