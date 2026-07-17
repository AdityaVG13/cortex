import React from "react";
import { formatSignedCompactNumber } from "../../number-format.js";
function MonteCarloProjectionChart({ projection, width = 820, height = 280 }) { if (!projection?.bandSeries?.length)
    return <div className="sparkline-empty">Not enough data for a projection yet</div>;
  const bandValues = projection.bandSeries.flatMap((point) => [point.p10, point.p25, point.p50, point.p75, point.p90]), minValue = 0,
    maxWithHeadroom = Math.max(...bandValues, 1) * 1.14, padding = { top: 16, right: 18, bottom: 30, left: 18 },
    innerWidth = width - padding.left - padding.right, innerHeight = height - padding.top - padding.bottom,
    valueRange = maxWithHeadroom - minValue || 1, toX = (index) => padding.left + (index / (projection.bandSeries.length - 1)) * innerWidth,
    toY = (value) => padding.top + innerHeight - ((value - minValue) / valueRange) * innerHeight, areaPath = (upperKey, lowerKey) => {
      const top = projection.bandSeries
          .map((point, index) => `${index === 0 ? "M" : "L"} ${toX(index)} ${toY(point[upperKey])}`)
          .join(" "), bottom = [...projection.bandSeries]
          .reverse()
          .map((point, reverseIndex) => { const index = projection.bandSeries.length - 1 - reverseIndex;
            return `L ${toX(index)} ${toY(point[lowerKey])}`;
          })
          .join(" ");
      return `${top} ${bottom} Z`; }, linePath = (key) => projection.bandSeries
        .map((point, index) => `${index === 0 ? "M" : "L"} ${toX(index)} ${toY(point[key])}`)
        .join(" "), samplePaths = projection.samples.map((sample) =>
      sample.map((value, index) => `${index === 0 ? "M" : "L"} ${toX(index)} ${toY(value)}`).join(" "), ),
    endPoint = projection.bandSeries.at(-1), summaryX = width - padding.right - 138, summaryY = padding.top + 10;
  return ( <svg
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="xMidYMid meet"
      className="projection-chart"
      role="img"
      aria-label="30-day Monte Carlo projection for cumulative savings gains" >
      <defs>
        <linearGradient id="projectionBandWide" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#4f7cff" stopOpacity="0.24" />
          <stop offset="100%" stopColor="#4f7cff" stopOpacity="0.02" />
        </linearGradient>
        <linearGradient id="projectionBandCore" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#4af2a1" stopOpacity="0.28" />
          <stop offset="100%" stopColor="#4af2a1" stopOpacity="0.04" />
        </linearGradient>
      </defs>
      <g className="projection-grid">
        {Array.from({ length: 4 }, (_, index) => { const y = padding.top + (index * innerHeight) / 3;
          return ( <line key={`y-${index}`} x1={padding.left} x2={width - padding.right} y1={y} y2={y} className="projection-grid-line" /> );
        })}
        {Array.from({ length: 6 }, (_, index) => { const x = padding.left + (index * innerWidth) / 5;
          return ( <line
              key={`x-${index}`}
              y1={padding.top}
              y2={height - padding.bottom}
              x1={x}
              x2={x}
              className="projection-grid-line projection-grid-line-vertical" /> );
        })}
        <line x1={padding.left} x2={width - padding.right} y1={toY(0)} y2={toY(0)} className="projection-baseline" />
      </g>
      <path d={areaPath("p90", "p10")} className="projection-band projection-band-wide" />
      <path d={areaPath("p75", "p25")} className="projection-band projection-band-core" />
      {samplePaths.map((path, index) => ( <path key={`sample-${index}`} d={path} className="projection-sample" style={{ animationDelay: `${index * 70}ms` }} />
      ))}
      <path d={linePath("p50")} className="projection-line" />
      {endPoint ? ( <React.Fragment>
          <circle cx={toX(projection.bandSeries.length - 1)} cy={toY(endPoint.p50)} r="9" className="projection-end-halo" />
          <circle cx={toX(projection.bandSeries.length - 1)} cy={toY(endPoint.p50)} r="3.5" className="projection-end-dot" />
          <g className="projection-summary" transform={`translate(${summaryX} ${summaryY})`}>
            <rect width="120" height="58" rx="10" className="projection-summary-panel" />
            <text x="12" y="18" className="projection-annotation projection-annotation-high">
              {"p90 "}
              {formatSignedCompactNumber(endPoint.p90)}
            </text>
            <text x="12" y="34" className="projection-annotation">
              {"p50 "}
              {formatSignedCompactNumber(endPoint.p50)}
            </text>
            <text x="12" y="50" className="projection-annotation projection-annotation-low">
              {"p10 "}
              {formatSignedCompactNumber(endPoint.p10)}
            </text>
          </g>
        </React.Fragment>
      ) : null}
      <text x={padding.left} y={height - 8} className="projection-axis-label">
        today
      </text>
      <text x={width - padding.right - 62} y={height - 8} className="projection-axis-label">
        +30d gain
      </text>
    </svg> );
}
export { MonteCarloProjectionChart };
