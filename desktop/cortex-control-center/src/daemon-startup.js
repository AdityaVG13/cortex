const DEFAULT_STARTUP_MAX_ATTEMPTS = 36,
  DEFAULT_STARTUP_MAX_WINDOW_MS = 45e3,
  DEFAULT_STARTUP_BASE_DELAY_MS = 750,
  DEFAULT_STARTUP_MAX_DELAY_MS = 3e3,
  TRANSIENT_FEEDBACK_PREFIXES = [
    "Auth token read failed:",
    "Waiting for daemon auth token",
    "Daemon is still starting.",
    "Daemon startup timed out after",
    "Daemon is reachable but still warming up.",
  ];
function isDaemonStartingState(daemonState) {
  return !!daemonState?.running && !daemonState?.reachable;
}
function shouldContinueStartupRecovery({
  invokeAvailable = !1,
  daemonReachable = !1,
  currentDaemonState = null,
  previousDaemonState = null,
} = {}) {
  return !invokeAvailable || daemonReachable
    ? !1
    : currentDaemonState?.managed
      ? !0
      : isDaemonStartingState(currentDaemonState) ||
        isDaemonStartingState(previousDaemonState);
}
function isTransientDaemonFeedback(message) {
  const text = String(message || "");
  return (
    text === "Checking daemon..." ||
    text.includes("could not authenticate") ||
    text.includes(": HTTP 401") ||
    text.includes(": HTTP 403") ||
    TRANSIENT_FEEDBACK_PREFIXES.some((prefix) => text.startsWith(prefix))
  );
}
function daemonStatusPill(daemonState) {
  return daemonState?.reachable
    ? { className: "pill online", label: "Online" }
    : isDaemonStartingState(daemonState)
      ? { className: "pill starting", label: "Starting" }
      : { className: "pill offline", label: "Offline" };
}
function daemonUtilityPill(daemonState) {
  return daemonState?.reachable
    ? { className: "online", label: "Live" }
    : isDaemonStartingState(daemonState)
      ? { className: "starting", label: "Boot" }
      : { className: "offline", label: "Wait" };
}
function daemonSystemStatus(daemonState) {
  return daemonState?.reachable
    ? {
        toneClass: "sys-ok",
        daemonLabel: "RUNNING",
        embeddingsLabel: "ONNX ACTIVE",
      }
    : isDaemonStartingState(daemonState)
      ? {
          toneClass: "sys-warn",
          daemonLabel: "STARTING",
          embeddingsLabel: "WARMING",
        }
      : {
          toneClass: "sys-err",
          daemonLabel: "OFFLINE",
          embeddingsLabel: "OFFLINE",
        };
}
function buildFirstRunReadiness({
  daemonState = null,
  stats = null,
  sessions = [],
  editorSetupSummary = null,
  healthMeta = null,
  canStartDaemon = !1,
  canSetupEditors = !1,
  isSettingUpEditors = !1,
} = {}) {
  const daemonReachable = !!daemonState?.reachable,
    daemonStarting = isDaemonStartingState(daemonState),
    dbCorrupted = !!healthMeta?.dbCorrupted,
    degraded = !!healthMeta?.degraded,
    registeredEditors = Number(editorSetupSummary?.registered || 0),
    sessionCount = Array.isArray(sessions) ? sessions.length : 0,
    connectedTools = registeredEditors + sessionCount,
    memoryCount = Number(stats?.memories || 0) + Number(stats?.decisions || 0),
    steps = [
      {
        key: "runtime",
        label: "Memory runtime",
        state: daemonReachable
          ? dbCorrupted
            ? "Repair"
            : degraded
              ? "Degraded"
              : "Ready"
          : daemonStarting
            ? "Starting"
            : "Offline",
        tone:
          daemonReachable && !dbCorrupted ? (degraded ? "warn" : "ok") : "warn",
        detail: daemonReachable
          ? "Local daemon answered readiness checks."
          : daemonStarting
            ? "Daemon is still initializing."
            : "Control Center has not reached the local daemon.",
      },
      {
        key: "tool",
        label: "AI tool connection",
        state: connectedTools > 0 ? "Linked" : "Pending",
        tone: connectedTools > 0 ? "ok" : "warn",
        detail:
          connectedTools > 0
            ? `${connectedTools} tool connection${connectedTools === 1 ? "" : "s"} seen.`
            : "No active session or registered editor has been seen yet.",
      },
      {
        key: "memory",
        label: "First memory",
        state: memoryCount > 0 ? "Seen" : "Pending",
        tone: memoryCount > 0 ? "ok" : "warn",
        detail:
          memoryCount > 0
            ? `${memoryCount} knowledge entr${memoryCount === 1 ? "y" : "ies"} available.`
            : "Store one memory, then recall it from a connected tool.",
      },
    ];
  return daemonReachable
    ? dbCorrupted
      ? {
          statusLabel: "Repair",
          tone: "warn",
          nextAction:
            "Restart Cortex from Control Center to trigger local repair.",
          action: { kind: "restart_daemon", label: "Restart", disabled: !1 },
          steps,
        }
      : connectedTools === 0
        ? {
            statusLabel: "Connect",
            tone: "warn",
            nextAction: canSetupEditors
              ? "Register Cortex MCP in your AI tool."
              : "Open the desktop app to register Cortex MCP in your AI tool.",
            action: {
              kind: "setup_mcp",
              label: isSettingUpEditors ? "Setting Up..." : "Setup MCP",
              disabled: !canSetupEditors,
            },
            steps,
          }
        : memoryCount === 0
          ? {
              statusLabel: "Store",
              tone: "warn",
              nextAction:
                "Store one memory from a connected tool, then recall it.",
              action: {
                kind: "open_memory",
                label: "Open Memory",
                disabled: !1,
              },
              steps,
            }
          : {
              statusLabel: "Ready",
              tone: degraded ? "warn" : "ok",
              nextAction: degraded
                ? "Recall is available, but semantic search is degraded; restart if it persists."
                : "Cortex memory is ready for connected AI tools.",
              action: {
                kind: "open_memory",
                label: "Open Memory",
                disabled: !1,
              },
              steps,
            }
    : daemonStarting
      ? {
          statusLabel: "Starting",
          tone: "warn",
          nextAction: "Wait for startup to finish, then refresh.",
          action: { kind: "refresh", label: "Refresh", disabled: !1 },
          steps,
        }
      : {
          statusLabel: "Start",
          tone: "warn",
          nextAction: canStartDaemon
            ? "Start the local Cortex runtime from Control Center."
            : "Open the desktop app runtime controls, then start Cortex.",
          action: {
            kind: "start_daemon",
            label: "Start",
            disabled: !canStartDaemon,
          },
          steps,
        };
}
function computeStartupRetryStep(
  previousState = {},
  nowMs = Date.now(),
  overrides = {},
) {
  const maxAttempts = Number.isFinite(overrides.maxAttempts)
      ? Math.max(1, Math.floor(overrides.maxAttempts))
      : 36,
    maxWindowMs = Number.isFinite(overrides.maxWindowMs)
      ? Math.max(1e3, Math.floor(overrides.maxWindowMs))
      : 45e3,
    baseDelayMs = Number.isFinite(overrides.baseDelayMs)
      ? Math.max(200, Math.floor(overrides.baseDelayMs))
      : 750,
    maxDelayMs = Number.isFinite(overrides.maxDelayMs)
      ? Math.max(baseDelayMs, Math.floor(overrides.maxDelayMs))
      : 3e3,
    initialStartedAtMs = Number(previousState?.startedAtMs) || 0,
    startedAtMs = initialStartedAtMs > 0 ? initialStartedAtMs : nowMs,
    attempts = (Number(previousState?.attempts) || 0) + 1,
    elapsedMs = Math.max(0, nowMs - startedAtMs),
    exhausted = attempts >= maxAttempts || elapsedMs >= maxWindowMs,
    backoffStage = Math.max(0, Math.floor((attempts - 1) / 4)),
    nextDelayMs = Math.min(maxDelayMs, baseDelayMs * 2 ** backoffStage);
  return { startedAtMs, attempts, elapsedMs, exhausted, nextDelayMs };
}
export {
  buildFirstRunReadiness,
  computeStartupRetryStep,
  daemonStatusPill,
  daemonSystemStatus,
  daemonUtilityPill,
  isDaemonStartingState,
  isTransientDaemonFeedback,
  shouldContinueStartupRecovery,
};
