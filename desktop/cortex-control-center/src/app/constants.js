// Matches daemon-rs/src/main.rs:DEFAULT_CORTEX_PORT. Bump both simultaneously.
export const DEFAULT_CORTEX_PORT = 7437;
export const DEFAULT_CORTEX_BASE = `http://127.0.0.1:${DEFAULT_CORTEX_PORT}`;
export const FALLBACK_REFRESH_MS = 15000;
export const ANALYTICS_REFRESH_MS = 60000;
export const SSE_REFRESH_THROTTLE_MS = 300;
export const CORE_REFRESH_MIN_INTERVAL_MS = 1200;
export const SECONDARY_REFRESH_MIN_INTERVAL_MS = 12000;
export const SSE_RECONNECT_BASE_MS = 1000;
export const SSE_RECONNECT_MAX_MS = 5000;
export const DAEMON_START_WAIT_TIMEOUT_MS = 25000;
export const DAEMON_START_POLL_INTERVAL_MS = 600;
export const DAEMON_START_STILL_STARTING_GRACE_MS = 9000;
export const DAEMON_STOP_HANG_TIMEOUT_MS = 5000;
export const DAEMON_STOP_WAIT_TIMEOUT_MS = 15000;
export const SAVINGS_USD_PER_MILLION = 15;
export const SAVINGS_HISTORY_DAYS = 30;
export const SIDEBAR_COLLAPSE_BREAKPOINT_PX = 1100;

export const FEED_KIND_LABEL = {
  prompt: "Prompt",
  completion: "Completion",
  task_complete: "Task Complete",
  system: "System",
};

export const PANEL_SEQUENCE = [
  { key: "overview", label: "Overview", icon: "overview" },
  { key: "analytics", label: "Analytics", icon: "analytics" },
  { key: "agents", label: "Agents", icon: "agents" },
  { key: "work", label: "Work", icon: "work" },
  { key: "memory", label: "Memory", icon: "memory" },
  { key: "brain", label: "Brain", icon: "brain" },
  { key: "settings", label: "Settings", icon: "settings" },
  { key: "about", label: "About", icon: "about" },
];
export const PANEL_SEQUENCE_INDEX = new Map(PANEL_SEQUENCE.map((entry, idx) => [entry.key, idx]));
export const PANEL_SEQUENCE_LABEL = new Map(PANEL_SEQUENCE.map((entry) => [entry.key, entry.label]));
export const PANEL_SEQUENCE_KEYS = new Set(PANEL_SEQUENCE.map((entry) => entry.key));

export function panelIndex(panelKey) {
  return PANEL_SEQUENCE_INDEX.get(panelKey) ?? -1;
}

export const EMPTY_DAEMON = {
  running: false,
  reachable: false,
  managed: false,
  authTokenReady: false,
  pid: null,
  message: "Checking daemon...",
};

export const EMPTY_HEALTH_META = {
  status: "unknown",
  degraded: false,
  dbCorrupted: false,
  runtimeVersion: "",
  budgets: null,
};

export const CONTROL_CENTER_VERSION = "0.6.0";
export const RECALL_HEADLINE_MIN_QUERIES = 20;
export const CORTEX_BASE_STORAGE_KEY = "cortex_base";
export const CORTEX_AUTH_STORAGE_KEY = "cortex_auth_token";
export const LEGACY_CORTEX_AUTH_STORAGE_KEYS = ["cortex_token"];
export const CORTEX_OPERATOR_STORAGE_KEY = "cortex_operator";
export const CORTEX_PANEL_STORAGE_KEY = "cortex_panel";
export const DEV_RESTART_VERIFY_ENABLED = import.meta.env.VITE_CORTEX_DEV_VERIFY_RESTART === "1";
export const DEV_RESTART_VERIFY_TIMEOUT_MS = 30000;
export const MISSION_METRIC_LEGEND = [
  { abbreviation: "t", meaning: "tokens" },
  { abbreviation: "K", meaning: "thousand tokens" },
  { abbreviation: "M", meaning: "million tokens" },
  { abbreviation: "B", meaning: "billion tokens" },
  { abbreviation: "T", meaning: "trillion tokens" },
];

export const ANALYTICS_METRIC_LEGEND = [
  { label: "Compounding return", meaning: "30-day total boot tokens saved" },
  { label: "Efficiency", meaning: "Average compression over the same 30-day window" },
  { label: "Throughput", meaning: "Boot compilations counted over the last 7 calendar days" },
  { label: "Compiled context", meaning: "30-day total prompt tokens served at boot" },
  { label: "Economic value", meaning: "Estimated currency value based on saved tokens" },
];
