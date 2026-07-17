const MOTION_MS = Object.freeze({ fast: 150, base: 200, panel: 340, shell: 320, number: 600, numberSlow: 1e3, }),
  MOTION_EASING = Object.freeze({ standard: "cubic-bezier(0.22, 1, 0.36, 1)" }), MOTION_CSS_VARS = Object.freeze({
    base: `${MOTION_MS.base}ms`, panel: `${MOTION_MS.panel}ms`, shell: `${MOTION_MS.shell}ms`, ease: MOTION_EASING.standard, });
function easeOutCubic(progress) { return 1 - (1 - Math.min(Math.max(Number(progress) || 0, 0), 1)) ** 3;
}
export { MOTION_MS, easeOutCubic };
