import { CURRENCY_OPTIONS } from "../../constants.js";
export function formatDaemonEndpoint(cortexBase) {
  try {
    const url = new URL(cortexBase);
    const port = url.port || (url.protocol === "https:" ? "443" : "80");
    return `${url.hostname}:${port}`;
  } catch {
    return `127.0.0.1:${DEFAULT_CORTEX_PORT}`;
  }
}

export function feedKindLabel(kind) {
  return FEED_KIND_LABEL[kind] || kind || "Unknown";
}

export function getOsReducedMotionPreference() {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return false;
  }
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

export function normalizeCurrencyCode(raw) {
  const candidate = String(raw || "").trim().toUpperCase();
  return CURRENCY_OPTIONS.includes(candidate) ? candidate : "USD";
}

export function priorityRank(priority) {
  const map = { critical: 4, high: 3, medium: 2, low: 1 };
  return map[priority] || 0;
}
