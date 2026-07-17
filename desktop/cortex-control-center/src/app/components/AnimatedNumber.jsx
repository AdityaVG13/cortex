import { useEffect, useRef, useState } from "react";
import { MOTION_MS, easeOutCubic } from "../../design/motion.js";

export function AnimatedNumber({ value, duration = MOTION_MS.number, reducedMotion = false }) {
  const [display, setDisplay] = useState(value);
  const prevRef = useRef(value);

  useEffect(() => {
    if (reducedMotion) {
      setDisplay(value);
      prevRef.current = value;
      return undefined;
    }

    const from = typeof prevRef.current === "number" ? prevRef.current : 0;
    const to = typeof value === "number" ? value : 0;
    if (from === to || typeof value !== "number") {
      setDisplay(value);
      prevRef.current = value;
      return;
    }

    let cancelled = false;
    const start = performance.now();
    const diff = to - from;

    function tick(now) {
      if (cancelled) return;
      const elapsed = now - start;
      const progress = Math.min(elapsed / duration, 1);
      const eased = easeOutCubic(progress);
      setDisplay(Math.round(from + diff * eased));
      if (progress < 1) requestAnimationFrame(tick);
    }

    requestAnimationFrame(tick);
    prevRef.current = to;
    return () => { cancelled = true; };
  }, [value, duration, reducedMotion]);

  return <>{typeof display === "number" ? display.toLocaleString() : display}</>;
}
