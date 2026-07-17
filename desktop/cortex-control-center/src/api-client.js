const TOKEN_REFRESH_ATTEMPTS = 4, TOKEN_REFRESH_DELAY_MS = 250, IPC_ABORT_TIMEOUT_MS = 8e3, IPC_ABORT_TIMEOUT_HEALTH_MS = 12e3,
  IPC_ABORT_TIMEOUT_MCP_MS = 3e4, IPC_ABORT_TIMEOUT_RECALL_MS = 2e4, IPC_ABORT_TIMEOUT_CORE_MS = 15e3, IPC_ABORT_TIMEOUT_SECONDARY_MS = 2e4,
  IPC_ABORT_TIMEOUT_ANALYTICS_MS = 6e4, IPC_TRANSPORT_MARGIN_MS = 500;
function wait(ms) { return new Promise((resolve) => setTimeout(resolve, ms));
}
function isAuthStatus(status) { return status === 401 || status === 403;
}
async function withTimeout(promise, timeoutMs, label) { let timer = null;
  try { return await Promise.race([ promise, new Promise((_, reject) => {
        timer = setTimeout(() => { reject(new Error(`${label}: timed out after ${timeoutMs}ms`));
        }, timeoutMs);
      }), ]);
  } finally { timer && clearTimeout(timer);
  }
}
function normalizePathForTimeoutRouting(path) { const raw = String(path || "")
    .trim()
    .toLowerCase();
  if (!raw) return "";
  if (raw.startsWith("http://") || raw.startsWith("https://"))
    try { const parsed = new URL(raw);
      return `${parsed.pathname || "/"}`.toLowerCase() + (parsed.search || "");
    } catch { return raw;
    }
  return raw.startsWith("/") ? raw : `/${raw}`;
}
function normalizeCortexBaseUrl(cortexBase) { let url;
  try { url = new URL(String(cortexBase || "").trim());
  } catch { throw new Error("Cortex base URL must be a valid URL.");
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") throw new Error("Cortex base URL must use http or https.");
  if (!url.hostname) throw new Error("Cortex base URL must include a host.");
  if (url.username || url.password) throw new Error("Cortex base URL must not include embedded credentials.");
  return ((url.hash = ""), (url.search = ""), url.toString().replace(/\/+$/, ""));
}
function buildHttpFallbackUrl(cortexBase, path) { const normalizedPath = String(path || "").trim(),
    route = normalizedPath.startsWith("/") ? normalizedPath : `/${normalizedPath}`;
  return `${normalizeCortexBaseUrl(cortexBase)}${route}`;
}
function formatHttpError(path, status, bodyText) { if (!bodyText) return `${path}: HTTP ${status}`;
  try { const parsed = JSON.parse(bodyText);
    if (parsed && typeof parsed.error == "string" && parsed.error.trim())
      return `${path}: HTTP ${status} (${parsed.error.trim()})`;
    if (parsed && typeof parsed.message == "string" && parsed.message.trim())
      return `${path}: HTTP ${status} (${parsed.message.trim()})`;
  } catch { const trimmed = bodyText.trim().slice(0, 200);
    if (trimmed) return `${path}: HTTP ${status} (${trimmed})`;
  }
  return `${path}: HTTP ${status}`;
}
function resolveIpcTimeoutMs(path) { const normalized = normalizePathForTimeoutRouting(path);
  return normalized === "/health" || normalized.startsWith("/health?")
    ? 12e3
    : normalized === "/sessions" || normalized === "/locks" || normalized.startsWith("/tasks")
      ? 15e3
      : normalized.startsWith("/feed") || normalized.startsWith("/messages") || normalized.startsWith("/activity") || normalized.startsWith("/conflicts") ||
          normalized.startsWith("/permissions")
        ? 2e4
        : normalized.startsWith("/savings")
          ? 6e4
          : normalized.startsWith("/mcp-rpc")
            ? 3e4
            : normalized.startsWith("/recall")
              ? 2e4
              : 8e3;
}
function resolveIpcTransportTimeoutMs(path) { return Math.max(500, resolveIpcTimeoutMs(path) - 500);
}
function isIpcResponseEnvelope(value) { return !value || typeof value != "object" ? !1 : typeof value.status == "number" && typeof value.body == "string";
}
function shouldFallbackToHttp(error) { const message = String(error?.message || error || "").toLowerCase();
  return ( message.includes("ipc request") || message.includes("task failed") || message.includes("invalid ipc response") ||
    message.includes("read failed") || message.includes("write failed") ||
    message.includes("os error 10060") || message.includes("connection attempt failed because the connected party did not properly respond") ||
    message.includes("established connection failed because connected host has failed to respond") || message.includes("cannot connect to daemon") ||
    message.includes("cannot set read timeout") || message.includes("cannot set write timeout") );
}
function errorMessage(error) { return error instanceof Error ? error.message : String(error ?? "");
}
function buildFallbackFailure(ipcError, httpError) { const ipcMessage = errorMessage(ipcError), httpMessage = errorMessage(httpError);
  return new Error(`${ipcMessage}; HTTP fallback failed: ${httpMessage}`);
}
async function refreshTokenIfChanged(onTokenRefresh, getToken, previousToken) { if (!onTokenRefresh) return !1;
  const requiresRotation = !!previousToken;
  for (let attempt = 1; attempt <= 4; attempt += 1) { await onTokenRefresh(previousToken, attempt);
    const nextToken = getToken();
    if (!!nextToken && (!requiresRotation || nextToken !== previousToken)) return !0;
    attempt < 4 && (await wait(250 * attempt));
  }
  return !1;
}
function createApi({ getInvoke, getToken, cortexBase, onTokenRefresh }) { return async function api(path, withAuth = !1, _retried = !1) {
    const invoke = getInvoke();
    let token = getToken();
    if (withAuth && !token && !_retried) { if (await refreshTokenIfChanged(onTokenRefresh, getToken, token)) return api(path, withAuth, !0);
      token = getToken();
    }
    if (withAuth && !token) throw new Error(`${path}: no auth token (Tauri IPC ${invoke ? "available" : "missing"})`);
    const requestViaHttp = async () => { const headers = { "X-Cortex-Request": "true" };
      withAuth && (headers.Authorization = `Bearer ${token}`);
      const response = await fetch(buildHttpFallbackUrl(cortexBase, path), { headers, });
      if ( isAuthStatus(response.status) && withAuth && !_retried && (await refreshTokenIfChanged(onTokenRefresh, getToken, token)) )
        return api(path, withAuth, !0);
      if (!response.ok) { const bodyText = await response.text().catch(() => "");
        throw new Error(formatHttpError(path, response.status, bodyText));
      }
      return await response.json();
    };
    if (invoke)
      try { const timeoutMs = resolveIpcTimeoutMs(path), transportTimeoutMs = resolveIpcTransportTimeoutMs(path), response = await withTimeout(
            invoke("fetch_cortex", { path, authToken: withAuth ? token : "", timeoutMs: transportTimeoutMs, }), timeoutMs, `${path}: IPC request`, );
        if (!isIpcResponseEnvelope(response)) throw new Error(`${path}: invalid IPC response`);
        if ( isAuthStatus(response.status) && withAuth && !_retried && (await refreshTokenIfChanged(onTokenRefresh, getToken, token)) )
          return api(path, withAuth, !0);
        if (response.status < 200 || response.status >= 300)
          throw new Error(formatHttpError(path, response.status, response.body));
        return JSON.parse(response.body);
      } catch (ipcError) { if (!shouldFallbackToHttp(ipcError)) throw ipcError;
        try { return await requestViaHttp();
        } catch (httpError) { throw buildFallbackFailure(ipcError, httpError);
        }
      }
    return requestViaHttp();
  };
}
function createPostApi({ getInvoke, getToken, cortexBase, onTokenRefresh }) { return async function postApi(path, body = {}, _retried = !1) {
    const invoke = getInvoke();
    let token = getToken();
    if (!token && !_retried) { if (await refreshTokenIfChanged(onTokenRefresh, getToken, token)) return postApi(path, body, !0);
      token = getToken();
    }
    if (!token) throw new Error(`POST ${path}: no auth token`);
    const requestViaHttp = async () => { const response = await fetch(buildHttpFallbackUrl(cortexBase, path), { method: "POST", headers: {
          "Content-Type": "application/json", "X-Cortex-Request": "true", Authorization: `Bearer ${token}`, }, body: JSON.stringify(body), });
      if (isAuthStatus(response.status) && !_retried && (await refreshTokenIfChanged(onTokenRefresh, getToken, token)))
        return postApi(path, body, !0);
      if (!response.ok) { const bodyText = await response.text().catch(() => "");
        throw new Error(formatHttpError(`POST ${path}`, response.status, bodyText));
      }
      return await response.json();
    };
    if (invoke)
      try { const timeoutMs = resolveIpcTimeoutMs(path), transportTimeoutMs = resolveIpcTransportTimeoutMs(path), response = await withTimeout(
            invoke("post_cortex", { path, authToken: token, body: JSON.stringify(body),
              timeoutMs: transportTimeoutMs, }), timeoutMs, `POST ${path}: IPC request`, );
        if (!isIpcResponseEnvelope(response)) throw new Error(`POST ${path}: invalid IPC response`);
        if ( isAuthStatus(response.status) && !_retried && (await refreshTokenIfChanged(onTokenRefresh, getToken, token)) )
          return postApi(path, body, !0);
        if (response.status < 200 || response.status >= 300)
          throw new Error(formatHttpError(`POST ${path}`, response.status, response.body));
        return JSON.parse(response.body);
      } catch (ipcError) { if (!shouldFallbackToHttp(ipcError)) throw ipcError;
        try { return await requestViaHttp();
        } catch (httpError) { throw buildFallbackFailure(ipcError, httpError);
        }
      }
    return requestViaHttp();
  };
}
const PANEL_LABELS = { "/sessions": "Sessions", "/locks": "Locks", "/tasks": "Tasks",
  "/feed": "Feed", "/messages": "Messages", "/activity": "Activity", "/savings": "Savings", "/conflicts": "Conflicts", "/permissions": "Permissions", };
function panelLabelFromError(message) { const path = String(message || "").split(":")[0],
    normalized = Object.keys(PANEL_LABELS).find((candidate) => path.startsWith(candidate));
  return normalized ? PANEL_LABELS[normalized] : null;
}
function isAuthFailure(message) { const text = String(message || "");
  return text.includes("HTTP 401") || text.includes("HTTP 403") || text.includes("no auth token");
}
function summarizeDashboardErrors(errors) { const unique = [...new Set((errors || []).filter(Boolean))];
  if (!unique.length) return "";
  const authFailures = unique.filter(isAuthFailure);
  if (authFailures.length !== unique.length) return unique.join("; ");
  const panels = authFailures.map(panelLabelFromError).filter(Boolean);
  return panels.length
    ? `${panels.join(", ")} could not authenticate. Refresh the token or restart the daemon from Control Center.`
    : "Protected Cortex panels could not authenticate. Refresh the token or restart the daemon from Control Center.";
}
async function settledWithRethrow(tasks) { const results = await Promise.allSettled(tasks.map((t) => t.fn()));
  results.forEach((r, i) => { tasks[i].apply(r.status === "fulfilled" ? r.value : null);
  });
  const failed = results.filter((r) => r.status === "rejected");
  if (failed.length) { const reasons = failed.map((f) => errorMessage(f.reason));
    throw new Error(reasons.join("; "));
  }
}
async function settledCollectErrors(fns) { const failures = (await Promise.allSettled(fns.map((fn) => fn()))).filter((r) => r.status === "rejected");
  if (!failures.length) return [];
  const reasons = failures.map((f) => errorMessage(f.reason));
  return [...new Set(reasons)];
}
export { createApi, createPostApi, isAuthFailure, settledCollectErrors, settledWithRethrow, summarizeDashboardErrors };
