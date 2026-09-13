function isDaemonOfflineErrorMessage(message) { const value = String(message || "").toLowerCase();
  return ( value.includes("cannot connect to daemon") || value.includes("cannot reach daemon") || value.includes("actively refused") ||
    value.includes("os error 10061") || value.includes("connection refused") );
}
function isDaemonTimeoutErrorMessage(message) { const value = String(message || "").toLowerCase();
  return ( value.includes("ipc request: timed out") ||
    value.includes("os error 10060") || value.includes("connection attempt failed because the connected party did not properly respond") ||
    value.includes("established connection failed because connected host has failed to respond") );
}
function isDaemonSuppressibleErrorMessage(message) { return isDaemonOfflineErrorMessage(message) || isDaemonTimeoutErrorMessage(message);
}
function isReachableHealthPayload(health) { const status = String(health?.status || "").toLowerCase();
  if (health?.ready === !1) return !1;
  return status !== "ok" && status !== "degraded" ? !1 : !!health?.runtime || !!health?.stats;
}
function isStartingHealthPayload(health) {
  if (!health || typeof health != "object") return !1;
  if (health.ready === !1) return !0;
  const status = String(health.status || "").toLowerCase();
  return status === "starting" || status === "warming";
}
function isDaemonCommandResult(value) {
  return !!value && typeof value == "object" && typeof value.running == "boolean" && typeof value.reachable == "boolean";
}
function isDaemonOfflineState(daemonState) {
  return !daemonState?.running && !daemonState?.reachable;
}
function daemonStateAfterStatusProbeFailure(previousState, healthState) {
  if (isDaemonStartingStateLike(previousState) || previousState?.managed)
    return { running: !0, reachable: !1, managed: !!previousState?.managed, authTokenReady: !1, pid: previousState?.pid ?? null,
      message: previousState?.message || healthState?.message || "Daemon is still starting.", };
  return healthState;
}
function isDaemonStartingStateLike(daemonState) {
  return !!daemonState?.running && !daemonState?.reachable;
}
function shouldOpenSseStream({ token = "", reachable = !1 } = {}) {
  return !!String(token || "").trim() && !!reachable;
}
function shouldScheduleSseReconnect({ token = "", reachable = !1, authTokenReady = !1, reconnectAttempt = 0, maxAttempts = 8 } = {}) {
  return reconnectAttempt >= maxAttempts ? !1 : !!String(token || "").trim() || !!reachable || !!authTokenReady;
}
function setElementInert(element, inert) { if (element) { if (inert) { (element.setAttribute("inert", ""), (element.inert = !0));
      return;
    }
    (element.removeAttribute("inert"), (element.inert = !1));
  }
}
function isReadyReadinessPayload(readiness) { if (!readiness || typeof readiness != "object") return !1;
  if (readiness.ready === !1) return !1;
  if (readiness.ready === !0) return !0;
  const status = String(readiness.status || "").toLowerCase();
  return status === "ready";
}
export {
  daemonStateAfterStatusProbeFailure, isDaemonCommandResult, isDaemonOfflineErrorMessage, isDaemonOfflineState, isDaemonSuppressibleErrorMessage,
  isDaemonTimeoutErrorMessage, isReachableHealthPayload, isReadyReadinessPayload, isStartingHealthPayload, setElementInert, shouldOpenSseStream,
  shouldScheduleSseReconnect, };
