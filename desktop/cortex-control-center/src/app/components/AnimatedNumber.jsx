import React from "react";
import { useEffect, useRef, useState } from "react";
import { MOTION_MS, easeOutCubic } from "../../design/motion.js";
function AnimatedNumber({ value, duration = MOTION_MS.number, reducedMotion = !1 }) {
  const [display, setDisplay] = useState(value),
    prevRef = useRef(value);
  return (
    useEffect(() => {
      if (reducedMotion) {
        (setDisplay(value), (prevRef.current = value));
        return;
      }
      const from = typeof prevRef.current == "number" ? prevRef.current : 0,
        to = typeof value == "number" ? value : 0;
      if (from === to || typeof value != "number") {
        (setDisplay(value), (prevRef.current = value));
        return;
      }
      let cancelled = !1;
      const start = performance.now(),
        diff = to - from;
      function tick(now) {
        if (cancelled) return;
        const elapsed = now - start,
          progress = Math.min(elapsed / duration, 1),
          eased = easeOutCubic(progress);
        (setDisplay(Math.round(from + diff * eased)), progress < 1 && requestAnimationFrame(tick));
      }
      return (
        requestAnimationFrame(tick),
        (prevRef.current = to),
        () => {
          cancelled = !0;
        }
      );
    }, [value, duration, reducedMotion]),
    React.createElement(React.Fragment, null, typeof display == "number" ? display.toLocaleString() : display)
  );
}
export { AnimatedNumber };
