export function clamp01(x) {
  return x < 0 ? 0 : x > 1 ? 1 : x;
}

export function easeOutCubic(t) {
  const x = clamp01(t);
  return 1 - Math.pow(1 - x, 3);
}

export function easeInQuad(t) {
  const x = clamp01(t);
  return x * x;
}

export function expDecay(tMs, tauMs) {
  if (!Number.isFinite(tMs) || tMs <= 0 || tauMs <= 0) return tMs <= 0 ? 1 : 0;
  return Math.exp(-tMs / tauMs);
}

export function riseDecay(tMs, riseMs, tauMs) {
  if (tMs <= 0) return 0;
  if (tMs <= riseMs) return easeOutCubic(tMs / riseMs);
  return expDecay(tMs - riseMs, tauMs);
}
