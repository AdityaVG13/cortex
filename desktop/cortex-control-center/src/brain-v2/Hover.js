import * as THREE from "three";

const _ndc = new THREE.Vector2();

export function createHover({ camera, instancedMesh, slotsRef, onHoverChange }) {
  const raycaster = new THREE.Raycaster();
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
    if (!pendingNDC || !instancedMesh || !slotsRef?.current) return;
    _ndc.set(pendingNDC.x, pendingNDC.y);
    pendingNDC = null;
    raycaster.setFromCamera(_ndc, camera);
    const hits = raycaster.intersectObject(instancedMesh, false);
    if (!hits.length) {
      if (lastHoveredId != null) {
        lastHoveredId = null;
        onHoverChange?.(null);
      }
      return;
    }
    const hit = hits[0];
    const slots = slotsRef.current;
    const slot = slots[hit.instanceId];
    const id = slot?.id ?? null;
    if (id !== lastHoveredId) {
      lastHoveredId = id;
      onHoverChange?.(slot || null);
    }
  }

  return { setCursor, clearCursor, tick };
}
