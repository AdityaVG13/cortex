function trimFractionZeros(text) {
  return text.replace(/\.0+$/, "").replace(/(\.\d*[1-9])0+$/, "$1");
}

const COMPACT_SUFFIXES = ["", "K", "M", "B", "T", "Q", "Qi", "Sx", "Sp", "Oc", "No", "Dc"];

function normalizedFractionDigits(maxFractionDigits) {
  const digits = Number(maxFractionDigits);
  if (!Number.isFinite(digits)) return 1;
  return Math.min(2, Math.max(0, Math.trunc(digits)));
}

function formatScaled(scaled, suffix, maxFractionDigits) {
  const decimals = scaled >= 100 ? 0 : scaled >= 10 ? 1 : maxFractionDigits;
  return `${trimFractionZeros(scaled.toFixed(decimals))}${suffix}`;
}

export function formatCompactNumber(value, maxFractionDigits = 1) {
  const numeric = Number(value || 0);
  if (!Number.isFinite(numeric)) return "0";
  const absolute = Math.abs(numeric);
  if (absolute < 1_000) {
    return Math.round(numeric).toString();
  }

  const digits = normalizedFractionDigits(maxFractionDigits);
  let scaled = absolute;
  let suffixIndex = 0;
  while (scaled >= 1_000 && suffixIndex < COMPACT_SUFFIXES.length - 1) {
    scaled /= 1_000;
    suffixIndex += 1;
  }

  let overflowSteps = 0;
  while (scaled >= 1_000) {
    scaled /= 1_000;
    overflowSteps += 1;
  }

  const suffix = overflowSteps > 0
    ? `${COMPACT_SUFFIXES[COMPACT_SUFFIXES.length - 1]}+${overflowSteps}`
    : COMPACT_SUFFIXES[suffixIndex];
  const compact = formatScaled(scaled, suffix, digits);
  if (numeric < 0) {
    return `-${compact}`;
  }

  return compact;
}

export function formatSignedCompactNumber(value, maxFractionDigits = 1) {
  const numeric = Number(value || 0);
  if (!Number.isFinite(numeric)) return "0";
  const prefix = numeric > 0 ? "+" : numeric < 0 ? "-" : "";
  return `${prefix}${formatCompactNumber(Math.abs(numeric), maxFractionDigits)}`;
}
