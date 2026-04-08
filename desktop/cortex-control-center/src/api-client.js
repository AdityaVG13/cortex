/**
 * Extracted API client logic for Cortex Control Center.
 * Pure functions that accept deps as params -- testable without React.
 */

/**
 * Creates a GET API caller.
 * @param {object} deps
 * @param {() => Function|null} deps.getInvoke - returns Tauri invoke fn or null
 * @param {() => string} deps.getToken - returns current auth token
 * @param {string} deps.cortexBase - base URL for browser fallback
 * @returns {(path: string, withAuth?: boolean) => Promise<any>}
 */
export function createApi({ getInvoke, getToken, cortexBase, onTokenRefresh }) {
  return async function api(path, withAuth = false, _retried = false) {
    const invoke = getInvoke();
    const token = getToken();

    if (withAuth && !token && !_retried) {
      // Token not loaded yet -- try refreshing once (daemon may still be writing it)
      if (onTokenRefresh) await onTokenRefresh();
      return api(path, withAuth, true);
    }

    if (withAuth && !token) {
      throw new Error(`${path}: no auth token (Tauri IPC ${invoke ? "available" : "missing"})`);
    }

    // Tauri IPC path
    if (invoke) {
      const response = await invoke("fetch_cortex", {
        path,
        authToken: withAuth ? token : "",
      });
      if (!response || typeof response.status !== "number" || typeof response.body !== "string") {
        throw new Error(`${path}: invalid IPC response`);
      }
      // On 401, re-read token and retry once (handles daemon token rotation on startup)
      if (response.status === 401 && withAuth && !_retried && onTokenRefresh) {
        await onTokenRefresh();
        return api(path, withAuth, true);
      }
      if (response.status < 200 || response.status >= 300) {
        throw new Error(`${path}: HTTP ${response.status}`);
      }
      return JSON.parse(response.body);
    }

    // Browser fallback
    const headers = { "X-Cortex-Request": "true" };
    if (withAuth) headers.Authorization = `Bearer ${token}`;
    const response = await fetch(`${cortexBase}${path}`, { headers });
    if (response.status === 401 && withAuth && !_retried && onTokenRefresh) {
      await onTokenRefresh();
      return api(path, withAuth, true);
    }
    if (!response.ok) {
      throw new Error(`${path}: HTTP ${response.status}`);
    }
    return await response.json();
  };
}

/**
 * Creates a POST API caller.
 * @param {object} deps
 * @param {() => Function|null} deps.getInvoke - returns Tauri invoke fn or null
 * @param {() => string} deps.getToken - returns current auth token
 * @param {string} deps.cortexBase - base URL for browser fallback
 * @returns {(path: string, body?: object) => Promise<any>}
 */
export function createPostApi({ getInvoke, getToken, cortexBase }) {
  return async function postApi(path, body = {}) {
    const invoke = getInvoke();
    const token = getToken();

    if (!token) {
      throw new Error(`POST ${path}: no auth token`);
    }

    // Tauri IPC path
    if (invoke) {
      const response = await invoke("post_cortex", {
        path,
        authToken: token,
        body: JSON.stringify(body),
      });
      if (!response || typeof response.status !== "number" || typeof response.body !== "string") {
        throw new Error(`POST ${path}: invalid IPC response`);
      }
      if (response.status < 200 || response.status >= 300) {
        throw new Error(`POST ${path}: HTTP ${response.status}`);
      }
      return JSON.parse(response.body);
    }

    // Browser fallback
    const response = await fetch(`${cortexBase}${path}`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-Cortex-Request": "true",
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify(body),
    });
    if (!response.ok) {
      throw new Error(`POST ${path}: HTTP ${response.status}`);
    }
    return await response.json();
  };
}

/**
 * Runs multiple async fns via allSettled, applies partial results,
 * then re-throws if any failed. Used by refreshCoreData.
 * @param {Array<{fn: () => Promise<any>, apply: (value: any) => void}>} tasks
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
 * Never throws. Used by refreshAll.
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
