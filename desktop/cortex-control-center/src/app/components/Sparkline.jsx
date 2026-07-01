import { useState } from "react";
import { buildLineGeometry } from "./sparkline-utils.js";

export function Sparkline({
  data,
  width = 280,
  height = 60,
  color = "var(--cyan)",
  showArea = true,
  showEndDot = true,
  className = "",
}) {
  const [id] = useState(() => `spark-fill-${++sparklineCounter}`);
  const geometry = buildLineGeometry(data, width, height, 8);
  if (!geometry) return <div className="sparkline-empty">No data yet</div>;
  const lastPoint = geometry.points.at(-1);
  const gridLines = Array.from({ length: 4 }, (_, index) => {
    const y = 8 + (index * (height - 16)) / 3;
    return <line key={`grid-${index}`} x1="8" x2={width - 8} y1={y} y2={y} className="sparkline-grid-line" />;
  });

  return (
    <svg
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="xMidYMid meet"
      className={`sparkline ${className}`}
      aria-hidden="true"
      focusable="false"
    >
      <defs>
        <linearGradient id={id} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity="0.22" />
          <stop offset="70%" stopColor={color} stopOpacity="0.08" />
          <stop offset="100%" stopColor={color} stopOpacity="0" />
        </linearGradient>
      </defs>
      <g className="sparkline-grid">{gridLines}</g>
      {showArea ? <path d={geometry.area} fill={`url(#${id})`} className="sparkline-area" /> : null}
      <path d={geometry.line} fill="none" stroke={color} strokeWidth="2.25" strokeLinejoin="round" strokeLinecap="round" className="sparkline-line" />
      {showEndDot && lastPoint ? (
        <>
          <circle cx={lastPoint.x} cy={lastPoint.y} r="6" fill={color} fillOpacity="0.18" />
          <circle cx={lastPoint.x} cy={lastPoint.y} r="2.75" fill={color} className="sparkline-end-dot" />
        </>
      ) : null}
    </svg>
  );
}
