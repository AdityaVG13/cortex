const DEFAULT_STARTUP_MAX_ATTEMPTS = 36;
const DEFAULT_STARTUP_MAX_WINDOW_MS = 45000;
const DEFAULT_STARTUP_BASE_DELAY_MS = 750;
const DEFAULT_STARTUP_MAX_DELAY_MS = 3000;

const TRANSIENT_FEEDBACK_PREFIXES = [
  "Auth token read failed:",
  "Waiting for daemon auth token",
  "Daemon is still starting.",
  "Daemon startup timed out after",
  "Daemon is reachable but still warming up.",
];

export function isDaemonStartingState(daemonState) {
  return Boolean(daemonState?.running) && !Boolean(daemonState?.reachable);
}

export function shouldContinueStartupRecovery({
  invokeAvailable = false,
  daemonReachable = false,
  currentDaemonState = null,
  previousDaemonState = null,
} = {}) {
  if (!invokeAvailable || daemonReachable) {
    return false;
  }
  if (Boolean(currentDaemonState?.managed)) {
    return true;
  }
  return isDaemonStartingState(currentDaemonState) || isDaemonStartingState(previousDaemonState);
}

export function isTransientDaemonFeedback(message) {
  const text = String(message || "");
  return (
    text === "Checking daemon..."
    || text.includes("could not authenticate")
    || text.includes(": HTTP 401")
    || text.includes(": HTTP 403")
    || TRANSIENT_FEEDBACK_PREFIXES.some((prefix) => text.startsWith(prefix))
  );
}

export function daemonStatusPill(daemonState) {
  if (daemonState?.reachable) return { className: "pill online", label: "Online" };
  if (isDaemonStartingState(daemonState)) return { className: "pill starting", label: "Starting" };
  return { className: "pill offline", label: "Offline" };
}

export function daemonUtilityPill(daemonState) {
  if (daemonState?.reachable) return { className: "online", label: "Live" };
  if (isDaemonStartingState(daemonState)) return { className: "starting", label: "Boot" };
  return { className: "offline", label: "Wait" };
}

export function daemonSystemStatus(daemonState) {
  if (daemonState?.reachable) {
    return {
      toneClass: "sys-ok",
      daemonLabel: "RUNNING",
      embeddingsLabel: "ONNX ACTIVE",
    };
  }

  if (isDaemonStartingState(daemonState)) {
    return {
      toneClass: "sys-warn",
      daemonLabel: "STARTING",
      embeddingsLabel: "WARMING",
    };
  }

  return {
    toneClass: "sys-err",
    daemonLabel: "OFFLINE",
    embeddingsLabel: "OFFLINE",
  };
}

export function buildFirstRunReadiness({
  daemonState = null,
  stats = null,
  sessions = [],
  editorSetupSummary = null,
  healthMeta = null,
  canStartDaemon = false,
  canSetupEditors = false,
  isSettingUpEditors = false,
} = {}) {
  const daemonReachable = Boolean(daemonState?.reachable);
  const daemonStarting = isDaemonStartingState(daemonState);
  const dbCorrupted = Boolean(healthMeta?.dbCorrupted);
  const degraded = Boolean(healthMeta?.degraded);
  const registeredEditors = Number(editorSetupSummary?.registered || 0);
  const sessionCount = Array.isArray(sessions) ? sessions.length : 0;
  const connectedTools = registeredEditors + sessionCount;
  const memoryCount = Number(stats?.memories || 0) + Number(stats?.decisions || 0);

  const steps = [
    {
      key: "runtime",
      label: "Memory runtime",
      state: daemonReachable ? (dbCorrupted ? "Repair" : degraded ? "Degraded" : "Ready") : daemonStarting ? "Starting" : "Offline",
      tone: daemonReachable && !dbCorrupted ? (degraded ? "warn" : "ok") : "warn",
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
      detail: connectedTools > 0
        ? `${connectedTools} tool connection${connectedTools === 1 ? "" : "s"} seen.`
        : "No active session or registered editor has been seen yet.",
    },
    {
      key: "memory",
      label: "First memory",
      state: memoryCount > 0 ? "Seen" : "Pending",
      tone: memoryCount > 0 ? "ok" : "warn",
      detail: memoryCount > 0
        ? `${memoryCount} knowledge entr${memoryCount === 1 ? "y" : "ies"} available.`
        : "Store one memory, then recall it from a connected tool.",
    },
  ];

  if (!daemonReachable) {
    if (daemonStarting) {
      return {
        statusLabel: "Starting",
        tone: "warn",
        nextAction: "Wait for startup to finish, then refresh.",
        action: { kind: "refresh", label: "Refresh", disabled: false },
        steps,
      };
    }
    return {
      statusLabel: "Start",
      tone: "warn",
      nextAction: canStartDaemon
        ? "Start the local Cortex runtime from Control Center."
        : "Open the desktop app runtime controls, then start Cortex.",
      action: { kind: "start_daemon", label: "Start", disabled: !canStartDaemon },
      steps,
    };
  }

  if (dbCorrupted) {
    return {
      statusLabel: "Repair",
      tone: "warn",
      nextAction: "Restart Cortex from Control Center to trigger local repair.",
      action: { kind: "restart_daemon", label: "Restart", disabled: false },
      steps,
    };
  }

  if (connectedTools === 0) {
    return {
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
    };
  }

  if (memoryCount === 0) {
    return {
      statusLabel: "Store",
      tone: "warn",
      nextAction: "Store one memory from a connected tool, then recall it.",
      action: { kind: "open_memory", label: "Open Memory", disabled: false },
      steps,
    };
  }

  return {
    statusLabel: "Ready",
    tone: degraded ? "warn" : "ok",
    nextAction: degraded
      ? "Recall is available, but semantic search is degraded; restart if it persists."
      : "Cortex memory is ready for connected AI tools.",
    action: { kind: "open_memory", label: "Open Memory", disabled: false },
    steps,
  };
}

export function computeStartupRetryStep(previousState = {}, nowMs = Date.now(), overrides = {}) {
  const maxAttempts = Number.isFinite(overrides.maxAttempts)
    ? Math.max(1, Math.floor(overrides.maxAttempts))
    : DEFAULT_STARTUP_MAX_ATTEMPTS;
  const maxWindowMs = Number.isFinite(overrides.maxWindowMs)
    ? Math.max(1000, Math.floor(overrides.maxWindowMs))
    : DEFAULT_STARTUP_MAX_WINDOW_MS;
  const baseDelayMs = Number.isFinite(overrides.baseDelayMs)
    ? Math.max(200, Math.floor(overrides.baseDelayMs))
    : DEFAULT_STARTUP_BASE_DELAY_MS;
  const maxDelayMs = Number.isFinite(overrides.maxDelayMs)
    ? Math.max(baseDelayMs, Math.floor(overrides.maxDelayMs))
    : DEFAULT_STARTUP_MAX_DELAY_MS;

  const initialStartedAtMs = Number(previousState?.startedAtMs) || 0;
  const startedAtMs = initialStartedAtMs > 0 ? initialStartedAtMs : nowMs;
  const attempts = (Number(previousState?.attempts) || 0) + 1;
  const elapsedMs = Math.max(0, nowMs - startedAtMs);
  const exhausted = attempts >= maxAttempts || elapsedMs >= maxWindowMs;

  const backoffStage = Math.max(0, Math.floor((attempts - 1) / 4));
  const nextDelayMs = Math.min(maxDelayMs, baseDelayMs * (2 ** backoffStage));

  return {
    startedAtMs,
    attempts,
    elapsedMs,
    exhausted,
    nextDelayMs,
  };
}
