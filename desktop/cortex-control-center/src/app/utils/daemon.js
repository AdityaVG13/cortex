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
  return status !== "ok" && status !== "degraded" ? !1 : !!health?.runtime || !!health?.stats;
}
function setElementInert(element, inert) { if (element) { if (inert) { (element.setAttribute("inert", ""), (element.inert = !0));
      return;
    }
    (element.removeAttribute("inert"), (element.inert = !1));
  }
}
function isReadyReadinessPayload(readiness) { if (!readiness || typeof readiness != "object") return !1;
  if (readiness.ready === !0) return !0;
  const status = String(readiness.status || "").toLowerCase();
  return status === "ready" || status === "ok";
}
export {
  isDaemonOfflineErrorMessage, isDaemonSuppressibleErrorMessage, isDaemonTimeoutErrorMessage, isReachableHealthPayload,
  isReadyReadinessPayload, setElementInert, };
