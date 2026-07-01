export function isDaemonOfflineErrorMessage(message) {
  const value = String(message || "").toLowerCase();
  return (
    value.includes("cannot connect to daemon") ||
    value.includes("cannot reach daemon") ||
    value.includes("actively refused") ||
    value.includes("os error 10061") ||
    value.includes("connection refused")
  );
}

export function isDaemonTimeoutErrorMessage(message) {
  const value = String(message || "").toLowerCase();
  return (
    value.includes("ipc request: timed out")
    || value.includes("os error 10060")
    || value.includes("connection attempt failed because the connected party did not properly respond")
    || value.includes("established connection failed because connected host has failed to respond")
  );
}

export function isDaemonSuppressibleErrorMessage(message) {
  return isDaemonOfflineErrorMessage(message) || isDaemonTimeoutErrorMessage(message);
}

export function isReachableHealthPayload(health) {
  const status = String(health?.status || "").toLowerCase();
  if (status !== "ok" && status !== "degraded") {
    return false;
  }
  return Boolean(health?.runtime) || Boolean(health?.stats);
}

export function setElementInert(element, inert) {
  if (!element) return;
  if (inert) {
    element.setAttribute("inert", "");
    element.inert = true;
    return;
  }
  element.removeAttribute("inert");
  element.inert = false;
}

export function isReadyReadinessPayload(readiness) {
  if (!readiness || typeof readiness !== "object") return false;
  if (readiness.ready === true) return true;
  const status = String(readiness.status || "").toLowerCase();
  return status === "ready" || status === "ok";
}

export function parseMcpToolResult(result) {
  const text = result?.content?.find((item) => typeof item?.text === "string")?.text || "";
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return { text };
  }
}

export function extractMcpToolError(payload) {
  if (payload?.error?.message) {
    return payload.error.message;
  }
  if (!payload?.result?.isError) {
    return "";
  }
  const parsed = parseMcpToolResult(payload.result);
  if (parsed && typeof parsed === "object" && typeof parsed.error === "string") {
    return parsed.error;
  }
  return parsed?.text || "Unknown MCP error.";
}
