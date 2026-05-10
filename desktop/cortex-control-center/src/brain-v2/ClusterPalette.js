import * as THREE from "three";
import { fnv1a32 } from "./util/fnv1a.js";

const GOLDEN_ANGLE = 137.508;
const SATURATION = 0.70;
const LIGHTNESS = 0.58;
const cache = new Map();

export const DECISION_COLOR = new THREE.Color("#ffd166");
export const LOOSE_COLOR = new THREE.Color("#22d3ee");
export const SELECTED_COLOR = new THREE.Color("#ffffff");

export function paletteForCluster(centroidBytes) {
  const seed = fnv1a32(centroidBytes);
  const cached = cache.get(seed);
  if (cached) return cached;
  const hue = ((seed >>> 0) * GOLDEN_ANGLE / 0x100000000) % 360;
  const color = new THREE.Color().setHSL(hue / 360, SATURATION, LIGHTNESS);
  const entry = { seed, hue, saturation: SATURATION, lightness: LIGHTNESS, color };
  cache.set(seed, entry);
  return entry;
}

export function paletteForId(id) {
  return paletteForCluster(`cluster-${id}`);
}

export function clearPaletteCache() {
  cache.clear();
}
