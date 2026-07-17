import { CURRENCY_OPTIONS } from "../../constants.js";
function formatDaemonEndpoint(cortexBase) { try { const url = new URL(cortexBase), port = url.port || (url.protocol === "https:" ? "443" : "80");
    return `${url.hostname}:${port}`;
  } catch { return `127.0.0.1:${DEFAULT_CORTEX_PORT}`;
  }
}
function feedKindLabel(kind) { return FEED_KIND_LABEL[kind] || kind || "Unknown";
}
function getOsReducedMotionPreference() { return typeof window > "u" || typeof window.matchMedia != "function"
    ? !1
    : window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}
function normalizeCurrencyCode(raw) { const candidate = String(raw || "")
    .trim()
    .toUpperCase();
  return CURRENCY_OPTIONS.includes(candidate) ? candidate : "USD";
}
function priorityRank(priority) { return { critical: 4, high: 3, medium: 2, low: 1 }[priority] || 0;
}
export { feedKindLabel, formatDaemonEndpoint, getOsReducedMotionPreference, normalizeCurrencyCode, priorityRank };
