function trimFractionZeros(text) {
  return text.replace(/\.0+$/, "").replace(/(\.\d*[1-9])0+$/, "$1");
}

function formatScaled(value, divisor, suffix, maxFractionDigits) {
  const scaled = value / divisor;
  const decimals = scaled >= 100 ? 0 : scaled >= 10 ? 1 : maxFractionDigits;
  return `${trimFractionZeros(scaled.toFixed(decimals))}${suffix}`;
}

export function formatCompactNumber(value, maxFractionDigits = 1) {
  const numeric = Number(value || 0);
  if (!Number.isFinite(numeric)) return "0";
  const absolute = Math.abs(numeric);
  if (absolute >= 1_000_000_000_000_000) {
    return formatScaled(numeric, 1_000_000_000_000_000, "Q", maxFractionDigits);
  }
  if (absolute >= 1_000_000_000_000) {
    return formatScaled(numeric, 1_000_000_000_000, "T", maxFractionDigits);
  }
  if (absolute >= 1_000_000_000) {
    return formatScaled(numeric, 1_000_000_000, "B", maxFractionDigits);
  }
  if (absolute >= 1_000_000) {
    return formatScaled(numeric, 1_000_000, "M", maxFractionDigits);
  }
  if (absolute >= 1_000) {
    return formatScaled(numeric, 1_000, "K", maxFractionDigits);
  }
  return Math.round(numeric).toString();
}

export function formatSignedCompactNumber(value, maxFractionDigits = 1) {
  const numeric = Number(value || 0);
  if (!Number.isFinite(numeric)) return "0";
  const prefix = numeric > 0 ? "+" : numeric < 0 ? "-" : "";
  return `${prefix}${formatCompactNumber(Math.abs(numeric), maxFractionDigits)}`;
}
