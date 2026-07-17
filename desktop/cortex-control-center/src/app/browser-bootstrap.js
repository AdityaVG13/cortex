import {
  CORTEX_AUTH_STORAGE_KEY, CORTEX_BASE_STORAGE_KEY, CORTEX_PANEL_STORAGE_KEY, DEFAULT_CORTEX_BASE,
  LEGACY_CORTEX_AUTH_STORAGE_KEYS, PANEL_SEQUENCE_KEYS, } from "./constants.js";

export function clearLegacyBrowserAuthTokens() {
  if (typeof window === "undefined") return;
  try { for (const key of LEGACY_CORTEX_AUTH_STORAGE_KEYS) { window.sessionStorage.removeItem(key);
      window.localStorage.removeItem(key);
    }
  } catch {
    // Ignore storage failures in restricted browser contexts.
  }
}

export function readPersistedBrowserAuthToken() {
  if (typeof window === "undefined") return "";
  try { const sessionToken = window.sessionStorage.getItem(CORTEX_AUTH_STORAGE_KEY) || "";
    if (sessionToken) return sessionToken;

    for (const key of LEGACY_CORTEX_AUTH_STORAGE_KEYS) { const legacySessionToken = window.sessionStorage.getItem(key) || "";
      if (legacySessionToken) { window.sessionStorage.setItem(CORTEX_AUTH_STORAGE_KEY, legacySessionToken);
        clearLegacyBrowserAuthTokens();
        return legacySessionToken;
      }
    }

    const legacyToken = window.localStorage.getItem(CORTEX_AUTH_STORAGE_KEY) || "";
    if (legacyToken) { window.sessionStorage.setItem(CORTEX_AUTH_STORAGE_KEY, legacyToken);
      window.localStorage.removeItem(CORTEX_AUTH_STORAGE_KEY);
      clearLegacyBrowserAuthTokens();
      return legacyToken;
    }

    for (const key of LEGACY_CORTEX_AUTH_STORAGE_KEYS) { const legacyLocalToken = window.localStorage.getItem(key) || "";
      if (legacyLocalToken) { window.sessionStorage.setItem(CORTEX_AUTH_STORAGE_KEY, legacyLocalToken);
        clearLegacyBrowserAuthTokens();
        return legacyLocalToken;
      }
    }
  } catch { return "";
  }
  return "";
}

export function readBrowserBootstrap() {
  if (typeof window === "undefined") { return { cortexBase: "", authToken: "", panel: "overview" };
  }

  const params = new URLSearchParams(window.location.search);
  let storedPanel = "";
  let storedBase = DEFAULT_CORTEX_BASE;
  try { storedPanel = window.localStorage.getItem(CORTEX_PANEL_STORAGE_KEY) || "";
    storedBase = window.localStorage.getItem(CORTEX_BASE_STORAGE_KEY) || DEFAULT_CORTEX_BASE;
  } catch {
    // Ignore storage failures in restricted browser contexts.
  }

  const requestedPanel = params.get("panel") || storedPanel || "";
  const panel = PANEL_SEQUENCE_KEYS.has(requestedPanel) ? requestedPanel : "overview";
  const cortexBase = params.get("cortexBase") || storedBase || DEFAULT_CORTEX_BASE;
  const authTokenFromParams = params.get("authToken") || "";
  const authToken = authTokenFromParams || readPersistedBrowserAuthToken();

  try { if (params.get("panel")) { window.localStorage.setItem(CORTEX_PANEL_STORAGE_KEY, panel);
    }
    if (params.get("cortexBase")) { window.localStorage.setItem(CORTEX_BASE_STORAGE_KEY, cortexBase);
    }
  } catch {
    // Ignore storage failures in restricted browser contexts.
  }
  if (authTokenFromParams) { try { window.sessionStorage.setItem(CORTEX_AUTH_STORAGE_KEY, authToken);
      window.localStorage.removeItem(CORTEX_AUTH_STORAGE_KEY);
    } catch {
      // Ignore storage failures in restricted browser contexts.
    }
    params.delete("authToken");
    const nextQuery = params.toString();
    const nextUrl = `${window.location.pathname}${nextQuery ? `?${nextQuery}` : ""}${window.location.hash}`;
    window.history.replaceState({}, "", nextUrl);
  }

  return { cortexBase, authToken, panel };
}

export function readLocalStorageValue(key, fallback = "") {
  if (typeof window === "undefined") return fallback;
  try { return window.localStorage.getItem(key) || fallback;
  } catch { return fallback;
  }
}

export function normalizeCurrencyCode(raw) {
  const candidate = String(raw || "")
    .trim()
    .toUpperCase();
  return CURRENCY_OPTIONS.includes(candidate) ? candidate : "USD";
}

export function persistBrowserAuthToken(token) {
  if (typeof window === "undefined") return;
  try { if (token) { window.sessionStorage.setItem(CORTEX_AUTH_STORAGE_KEY, token);
      window.localStorage.removeItem(CORTEX_AUTH_STORAGE_KEY);
      clearLegacyBrowserAuthTokens();
    } else { window.sessionStorage.removeItem(CORTEX_AUTH_STORAGE_KEY);
      window.localStorage.removeItem(CORTEX_AUTH_STORAGE_KEY);
      clearLegacyBrowserAuthTokens();
    }
  } catch {
    // Ignore storage failures in restricted browser contexts.
  }
}

export function priorityRank(priority) {
  const map = { critical: 4, high: 3, medium: 2, low: 1 };
  return map[priority] || 0;
}

export async function readTauriInvoke() {
  if (typeof window === "undefined" || !window.__TAURI_INTERNALS__) { return null;
  }
  try { const { invoke } = await import("@tauri-apps/api/core");
    return invoke;
  } catch { return null;
  }
}
