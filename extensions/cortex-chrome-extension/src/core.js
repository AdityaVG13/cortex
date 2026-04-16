// SPDX-License-Identifier: MIT

export const DEFAULT_CORTEX_URL = "http://127.0.0.1:7437";
export const DEFAULT_AGENT = "chrome-extension";
export const DEFAULT_RECALL_BUDGET = 280;
export const DEFAULT_RECALL_K = 6;
export const DEFAULT_TIMEOUT_MS = 8_000;

export function normalizeCortexUrl(rawValue) {
  const candidate = String(rawValue ?? "").trim();
  if (!candidate) {
    throw new Error("Cortex URL is required.");
  }
  let parsed;
  try {
    parsed = new URL(candidate);
  } catch (_error) {
    throw new Error("Cortex URL must be a valid http(s) URL.");
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    throw new Error("Cortex URL must use http or https.");
  }
  parsed.hash = "";
  parsed.search = "";
  const normalizedPath = parsed.pathname.endsWith("/")
    ? parsed.pathname.slice(0, -1)
    : parsed.pathname;
  parsed.pathname = normalizedPath || "";
  return parsed.toString().replace(/\/$/, "");
}

export function normalizeLocalCortexUrl(rawValue) {
  const normalized = normalizeCortexUrl(rawValue);
  const url = new URL(normalized);
  if (url.protocol !== "http:") {
    throw new Error(
      "This Chrome Web Store build only supports http loopback Cortex URLs."
    );
  }
  if (!isLoopbackUrl(normalized)) {
    throw new Error(
      "This Chrome Web Store build only supports local Cortex URLs (localhost or 127.0.0.1)."
    );
  }
  return normalized;
}

export function originPatternForUrl(rawValue) {
  const normalized = normalizeCortexUrl(rawValue);
  const url = new URL(normalized);
  return `${url.origin}/*`;
}

export function isLoopbackUrl(rawValue) {
  const normalized = normalizeCortexUrl(rawValue);
  const url = new URL(normalized);
  return url.hostname === "127.0.0.1" || url.hostname === "localhost";
}

export function normalizePositiveInteger(rawValue, fallback) {
  const value = Number.parseInt(String(rawValue ?? ""), 10);
  if (!Number.isFinite(value) || value < 1) {
    return fallback;
  }
  return value;
}

export function sanitizeDecision(rawValue) {
  const value = String(rawValue ?? "").trim();
  if (!value) {
    throw new Error("Memory text cannot be empty.");
  }
  return value;
}

export function normalizeAgent(rawValue) {
  const value = String(rawValue ?? "").trim();
  return value || DEFAULT_AGENT;
}
