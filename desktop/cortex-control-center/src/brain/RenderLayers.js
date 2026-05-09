export const BRAIN_LAYERS = Object.freeze({
  BASE: 0,
  BLOOM: 1,
});

export function assignLayer(object3d, layer) {
  if (!object3d) return;
  object3d.traverse?.(child => {
    if (child.layers && typeof child.layers.set === "function") {
      child.layers.set(layer);
    }
  });
  if (object3d.layers && typeof object3d.layers.set === "function") {
    object3d.layers.set(layer);
  }
}

export function markBloom(object3d, on = true) {
  if (!object3d) return;
  object3d.userData = object3d.userData || {};
  object3d.userData.brainBloom = on;
}
