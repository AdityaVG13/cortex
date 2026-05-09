import * as THREE from "three";

let cached = null;

export function getHaloTexture() {
  if (cached) return cached;
  const size = 64;
  const canvas = typeof document !== "undefined"
    ? document.createElement("canvas")
    : null;
  if (!canvas) {
    const data = new Uint8Array(size * size * 4);
    cached = new THREE.DataTexture(data, size, size, THREE.RGBAFormat, THREE.UnsignedByteType);
    cached.needsUpdate = true;
    return cached;
  }
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d");
  const grad = ctx.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2);
  grad.addColorStop(0, "rgba(255,255,255,1)");
  grad.addColorStop(0.35, "rgba(255,255,255,0.55)");
  grad.addColorStop(0.7, "rgba(255,255,255,0.12)");
  grad.addColorStop(1, "rgba(255,255,255,0)");
  ctx.fillStyle = grad;
  ctx.fillRect(0, 0, size, size);

  const texture = new THREE.CanvasTexture(canvas);
  texture.minFilter = THREE.LinearFilter;
  texture.magFilter = THREE.LinearFilter;
  texture.needsUpdate = true;
  cached = texture;
  return cached;
}
