import React from "react";
import { useState } from "react";
import { buildLineGeometry } from "./sparkline-utils.js";
function Sparkline({
  data,
  width = 280,
  height = 60,
  color = "var(--cyan)",
  showArea = !0,
  showEndDot = !0,
  className = "",
}) {
  const [id] = useState(() => `spark-fill-${++sparklineCounter}`),
    geometry = buildLineGeometry(data, width, height, 8);
  if (!geometry)
    return React.createElement(
      "div",
      { className: "sparkline-empty" },
      "No data yet",
    );
  const lastPoint = geometry.points.at(-1),
    gridLines = Array.from({ length: 4 }, (_, index) => {
      const y = 8 + (index * (height - 16)) / 3;
      return React.createElement("line", {
        key: `grid-${index}`,
        x1: "8",
        x2: width - 8,
        y1: y,
        y2: y,
        className: "sparkline-grid-line",
      });
    });
  return React.createElement(
    "svg",
    {
      width,
      height,
      viewBox: `0 0 ${width} ${height}`,
      preserveAspectRatio: "xMidYMid meet",
      className: `sparkline ${className}`,
      "aria-hidden": "true",
      focusable: "false",
    },
    React.createElement(
      "defs",
      null,
      React.createElement(
        "linearGradient",
        { id, x1: "0", y1: "0", x2: "0", y2: "1" },
        React.createElement("stop", {
          offset: "0%",
          stopColor: color,
          stopOpacity: "0.22",
        }),
        React.createElement("stop", {
          offset: "70%",
          stopColor: color,
          stopOpacity: "0.08",
        }),
        React.createElement("stop", {
          offset: "100%",
          stopColor: color,
          stopOpacity: "0",
        }),
      ),
    ),
    React.createElement("g", { className: "sparkline-grid" }, gridLines),
    showArea
      ? React.createElement("path", {
          d: geometry.area,
          fill: `url(#${id})`,
          className: "sparkline-area",
        })
      : null,
    React.createElement("path", {
      d: geometry.line,
      fill: "none",
      stroke: color,
      strokeWidth: "2.25",
      strokeLinejoin: "round",
      strokeLinecap: "round",
      className: "sparkline-line",
    }),
    showEndDot && lastPoint
      ? React.createElement(
          React.Fragment,
          null,
          React.createElement("circle", {
            cx: lastPoint.x,
            cy: lastPoint.y,
            r: "6",
            fill: color,
            fillOpacity: "0.18",
          }),
          React.createElement("circle", {
            cx: lastPoint.x,
            cy: lastPoint.y,
            r: "2.75",
            fill: color,
            className: "sparkline-end-dot",
          }),
        )
      : null,
  );
}
export { Sparkline };
