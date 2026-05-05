export const MOTION_MS = Object.freeze({
  fast: 150,
  base: 200,
  panel: 260,
  shell: 320,
  number: 600,
  numberSlow: 1000,
});

export const MOTION_EASING = Object.freeze({
  standard: "cubic-bezier(0.22, 1, 0.36, 1)",
});

export const MOTION_CSS_VARS = Object.freeze({
  base: `${MOTION_MS.base}ms`,
  panel: `${MOTION_MS.panel}ms`,
  shell: `${MOTION_MS.shell}ms`,
  ease: MOTION_EASING.standard,
});

export function easeOutCubic(progress) {
  const clamped = Math.min(Math.max(Number(progress) || 0, 0), 1);
  return 1 - (1 - clamped) ** 3;
}
