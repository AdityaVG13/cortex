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
