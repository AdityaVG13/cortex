export let sparklineCounter = 0;

export function clampNumber(value, min, max) {
  return Math.min(Math.max(value, min), max);
}

export function buildLineGeometry(data, width, height, padding = 8) {
  if (!data || data.length < 2) return null;
  const numeric = data.map((value) => Number(value || 0));
  const max = Math.max(...numeric, 1);
  const min = Math.min(...numeric, 0);
  const range = max - min || 1;
  const innerWidth = width - padding * 2;
  const innerHeight = height - padding * 2;
  const points = numeric.map((value, index) => {
    const x = padding + (index / (numeric.length - 1)) * innerWidth;
    const y = padding + innerHeight - ((value - min) / range) * innerHeight;
    return { x, y, value };
  });
  const line = points.map((point, index) => `${index === 0 ? "M" : "L"} ${point.x} ${point.y}`).join(" ");
  const area = `${line} L ${points[points.length - 1].x} ${height - padding} L ${points[0].x} ${height - padding} Z`;
  return { points, line, area, min, max, padding };
}
