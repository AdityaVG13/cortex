/**
 * Extracted API client logic for Cortex Control Center.
 * Pure functions that accept deps as params -- testable without React.
 */

const TOKEN_REFRESH_ATTEMPTS = 4;
const TOKEN_REFRESH_DELAY_MS = 250;
const IPC_ABORT_TIMEOUT_MS = 8_000;
const IPC_ABORT_TIMEOUT_HEALTH_MS = 12_000;
const IPC_ABORT_TIMEOUT_MCP_MS = 30_000;
const IPC_ABORT_TIMEOUT_RECALL_MS = 20_000;
const IPC_TRANSPORT_MARGIN_MS = 500;

function wait(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function isAuthStatus(status) {
  return status === 401 || status === 403;
}

async function withTimeout(promise, timeoutMs, label) {
  let timer = null;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => {
          reject(new Error(`${label}: timed out after ${timeoutMs}ms`));
        }, timeoutMs);
      }),
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}

function resolveIpcTimeoutMs(path) {
  const normalized = String(path || "").toLowerCase();
  if (normalized === "/health" || normalized.startsWith("/health?")) return IPC_ABORT_TIMEOUT_HEALTH_MS;
  if (normalized.startsWith("/mcp-rpc")) return IPC_ABORT_TIMEOUT_MCP_MS;
  if (normalized.startsWith("/recall")) return IPC_ABORT_TIMEOUT_RECALL_MS;
  return IPC_ABORT_TIMEOUT_MS;
}

function resolveIpcTransportTimeoutMs(path) {
  return Math.max(500, resolveIpcTimeoutMs(path) - IPC_TRANSPORT_MARGIN_MS);
}

async function refreshTokenIfChanged(onTokenRefresh, getToken, previousToken) {
  if (!onTokenRefresh) return false;

  const requiresRotation = Boolean(previousToken);
  for (let attempt = 1; attempt <= TOKEN_REFRESH_ATTEMPTS; attempt += 1) {
    await onTokenRefresh(previousToken, attempt);
    const nextToken = getToken();
    const ready = Boolean(nextToken) && (!requiresRotation || nextToken !== previousToken);
    if (ready) {
      return true;
    }
    if (attempt < TOKEN_REFRESH_ATTEMPTS) {
      await wait(TOKEN_REFRESH_DELAY_MS * attempt);
    }
  }

  return false;
}

/**
 * Creates a GET API caller.
 * @template TResponse
 * @param {object} deps
 * @param {() => Function|null} deps.getInvoke - returns Tauri invoke fn or null
 * @param {() => string} deps.getToken - returns current auth token
 * @param {string} deps.cortexBase - base URL for browser fallback
 * @returns {(path: string, withAuth?: boolean) => Promise<TResponse>}
 */
export function createApi({ getInvoke, getToken, cortexBase, onTokenRefresh }) {
  return async function api(path, withAuth = false, _retried = false) {
    const invoke = getInvoke();
    let token = getToken();

    if (withAuth && !token && !_retried) {
      // Token not loaded yet -- try refreshing once (daemon may still be writing it)
      const refreshed = await refreshTokenIfChanged(onTokenRefresh, getToken, token);
      if (refreshed) {
        return api(path, withAuth, true);
      }
      token = getToken();
    }

    if (withAuth && !token) {
      throw new Error(`${path}: no auth token (Tauri IPC ${invoke ? "available" : "missing"})`);
    }

    if (invoke) {
      const timeoutMs = resolveIpcTimeoutMs(path);
      const transportTimeoutMs = resolveIpcTransportTimeoutMs(path);
      const response = await withTimeout(invoke("fetch_cortex", {
        path,
        authToken: withAuth ? token : "",
        timeoutMs: transportTimeoutMs,
      }), timeoutMs, `${path}: IPC request`);
      if (!response || typeof response.status !== "number" || typeof response.body !== "string") {
        throw new Error(`${path}: invalid IPC response`);
      }
      // On 401, re-read token and retry once (handles daemon token rotation on startup)
      if (isAuthStatus(response.status) && withAuth && !_retried) {
        const refreshed = await refreshTokenIfChanged(onTokenRefresh, getToken, token);
        if (refreshed) {
          return api(path, withAuth, true);
        }
      }
      if (response.status < 200 || response.status >= 300) {
        throw new Error(`${path}: HTTP ${response.status}`);
      }
      return JSON.parse(response.body);
    }

    const headers = { "X-Cortex-Request": "true" };
    if (withAuth) headers.Authorization = `Bearer ${token}`;
    const response = await fetch(`${cortexBase}${path}`, { headers });
    if (isAuthStatus(response.status) && withAuth && !_retried) {
      const refreshed = await refreshTokenIfChanged(onTokenRefresh, getToken, token);
      if (refreshed) {
        return api(path, withAuth, true);
      }
    }
    if (!response.ok) {
      throw new Error(`${path}: HTTP ${response.status}`);
    }
    return await response.json();
  };
}

