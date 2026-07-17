import * as THREE from "three";
import { fnv1a32 } from "./util/fnv1a.js";
const GOLDEN_ANGLE = 137.508, SATURATION = 0.7, LIGHTNESS = 0.58, cache = new Map(),
  DECISION_COLOR = new THREE.Color("#ffd166"), LOOSE_COLOR = new THREE.Color("#22d3ee"), SELECTED_COLOR = new THREE.Color("#ffffff");
function paletteForCluster(centroidBytes) { const seed = fnv1a32(centroidBytes), cached = cache.get(seed);
  if (cached) return cached;
  const hue = (((seed >>> 0) * GOLDEN_ANGLE) / 4294967296) % 360, color = new THREE.Color().setHSL(hue / 360, SATURATION, LIGHTNESS),
    entry = { seed, hue, saturation: SATURATION, lightness: LIGHTNESS, color };
  return (cache.set(seed, entry), entry);
}
function paletteForId(id) { return paletteForCluster(`cluster-${id}`);
}
function clearPaletteCache() { cache.clear();
}
export { DECISION_COLOR, LOOSE_COLOR, SELECTED_COLOR, paletteForCluster };