/**
 * Creates a POST API caller.
 * @template TResponse
 * @param {object} deps
 * @param {() => Function|null} deps.getInvoke - returns Tauri invoke fn or null
 * @param {() => string} deps.getToken - returns current auth token
 * @param {string} deps.cortexBase - base URL for browser fallback
 * @param {() => Promise<void>|void} [deps.onTokenRefresh] - refreshes auth token once on startup/rotation
 * @returns {(path: string, body?: Record<string, unknown>) => Promise<TResponse>}
 */
export function createPostApi({ getInvoke, getToken, cortexBase, onTokenRefresh }) {
  return async function postApi(path, body = {}, _retried = false) {
    const invoke = getInvoke();
    let token = getToken();

    if (!token && !_retried) {
      const refreshed = await refreshTokenIfChanged(onTokenRefresh, getToken, token);
      if (refreshed) {
        return postApi(path, body, true);
      }
      token = getToken();
    }

    if (!token) {
      throw new Error(`POST ${path}: no auth token`);
    }

    if (invoke) {
      const timeoutMs = resolveIpcTimeoutMs(path);
      const transportTimeoutMs = resolveIpcTransportTimeoutMs(path);
      const response = await withTimeout(invoke("post_cortex", {
        path,
        authToken: token,
        body: JSON.stringify(body),
        timeoutMs: transportTimeoutMs,
      }), timeoutMs, `POST ${path}: IPC request`);
      if (!response || typeof response.status !== "number" || typeof response.body !== "string") {
        throw new Error(`POST ${path}: invalid IPC response`);
      }
      if (isAuthStatus(response.status) && !_retried) {
        const refreshed = await refreshTokenIfChanged(onTokenRefresh, getToken, token);
        if (refreshed) {
          return postApi(path, body, true);
        }
      }
      if (response.status < 200 || response.status >= 300) {
        throw new Error(`POST ${path}: HTTP ${response.status}`);
      }
      return JSON.parse(response.body);
    }

    const response = await fetch(`${cortexBase}${path}`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-Cortex-Request": "true",
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify(body),
    });
    if (isAuthStatus(response.status) && !_retried) {
      const refreshed = await refreshTokenIfChanged(onTokenRefresh, getToken, token);
      if (refreshed) {
        return postApi(path, body, true);
      }
    }
    if (!response.ok) {
      throw new Error(`POST ${path}: HTTP ${response.status}`);
    }
    return await response.json();
  };
}

const PANEL_LABELS = {
  "/sessions": "Sessions",
  "/locks": "Locks",
  "/tasks": "Tasks",
  "/feed": "Feed",
  "/messages": "Messages",
  "/activity": "Activity",
  "/savings": "Savings",
  "/conflicts": "Conflicts",
  "/permissions": "Permissions",
};

function panelLabelFromError(message) {
  const path = String(message || "").split(":")[0];
  const normalized = Object.keys(PANEL_LABELS).find((candidate) => path.startsWith(candidate));
  return normalized ? PANEL_LABELS[normalized] : null;
}

export function isAuthFailure(message) {
  const text = String(message || "");
  return text.includes("HTTP 401") || text.includes("HTTP 403") || text.includes("no auth token");
}

export function summarizeDashboardErrors(errors) {
  const unique = [...new Set((errors || []).filter(Boolean))];
  if (!unique.length) return "";

  const authFailures = unique.filter(isAuthFailure);
  if (authFailures.length !== unique.length) {
    return unique.join("; ");
  }

  const panels = authFailures
    .map(panelLabelFromError)
    .filter(Boolean);

  if (!panels.length) {
    return "Protected Cortex panels could not authenticate. Refresh the token or restart the daemon from Control Center.";
  }

  return `${panels.join(", ")} could not authenticate. Refresh the token or restart the daemon from Control Center.`;
}

/**
 * Runs multiple async fns via allSettled, applies partial results,
 * then re-throws if any failed.
 * @template T
 * @param {Array<{fn: () => Promise<T>, apply: (value: T | null) => void}>} tasks
 */
export async function settledWithRethrow(tasks) {
  const results = await Promise.allSettled(tasks.map(t => t.fn()));
  results.forEach((r, i) => {
    tasks[i].apply(r.status === "fulfilled" ? r.value : null);
  });
  const failed = results.filter(r => r.status === "rejected");
  if (failed.length) {
    const reasons = failed.map(f => f.reason?.message || String(f.reason));
    throw new Error(reasons.join("; "));
  }
}

/**
 * Runs multiple async fns via allSettled, collects unique error messages.
 * Never throws.
 * @param {Array<() => Promise<void>>} fns
 * @returns {Promise<string[]>} unique error messages (empty if all succeeded)
 */
export async function settledCollectErrors(fns) {
  const results = await Promise.allSettled(fns.map(fn => fn()));
  const failures = results.filter(r => r.status === "rejected");
  if (!failures.length) return [];
  const reasons = failures.map(f => f.reason?.message || String(f.reason));
  return [...new Set(reasons)];
}
