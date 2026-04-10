import { useCallback, useEffect, useMemo, useRef, useState, Component } from "react";
import { BrainVisualizer } from "./BrainVisualizer.jsx";
import { checkForUpdates, installUpdate } from "./updater.js";
import { createApi, createPostApi, settledWithRethrow, settledCollectErrors } from "./api-client.js";
import { CURRENCY_OPTIONS, USD_TO_CURRENCY_RATE, SAVINGS_OPERATION_LABELS } from "./constants.js";

class BrainErrorBoundary extends Component {
  constructor(props) { super(props); this.state = { crashed: false, error: "" }; }
  static getDerivedStateFromError(err) { return { crashed: true, error: err?.message || "Unknown error" }; }
  render() {
    if (this.state.crashed) return (
      <div className="brain-loading">
        <div className="coming-icon" style={{ fontSize: 48 }}>◬</div>
        <p>Brain visualizer crashed: {this.state.error}</p>
        <button className="btn-sm btn-primary" onClick={() => this.setState({ crashed: false })} style={{ marginTop: 12 }}>Retry</button>
      </div>
    );
    return this.props.children;
  }
}

const DEFAULT_CORTEX_BASE = "http://127.0.0.1:7437";
const FALLBACK_REFRESH_MS = 5000;
const ANALYTICS_REFRESH_MS = 60000;
const SSE_REFRESH_THROTTLE_MS = 300;
const SSE_RECONNECT_BASE_MS = 1000;
const SSE_RECONNECT_MAX_MS = 5000;
const DAEMON_START_WAIT_TIMEOUT_MS = 90000;
const DAEMON_START_POLL_INTERVAL_MS = 750;
const DAEMON_STOP_HANG_TIMEOUT_MS = 5000;
const DAEMON_STOP_WAIT_TIMEOUT_MS = 15000;
const SAVINGS_USD_PER_MILLION = 15;

const FEED_KIND_LABEL = {
  prompt: "Prompt",
  completion: "Completion",
  task_complete: "Task Complete",
  system: "System",
};

const PANELS = [
  { key: "overview", label: "Overview", icon: "◈" },
  { key: "memory", label: "Memory", icon: "⬡" },
  { key: "analytics", label: "Analytics", icon: "△" },
  { key: "agents", label: "Agents", icon: "◉" },
  { key: "tasks", label: "Tasks", icon: "▣" },
  { key: "feed", label: "Feed", icon: "◫" },
  { key: "messages", label: "Messages", icon: "◧" },
  { key: "activity", label: "Activity", icon: "◍" },
  { key: "locks", label: "Locks", icon: "◎" },
  { key: "visualizer", label: "Brain", icon: "◬" },
  { key: "conflicts", label: "Conflicts", icon: "⚡" },
  { key: "about", label: "About", icon: "ℹ" },
];

const EMPTY_DAEMON = {
  running: false,
  reachable: false,
  pid: null,
  message: "Checking daemon...",
};

function timeAgo(iso) {
  if (!iso) return "unknown";
  const minutes = Math.floor((Date.now() - new Date(iso).getTime()) / 60000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)}h`;
  return `${Math.floor(minutes / 1440)}d`;
}

function priorityRank(priority) {
  const map = { critical: 4, high: 3, medium: 2, low: 1 };
  return map[priority] || 0;
}

async function readTauriInvoke() {
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke;
  } catch {
    return null;
  }
}

function statusPill(daemonState) {
  if (daemonState.reachable) return { className: "pill online", label: "Online" };
  return { className: "pill offline", label: "Offline" };
}

function feedKindLabel(kind) {
  return FEED_KIND_LABEL[kind] || kind || "Unknown";
}

function AnimatedNumber({ value, duration = 600 }) {
  const [display, setDisplay] = useState(value);
  const prevRef = useRef(value);

  useEffect(() => {
    const from = typeof prevRef.current === "number" ? prevRef.current : 0;
    const to = typeof value === "number" ? value : 0;
    if (from === to || typeof value !== "number") {
      setDisplay(value);
      prevRef.current = value;
      return;
    }

    let cancelled = false;
    const start = performance.now();
    const diff = to - from;

    function tick(now) {
      if (cancelled) return;
      const elapsed = now - start;
      const progress = Math.min(elapsed / duration, 1);
      const eased = 1 - Math.pow(1 - progress, 3);
      setDisplay(Math.round(from + diff * eased));
      if (progress < 1) requestAnimationFrame(tick);
    }

    requestAnimationFrame(tick);
    prevRef.current = to;
    return () => { cancelled = true; };
  }, [value, duration]);

  return <>{typeof display === "number" ? display.toLocaleString() : display}</>;
}

let sparklineCounter = 0;

function Sparkline({ data, width = 280, height = 60, color = "var(--cyan)" }) {
  const [id] = useState(() => `spark-fill-${++sparklineCounter}`);
  if (!data || data.length < 2) return <div className="sparkline-empty">No data yet</div>;
  const max = Math.max(...data, 1);
  const min = Math.min(...data, 0);
  const range = max - min || 1;
  const points = data.map((v, i) => {
    const x = (i / (data.length - 1)) * width;
    const y = height - ((v - min) / range) * (height - 4) - 2;
    return `${x},${y}`;
  }).join(" ");

  return (
    <svg width={width} height={height} className="sparkline">
      <defs>
        <linearGradient id={id} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity="0.2" />
          <stop offset="100%" stopColor={color} stopOpacity="0" />
        </linearGradient>
      </defs>
      <polygon
        points={`0,${height} ${points} ${width},${height}`}
        fill={`url(#${id})`}
      />
      <polyline
        points={points}
        fill="none"
        stroke={color}
        strokeWidth="1.5"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function ComingSoon({ title, description }) {
  return (
    <section className="panel active">
      <div className="panel-header">
        <h1>{title}</h1>
      </div>
      <div className="coming-soon">
        <div className="coming-icon">◬</div>
        <h2>COMING SOON</h2>
        <p>{description}</p>
      </div>
    </section>
  );
}

function EmptyItem({ text }) {
  return <li className="empty">{text}</li>;
}

function agentColor(name) {
  if (!name) return "var(--cyan)";
  const n = name.toLowerCase();
  if (n.includes("claude")) return "var(--agent-claude)";
  if (n.includes("droid") || n.includes("factory")) return "var(--agent-droid)";
  if (n.includes("gemini")) return "var(--agent-gemini)";
  if (n.includes("qwen") || n.includes("deepseek")) return "#22c55e";
  return "var(--cyan)";
}

function AgentItem({ session }) {
  const color = agentColor(session.agent);
  return (
    <li>
      <div className="agent-row">
        <span className="agent-indicator" style={{ background: color, boxShadow: `0 0 8px ${color}` }} />
        <span className="item-name">{session.agent}</span>
        <span className="agent-pulse" style={{ color }}>ACTIVE</span>
      </div>
      <div className="item-detail">
        {session.description || "Working"} · {session.project || "—"}
      </div>
      <div className="item-meta">
        <span className="mono-inline">
          {(session.files || []).slice(0, 4).map((file) => (
            <span key={file} className="lock-path">
              {file}
            </span>
          ))}
        </span>
        <span className="muted-inline">{timeAgo(session.lastHeartbeat)}</span>
      </div>
    </li>
  );
}

function TaskItem({ task }) {
  const detail = task.claimedBy
    ? `${task.claimedBy}${task.summary ? ` — ${task.summary}` : ""} · ${timeAgo(task.claimedAt || task.completedAt)}`
    : task.project || "—";

  return (
    <li>
      <div className="task-top">
        <span className={`status-dot ${task.status}`} />
        <span className={`priority priority-${task.priority}`}>{task.priority}</span>
        <span className="item-name">{task.title}</span>
      </div>
      <div className="item-detail">{detail}</div>
    </li>
  );
}

function LockItem({ lock }) {
  const expiryMinutes = Math.max(
    0,
    Math.ceil((new Date(lock.expiresAt).getTime() - Date.now()) / 60000)
  );

  return (
    <li>
      <div className="lock-path">{lock.path}</div>
      <div className="item-meta">
        <span className="lock-agent">{lock.agent}</span>
        <span className="lock-expiry">{expiryMinutes}m remaining</span>
      </div>
    </li>
  );
}

function FeedItem({ entry }) {
  const files = Array.isArray(entry.files) ? entry.files.slice(0, 6) : [];
  const metaBits = [timeAgo(entry.timestamp)];
  if (entry.priority) metaBits.push(entry.priority);
  if (typeof entry.tokens === "number") metaBits.push(`${entry.tokens} tok`);

  return (
    <li>
      <div className="item-meta">
        <span className="feed-kind">{feedKindLabel(entry.kind)}</span>
        <span className="item-name">{entry.agent || "unknown"}</span>
        <span className="muted-inline">{metaBits.join(" · ")}</span>
      </div>
      <div className="feed-summary">{entry.summary || "(no summary)"}</div>
      {entry.taskId ? <div className="item-detail">task: {entry.taskId}</div> : null}
      {files.length ? (
        <div className="feed-files">
          {files.map((file) => (
            <span key={`${entry.id}-${file}`} className="lock-path">
              {file}
            </span>
          ))}
        </div>
      ) : null}
    </li>
  );
}

function MessageItem({ entry }) {
  const fromColor = agentColor(entry.from);
  return (
    <li className="msg-bubble">
      <div className="msg-header">
        <span className="msg-agent" style={{ color: fromColor }}>
          <span className="agent-indicator" style={{ background: fromColor, boxShadow: `0 0 6px ${fromColor}`, display: "inline-block", width: 6, height: 6, borderRadius: "50%", marginRight: 6, verticalAlign: "middle" }} />
          {entry.from || "unknown"}
        </span>
        <span className="msg-arrow">→</span>
        <span className="msg-to">{entry.to || "unknown"}</span>
        <span className="muted-inline">{timeAgo(entry.timestamp)}</span>
      </div>
      <div className="msg-body">{entry.message || "(empty message)"}</div>
    </li>
  );
}

function ActivityItem({ entry }) {
  const files = Array.isArray(entry.files) ? entry.files.slice(0, 6) : [];

  return (
    <li>
      <div className="item-meta">
        <span className="item-name">{entry.agent || "unknown"}</span>
        <span className="muted-inline">{timeAgo(entry.timestamp)}</span>
      </div>
      <div className="feed-summary">{entry.description || "(no activity details)"}</div>
      {files.length ? (
        <div className="feed-files">
          {files.map((file) => (
            <span key={`${entry.id}-${file}`} className="lock-path">
              {file}
            </span>
          ))}
        </div>
      ) : null}
    </li>
  );
}

function normalizeSession(session, index) {
  const files = Array.isArray(session?.files)
    ? session.files
    : Array.isArray(session?.files_json)
      ? session.files_json
      : [];
  const startedAt = session?.startedAt ?? session?.started_at ?? null;
  const lastHeartbeat = session?.lastHeartbeat ?? session?.last_heartbeat ?? startedAt;
  const expiresAt = session?.expiresAt ?? session?.expires_at ?? null;
  const sessionId = session?.sessionId ?? session?.session_id ?? `${session?.agent || "agent"}-${index}`;

  return {
    ...session,
    files,
    sessionId,
    startedAt,
    lastHeartbeat,
    expiresAt,
  };
}

function isDaemonOfflineErrorMessage(message) {
  const value = String(message || "").toLowerCase();
  return (
    value.includes("cannot connect to daemon") ||
    value.includes("cannot reach daemon") ||
    value.includes("actively refused") ||
    value.includes("os error 10061") ||
    value.includes("connection refused")
  );
}

export function App() {
  const [panel, setPanel] = useState("overview");
  const [daemonState, setDaemonState] = useState(EMPTY_DAEMON);
  const [stats, setStats] = useState({
    memories: "--",
    decisions: "--",
    events: "--",
  });
  const [sessions, setSessions] = useState([]);
  const [tasks, setTasks] = useState([]);
  const [locks, setLocks] = useState([]);
  const [feedEntries, setFeedEntries] = useState([]);
  const [messageEntries, setMessageEntries] = useState([]);
  const [activityEntries, setActivityEntries] = useState([]);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [savings, setSavings] = useState(null);
  const [memoryQuery, setMemoryQuery] = useState("");
  const [memoryResults, setMemoryResults] = useState([]);
  const [memorySearching, setMemorySearching] = useState(false);
  const [feedFilters, setFeedFilters] = useState({
    since: "1h",
    kind: "all",
    agent: "",
    unread: false,
  });
  const [messageAgent, setMessageAgent] = useState("");
  const [activitySince, setActivitySince] = useState("1h");
  const [feedbackMessage, setFeedbackMessage] = useState("Checking daemon...");
  const [conflictPairs, setConflictPairs] = useState([]);
  const [conflictLoading, setConflictLoading] = useState(false);
  const [editorSetup, setEditorSetup] = useState(null);
  const [cortexBase, setCortexBase] = useState(() => localStorage.getItem("cortex_base") || DEFAULT_CORTEX_BASE);
  const [showConnectionDialog, setShowConnectionDialog] = useState(false);
  const [availableUpdate, setAvailableUpdate] = useState(null);
  const [updateInstalling, setUpdateInstalling] = useState(false);
  const [restartingDaemon, setRestartingDaemon] = useState(false);
  const [restartError, setRestartError] = useState("");
  const [currency, setCurrency] = useState(() => localStorage.getItem("cortex_currency") || "USD");
  const [analyticsMode, setAnalyticsMode] = useState(() => localStorage.getItem("cortex_analytics_mode") || "aggregate");

  const invokeRef = useRef(null);
  const tokenRef = useRef("");
  const refreshAllRef = useRef(async () => {});
  const daemonTransitionRef = useRef(false);

  const normalizedSessions = useMemo(() => {
    if (!Array.isArray(sessions)) return [];
    const sorted = sessions
      .map((session, index) => normalizeSession(session, index))
      .sort((a, b) => {
        const aTs = new Date(a.lastHeartbeat || 0).getTime();
        const bTs = new Date(b.lastHeartbeat || 0).getTime();
        return bTs - aTs;
      });

    const deduped = new Map();
    for (const session of sorted) {
      const agentRaw = String(session?.agent || "").trim();
      if (!agentRaw) {
        deduped.set(session.sessionId || `session-${deduped.size}`, session);
        continue;
      }
      const base = agentRaw.replace(/\s*\([^)]*\)\s*$/, "").trim().toLowerCase();
      const key = base === "droid" ? "droid" : agentRaw.toLowerCase();
      const existing = deduped.get(key);
      if (!existing) {
        deduped.set(key, session);
        continue;
      }
      const existingHasModel = /\([^)]+\)/.test(String(existing.agent || ""));
      const currentHasModel = /\([^)]+\)/.test(agentRaw);
      if (currentHasModel && !existingHasModel) {
        deduped.set(key, session);
      }
    }

    return Array.from(deduped.values());
  }, [sessions]);

  const knownAgents = useMemo(() => {
    const allAgents = new Set(normalizedSessions.map((session) => session.agent).filter(Boolean));
    if (messageAgent.trim()) allAgents.add(messageAgent.trim());
    return Array.from(allAgents).sort((a, b) => a.localeCompare(b));
  }, [normalizedSessions, messageAgent]);

  const currencyRate = USD_TO_CURRENCY_RATE[currency] ?? USD_TO_CURRENCY_RATE.USD;

  const currencyFormatter = useMemo(
    () =>
      new Intl.NumberFormat(undefined, {
        style: "currency",
        currency,
        maximumFractionDigits: currency === "JPY" || currency === "KRW" ? 0 : 2,
      }),
    [currency]
  );

  const formatCurrency = useCallback(
    (usdAmount) => currencyFormatter.format((Number(usdAmount) || 0) * currencyRate),
    [currencyFormatter, currencyRate]
  );

  const refreshTokenForApi = useCallback(async () => {
    if (!invokeRef.current) return;
    try {
      const token = await invokeRef.current("read_auth_token");
      tokenRef.current = token || "";
    } catch { /* ignore */ }
  }, []);

  const api = useCallback(
    createApi({
      getInvoke: () => invokeRef.current,
      getToken: () => tokenRef.current,
      cortexBase,
      onTokenRefresh: refreshTokenForApi,
    }),
    [cortexBase, refreshTokenForApi]
  );

  const postApi = useCallback(
    createPostApi({
      getInvoke: () => invokeRef.current,
      getToken: () => tokenRef.current,
      cortexBase,
      onTokenRefresh: refreshTokenForApi,
    }),
    [cortexBase, refreshTokenForApi]
  );

  const call = useCallback(async (command, args = {}) => {
    if (!invokeRef.current) throw new Error("No Tauri IPC available");
    return invokeRef.current(command, args);
  }, []);

  const readAuthToken = useCallback(async () => {
    if (invokeRef.current) {
      try {
        const token = await call("read_auth_token");
        tokenRef.current = token || "";
        return;
      } catch (err) {
        tokenRef.current = "";
        const message = err?.message || String(err);
        if (!daemonTransitionRef.current || !isDaemonOfflineErrorMessage(message)) {
          setFeedbackMessage(`Auth token read failed: ${message}`);
        }
      }
    }
  }, [call]);

  const refreshDaemonState = useCallback(async () => {
    if (invokeRef.current) {
      try {
        const state = await call("daemon_status");
        setDaemonState(state);
        return;
      } catch {
        // fallback to HTTP health
      }
    }

    let health;
    try {
      health = await api("/health");
    } catch {
      // daemon unreachable is an expected state, not an error
    }
    if (health?.status === "ok") {
      const nextState = {
        running: true,
        reachable: true,
        pid: null,
        message: `Connected -- ${health.stats?.memories ?? 0} memories`,
      };
      setDaemonState(nextState);
    } else {
      const nextState = {
        running: false,
        reachable: false,
        pid: null,
        message: "Cannot reach daemon on :7437",
      };
      setDaemonState(nextState);
    }
  }, [api, call]);

  const refreshHealth = useCallback(async () => {
    let health;
    try {
      health = await api("/health");
    } catch {
      // daemon unreachable -- show dashes
    }
    if (!health?.stats) {
      setStats({
        memories: "--",
        decisions: "--",
        events: "--",
      });
      return;
    }

    const next = health.stats;
    setStats({
      memories: next.memories ?? 0,
      decisions: next.decisions ?? 0,
      events: next.events ?? 0,
    });
  }, [api]);

  const refreshCoreData = useCallback(async () => {
    await settledWithRethrow([
      {
        fn: () => api("/sessions", true),
        apply: (v) => setSessions(Array.isArray(v?.sessions) ? v.sessions : []),
      },
      {
        fn: () => api("/locks", true),
        apply: (v) => setLocks(Array.isArray(v?.locks) ? v.locks : []),
      },
      {
        fn: () => api("/tasks?status=all", true),
        apply: (v) => setTasks(Array.isArray(v?.tasks) ? v.tasks : []),
      },
    ]);
  }, [api]);

  const refreshFeed = useCallback(async () => {
    const query = new URLSearchParams();
    query.set("since", feedFilters.since);
    if (feedFilters.kind !== "all") query.set("kind", feedFilters.kind);
    if (feedFilters.agent.trim()) query.set("agent", feedFilters.agent.trim());
    if (feedFilters.unread && feedFilters.agent.trim()) query.set("unread", "true");

    const feedResult = await api(`/feed?${query.toString()}`, true);
    const entries = Array.isArray(feedResult?.entries) ? [...feedResult.entries].reverse() : [];
    setFeedEntries(entries);
  }, [api, feedFilters]);

  const refreshMessages = useCallback(async () => {
    const targetAgent = messageAgent.trim();
    if (!targetAgent) {
      setMessageEntries([]);
      return;
    }

    const query = new URLSearchParams();
    query.set("agent", targetAgent);
    const result = await api(`/messages?${query.toString()}`, true);
    const entries = Array.isArray(result?.messages) ? [...result.messages].reverse() : [];
    setMessageEntries(entries);
  }, [api, messageAgent]);

  const refreshActivity = useCallback(async () => {
    const query = new URLSearchParams();
    query.set("since", activitySince);
    const result = await api(`/activity?${query.toString()}`, true);
    const entries = Array.isArray(result?.activities) ? [...result.activities].reverse() : [];
    setActivityEntries(entries);
  }, [api, activitySince]);

  const refreshSavings = useCallback(async () => {
    const result = await api("/savings", true);
    if (result) setSavings(result);
  }, [api]);

  const refreshConflicts = useCallback(async () => {
    const result = await api("/conflicts", true);
    setConflictPairs(Array.isArray(result?.pairs) ? result.pairs : []);
  }, [api]);

  const handleResolveConflict = useCallback(async (keepId, action, supersededId) => {
    setConflictLoading(true);
    try {
      await postApi("/resolve", { keepId, action, supersededId });
      await refreshConflicts();
    } catch (err) {
      setFeedbackMessage(`Resolve failed: ${err.message || err}`);
    } finally {
      setConflictLoading(false);
    }
  }, [postApi, refreshConflicts]);

  const handleSetupEditors = useCallback(async () => {
    try {
      const result = await call("setup_editors");
      setEditorSetup(result);
      const registered = result.filter(e => e.registered).length;
      setFeedbackMessage(`Registered Cortex MCP in ${registered} editor(s)`);
    } catch (err) {
      setFeedbackMessage(`Editor setup: ${String(err)}`);
    }
  }, [call]);

  const refreshAll = useCallback(async () => {
    try {
      invokeRef.current = await readTauriInvoke();
    } catch {
      invokeRef.current = null;
    }
    await readAuthToken();

    if (daemonTransitionRef.current) {
      const transitionErrors = await settledCollectErrors([
        refreshDaemonState,
        refreshHealth,
      ]);
      if (
        transitionErrors.length &&
        !transitionErrors.every((error) => isDaemonOfflineErrorMessage(error))
      ) {
        const unique = [...new Set(transitionErrors)];
        setFeedbackMessage(unique.join("; "));
      }
      return;
    }

    const errors = await settledCollectErrors([
      refreshDaemonState,
      refreshHealth,
      refreshCoreData,
      refreshFeed,
      refreshMessages,
      refreshActivity,
      refreshSavings,
      refreshConflicts,
    ]);
    if (errors.length) {
      const unique = [...new Set(errors)];
      if (!unique.every((error) => isDaemonOfflineErrorMessage(error))) {
        setFeedbackMessage(unique.join("; "));
      }
    }
  }, [
    readAuthToken,
    refreshDaemonState,
    refreshHealth,
    refreshCoreData,
    refreshFeed,
    refreshMessages,
    refreshActivity,
    refreshSavings,
    refreshConflicts,
  ]);

  useEffect(() => {
    localStorage.setItem("cortex_base", cortexBase);
    refreshAllRef.current();
  }, [cortexBase]);

  useEffect(() => {
    localStorage.setItem("cortex_currency", currency);
  }, [currency]);

  useEffect(() => {
    localStorage.setItem("cortex_analytics_mode", analyticsMode);
  }, [analyticsMode]);

  useEffect(() => {
    refreshAllRef.current = refreshAll;
  }, [refreshAll]);

  useEffect(() => {
    // Call refreshAll directly on mount -- refreshAllRef.current isn't assigned
    // yet when this effect fires (ref-assignment effect hasn't run).
    refreshAll();
    const interval = setInterval(() => {
      refreshAllRef.current();
    }, FALLBACK_REFRESH_MS);
    return () => clearInterval(interval);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    checkForUpdates().then((update) => {
      if (update) setAvailableUpdate(update);
    });
  }, []);

  useEffect(() => {
    if (messageAgent.trim()) return;
    const defaultAgent = normalizedSessions.find((session) => session.agent)?.agent;
    if (defaultAgent) setMessageAgent(defaultAgent);
  }, [normalizedSessions, messageAgent]);

  useEffect(() => {
    refreshMessages().catch(err => setFeedbackMessage(`Messages: ${err.message || err}`));
  }, [refreshMessages]);

  useEffect(() => {
    refreshActivity().catch(err => setFeedbackMessage(`Activity: ${err.message || err}`));
  }, [refreshActivity]);

  useEffect(() => {
    if (panel !== "analytics") return;
    refreshSavings().catch(err => setFeedbackMessage(`Savings: ${err.message || err}`));
    const timer = setInterval(() => {
      refreshSavings().catch(err => setFeedbackMessage(`Savings: ${err.message || err}`));
    }, ANALYTICS_REFRESH_MS);
    return () => clearInterval(timer);
  }, [panel, refreshSavings]);

  useEffect(() => {
    let stream = null;
    let refreshTimer = null;
    let reconnectTimer = null;
    let reconnectAttempt = 0;
    let lastRefreshAt = 0;
    let refreshInFlight = false;
    let refreshQueued = false;
    let disposed = false;

    const clearRefreshTimer = () => {
      if (refreshTimer) {
        window.clearTimeout(refreshTimer);
        refreshTimer = null;
      }
    };

    const clearReconnectTimer = () => {
      if (reconnectTimer) {
        window.clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
    };

    const scheduleRefresh = (immediate = false) => {
      if (disposed || refreshTimer) return;
      const elapsed = Date.now() - lastRefreshAt;
      const delay = immediate ? 0 : Math.max(SSE_REFRESH_THROTTLE_MS - elapsed, 0);

      refreshTimer = window.setTimeout(() => {
        refreshTimer = null;
        if (refreshInFlight) {
          refreshQueued = true;
          return;
        }

        refreshInFlight = true;
        Promise.resolve(refreshAllRef.current())
          .finally(() => {
            lastRefreshAt = Date.now();
            refreshInFlight = false;
            if (refreshQueued && !disposed) {
              refreshQueued = false;
              scheduleRefresh();
            }
          });
      }, delay);
    };

    const handleRealtimeEvent = () => {
      scheduleRefresh();
    };

    const closeStream = () => {
      if (!stream) return;
      stream.close();
      stream = null;
    };

    const scheduleReconnect = () => {
      if (disposed) return;
      const exponentialDelay = Math.min(
        SSE_RECONNECT_MAX_MS,
        SSE_RECONNECT_BASE_MS * 2 ** reconnectAttempt
      );
      const jitter = Math.floor(Math.random() * 250);
      reconnectAttempt += 1;

      clearReconnectTimer();
      reconnectTimer = window.setTimeout(() => {
        reconnectTimer = null;
        connect();
      }, exponentialDelay + jitter);
    };

    const connect = () => {
      if (disposed || stream) return;
      const nextStream = new EventSource(`${cortexBase}/events/stream`);
      stream = nextStream;

      nextStream.onopen = () => {
        reconnectAttempt = 0;
        scheduleRefresh(true);
      };

      nextStream.onmessage = handleRealtimeEvent;
      nextStream.addEventListener("connected", handleRealtimeEvent);
      nextStream.addEventListener("task", handleRealtimeEvent);
      nextStream.addEventListener("session", handleRealtimeEvent);
      nextStream.addEventListener("lock", handleRealtimeEvent);
      nextStream.addEventListener("feed", handleRealtimeEvent);
      nextStream.addEventListener("message", handleRealtimeEvent);
      nextStream.addEventListener("activity", handleRealtimeEvent);

      nextStream.onerror = () => {
        if (disposed || stream !== nextStream) return;
        handleRealtimeEvent();
        closeStream();
        scheduleReconnect();
      };
    };

    const handleOnline = () => {
      if (disposed) return;
      reconnectAttempt = 0;
      clearReconnectTimer();
      closeStream();
      connect();
      scheduleRefresh(true);
    };

    connect();
    window.addEventListener("online", handleOnline);

    return () => {
      disposed = true;
      window.removeEventListener("online", handleOnline);
      clearRefreshTimer();
      clearReconnectTimer();
      closeStream();
    };
  }, [cortexBase]);

  const pendingTasks = useMemo(
    () => tasks.filter((task) => task.status === "pending").sort((a, b) => priorityRank(b.priority) - priorityRank(a.priority)),
    [tasks]
  );
  const claimedTasks = useMemo(() => tasks.filter((task) => task.status === "claimed"), [tasks]);
  const completedTasks = useMemo(() => tasks.filter((task) => task.status === "completed"), [tasks]);
  const recentOverviewTasks = useMemo(() => [...claimedTasks, ...pendingTasks].slice(0, 5), [claimedTasks, pendingTasks]);
  const pill = statusPill(daemonState);

  const operationRows = useMemo(
    () => (Array.isArray(savings?.byOperation) ? savings.byOperation : []),
    [savings]
  );

  const operationMaxSaved = useMemo(
    () => Math.max(...operationRows.map((row) => Number(row.saved || 0)), 1),
    [operationRows]
  );

  const cumulativeSeries = useMemo(
    () => (Array.isArray(savings?.cumulative) ? savings.cumulative : []),
    [savings]
  );

  const recallTrendSeries = useMemo(
    () => (Array.isArray(savings?.recallTrend) ? savings.recallTrend : []),
    [savings]
  );

  const activityHeatmap = useMemo(
    () => (Array.isArray(savings?.activityHeatmap) ? savings.activityHeatmap : []),
    [savings]
  );

  const activityHeatmapLookup = useMemo(() => {
    const map = new Map();
    activityHeatmap.forEach((entry) => {
      map.set(`${entry.day}:${Number(entry.hour)}`, Number(entry.count || 0));
    });
    return map;
  }, [activityHeatmap]);

  const activityHeatmapMax = useMemo(
    () => Math.max(...activityHeatmap.map((entry) => Number(entry.count || 0)), 1),
    [activityHeatmap]
  );

  const waitForDaemonReachable = useCallback(async () => {
    const started = Date.now();
    while (Date.now() - started < DAEMON_START_WAIT_TIMEOUT_MS) {
      try {
        if (invokeRef.current) {
          const state = await call("daemon_status");
          setDaemonState(state);
          if (state?.reachable) return true;
        } else {
          const health = await api("/health");
          if (health?.status === "ok") return true;
        }
      } catch {
        // continue polling until timeout
      }
      await new Promise((resolve) => setTimeout(resolve, DAEMON_START_POLL_INTERVAL_MS));
    }
    return false;
  }, [api, call]);

  const waitForDaemonOffline = useCallback(async () => {
    const started = Date.now();
    while (Date.now() - started < DAEMON_STOP_WAIT_TIMEOUT_MS) {
      try {
        if (invokeRef.current) {
          const state = await call("daemon_status");
          setDaemonState(state);
          if (!state?.reachable) return true;
        } else {
          await api("/health");
        }
      } catch (error) {
        if (isDaemonOfflineErrorMessage(error?.message || error)) {
          return true;
        }
      }
      await new Promise((resolve) => setTimeout(resolve, DAEMON_START_POLL_INTERVAL_MS));
    }
    return false;
  }, [api, call]);

  async function handleMemorySearch(e) {
    e?.preventDefault();
    if (!memoryQuery.trim()) return;
    setMemorySearching(true);
    try {
      const peekResult = await api(`/peek?q=${encodeURIComponent(memoryQuery.trim())}&k=15`);
      setMemoryResults(peekResult?.matches || []);
    } catch {
      setMemoryResults([]);
    }
    setMemorySearching(false);
  }

  async function handleMemoryExpand(source) {
    try {
      const recallResult = await api(`/recall?q=${encodeURIComponent(source)}&k=3`);
      const match = recallResult?.results?.find(r => r.source === source);
      if (match) {
        setMemoryResults(prev => prev.map(m =>
          m.source === source ? { ...m, excerpt: match.excerpt, expanded: true } : m
        ));
      }
    } catch (err) {
      setFeedbackMessage(`Memory expand failed: ${err.message || err}`);
    }
  }

  async function handleStartDaemon() {
    if (!invokeRef.current) return;
    daemonTransitionRef.current = true;
    try {
      const result = await call("start_daemon");
      setFeedbackMessage(result.message || "Daemon start requested.");
      const reachable = await waitForDaemonReachable();
      if (!reachable) {
        setFeedbackMessage("Daemon is still starting. Reconnect will continue automatically.");
      }
      daemonTransitionRef.current = false;
      await readAuthToken();
      await refreshAll();
    } catch (error) {
      setFeedbackMessage(`Start failed: ${error.message || error}`);
    } finally {
      daemonTransitionRef.current = false;
    }
  }

  async function handleStopDaemon() {
    if (!invokeRef.current) return;
    daemonTransitionRef.current = true;
    try {
      const result = await call("stop_daemon");
      setFeedbackMessage(result.message || "Daemon stop requested.");
      tokenRef.current = "";
      daemonTransitionRef.current = false;
      await refreshAll();
    } catch (error) {
      setFeedbackMessage(`Stop failed: ${error.message || error}`);
    } finally {
      daemonTransitionRef.current = false;
    }
  }

  async function handleRestartDaemon() {
    if (!invokeRef.current || restartingDaemon) return;

    setRestartingDaemon(true);
    setRestartError("");
    daemonTransitionRef.current = true;

    try {
      const statusBefore = await call("daemon_status").catch(() => null);
      const shouldStop = Boolean(statusBefore?.running || statusBefore?.reachable);

      if (shouldStop) {
        setFeedbackMessage("Restarting daemon: stopping...");
        const stopPromise = call("stop_daemon").catch(() => ({ failed: true }));
        const stopResult = await Promise.race([
          stopPromise,
          new Promise((resolve) => setTimeout(() => resolve({ timedOut: true }), DAEMON_STOP_HANG_TIMEOUT_MS)),
        ]);
        if (stopResult?.timedOut) {
          setFeedbackMessage("Shutdown is taking longer than expected. Waiting for daemon to go offline...");
        }
        const stopped = await waitForDaemonOffline();
        if (!stopped) {
          setFeedbackMessage("Daemon stop is still in progress. Continuing with start attempt...");
        }
      } else {
        setFeedbackMessage("Daemon already stopped. Starting...");
      }

      setFeedbackMessage("Restarting daemon: starting...");
      const startResult = await call("start_daemon");
      if (startResult?.message) {
        setFeedbackMessage(startResult.message);
      }

      const reachable = await waitForDaemonReachable();
      if (!reachable) {
        throw new Error("Daemon did not become reachable after restart.");
      }

      daemonTransitionRef.current = false;
      await readAuthToken();
      await refreshAll();
      setFeedbackMessage("Daemon restarted successfully.");
    } catch (error) {
      const message = error?.message || String(error);
      setRestartError(message);
      setFeedbackMessage(`Restart failed: ${message}`);
    } finally {
      daemonTransitionRef.current = false;
      setRestartingDaemon(false);
    }
  }

  // Keyboard nav
  useEffect(() => {
    function handleKey(e) {
      if (e.target.tagName === "INPUT" || e.target.tagName === "SELECT" || e.target.tagName === "TEXTAREA") return;
      const idx = PANELS.findIndex(p => p.key === panel);
      if (e.key === "ArrowDown" || e.key === "j") {
        e.preventDefault();
        setPanel(PANELS[(idx + 1) % PANELS.length].key);
      } else if (e.key === "ArrowUp" || e.key === "k") {
        e.preventDefault();
        setPanel(PANELS[(idx - 1 + PANELS.length) % PANELS.length].key);
      } else {
        const num = parseInt(e.key);
        if (num >= 1 && num <= PANELS.length) {
          e.preventDefault();
          setPanel(PANELS[num - 1].key);
        }
      }
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [panel]);

  return (
    <div className={`app ${sidebarCollapsed ? "sidebar-collapsed" : ""}`}>
      <aside className={`sidebar ${sidebarCollapsed ? "collapsed" : ""}`}>
        <div className="sidebar-header">
          <div className="logo">
            <span>Cortex</span>
          </div>
          <div className={pill.className}>{pill.label}</div>
        </div>

        <nav className="sidebar-nav">
          {PANELS.map((item, idx) => (
            <button
              key={item.key}
              type="button"
              className={`nav-item ${panel === item.key ? "active" : ""}`}
              onClick={() => setPanel(item.key)}
              data-key={idx + 1}
            >
              <span style={{ opacity: 0.5, fontSize: "12px" }}>{item.icon}</span>
              {item.label}
            </button>
          ))}
        </nav>

        <div className="sidebar-footer">
          <div className="daemon-restart-row">
            <button
              type="button"
              className="btn-ctrl btn-restart"
              onClick={handleRestartDaemon}
              disabled={restartingDaemon}
            >
              {restartingDaemon ? "Restarting... ⟳" : "Restart"}
            </button>
          </div>
          <div className="daemon-controls-grid">
            <button type="button" className="btn-ctrl btn-primary" onClick={handleStartDaemon}>Start</button>
            <button type="button" className="btn-ctrl" onClick={handleStopDaemon}>Stop</button>
            <button type="button" className="btn-ctrl btn-danger" onClick={async () => {
              if (invokeRef.current) {
                try { await call("quit_app"); } catch { /* app is exiting */ }
              }
            }}>Exit</button>
          </div>
          {restartError ? (
            <button type="button" className="btn-sm btn-danger btn-restart-retry" onClick={handleRestartDaemon}>
              Retry Restart
            </button>
          ) : null}
          {availableUpdate && (
            <div className="update-banner">
              <span>v{availableUpdate.version} available</span>
              <button
                type="button"
                className="btn-sm btn-primary"
                disabled={updateInstalling}
                onClick={async () => {
                  setUpdateInstalling(true);
                  setFeedbackMessage("Downloading update...");
                  try {
                    await installUpdate(availableUpdate);
                  } catch (err) {
                    setFeedbackMessage(`Update failed: ${String(err)}`);
                    setUpdateInstalling(false);
                  }
                }}
              >
                {updateInstalling ? "Installing..." : "Update"}
              </button>
            </div>
          )}
          <p className="sidebar-status">{feedbackMessage}</p>
          <button type="button" className="btn-sidebar-collapse" onClick={() => setSidebarCollapsed(c => !c)}>
            {sidebarCollapsed ? "▶" : "◀"}
          </button>
        </div>
      </aside>

      <main className="content">
        <div className={`topbar ${panel === "overview" ? "topbar-hidden" : ""}`}>
          <div className="topbar-left">
            <span className="topbar-path">CORTEX</span>
            <span className="topbar-sep">/</span>
            <span className="topbar-current">{PANELS.find(p => p.key === panel)?.label.toUpperCase()}</span>
          </div>
          <div className="topbar-right">
            <span className="topbar-stat"><span className="topbar-label">MEM</span> {stats.memories}</span>
            <span className="topbar-stat"><span className="topbar-label">DEC</span> {stats.decisions}</span>
            <span className="topbar-stat"><span className="topbar-label">EVT</span> {stats.events}</span>
            <span className="topbar-stat"><span className="topbar-label">AGENTS</span> {normalizedSessions.length}</span>
            <span className="topbar-stat topbar-connection" onClick={() => setShowConnectionDialog(true)} title="Click to change connection">
              <span className="topbar-label">HOST</span>
              {cortexBase === DEFAULT_CORTEX_BASE ? "LOCAL" : (() => { try { return new URL(cortexBase).hostname; } catch { return "?"; } })()}
            </span>
            <span className={`topbar-status ${daemonState.reachable ? "online" : "offline"}`}>
              {daemonState.reachable ? "● ONLINE" : "○ OFFLINE"}
            </span>
          </div>
        </div>

        {showConnectionDialog && (
          <div className="connection-overlay" onClick={() => setShowConnectionDialog(false)}>
            <div className="connection-dialog" onClick={e => e.stopPropagation()}>
              <h2>Connection Settings</h2>
              <p className="connection-subtitle">Connect to a local or remote Cortex daemon</p>
              <form onSubmit={(e) => {
                e.preventDefault();
                const fd = new FormData(e.target);
                const host = fd.get("host")?.toString().trim() || "127.0.0.1";
                const port = fd.get("port")?.toString().trim() || "7437";
                const token = fd.get("token")?.toString().trim();
                setCortexBase(`http://${host}:${port}`);
                if (token) tokenRef.current = token;
                setShowConnectionDialog(false);
              }}>
                <label className="connection-field">
                  <span>Host</span>
                  <input name="host" defaultValue={(() => { try { return new URL(cortexBase).hostname; } catch { return "127.0.0.1"; } })()} placeholder="127.0.0.1" />
                </label>
                <label className="connection-field">
                  <span>Port</span>
                  <input name="port" defaultValue={(() => { try { return new URL(cortexBase).port || "7437"; } catch { return "7437"; } })()} placeholder="7437" />
                </label>
                <label className="connection-field">
                  <span>Auth Token</span>
                  <input name="token" type="password" placeholder="Leave blank for local (auto-read)" />
                </label>
                <div className="connection-actions">
                  <button type="button" className="btn-sm" onClick={() => {
                    setCortexBase(DEFAULT_CORTEX_BASE);
                    tokenRef.current = "";
                    setShowConnectionDialog(false);
                    readAuthToken();
                  }}>Reset to Local</button>
                  <button type="submit" className="btn-sm btn-primary">Connect</button>
                </div>
              </form>
            </div>
          </div>
        )}

        {panel === "overview" ? (
          <section className="panel active">
            <div className="panel-header">
              <h1>Overview</h1>
              <button type="button" className="btn-sm" onClick={refreshAll}>
                Refresh
              </button>
            </div>

            <div className="metrics">
              <div className="metric" data-accent="cyan">
                <span className="metric-value"><AnimatedNumber value={typeof stats.memories === "number" ? stats.memories : 0} /></span>
                <span className="metric-label">Memories</span>
                <span className="metric-icon">⬡</span>
              </div>
              <div className="metric" data-accent="blue">
                <span className="metric-value"><AnimatedNumber value={typeof stats.decisions === "number" ? stats.decisions : 0} /></span>
                <span className="metric-label">Decisions</span>
                <span className="metric-icon">◆</span>
              </div>
              <div className="metric" data-accent="purple">
                <span className="metric-value"><AnimatedNumber value={typeof stats.events === "number" ? stats.events : 0} /></span>
                <span className="metric-label">Events</span>
                <span className="metric-icon">◍</span>
              </div>
            </div>

            <div className="system-strip">
              <div className="sys-item">
                <span className="sys-label">DAEMON</span>
                <span className={`sys-value ${daemonState.reachable ? "sys-ok" : "sys-err"}`}>
                  {daemonState.reachable ? "RUNNING" : "OFFLINE"}
                </span>
              </div>
              <div className="sys-item">
                <span className="sys-label">EMBEDDINGS</span>
                <span className={`sys-value ${daemonState.reachable ? "sys-ok" : "sys-err"}`}>
                  {daemonState.reachable ? "ONNX ACTIVE" : "OFFLINE"}
                </span>
              </div>
              <div className="sys-item">
                <span className="sys-label">AGENTS</span>
                <span className="sys-value sys-ok">{normalizedSessions.length} CONNECTED</span>
              </div>
              <div className="sys-item">
                <span className="sys-label">LOCKS</span>
                <span className="sys-value">{locks.length} ACTIVE</span>
              </div>
              <div className="sys-item">
                <span className="sys-label">TASKS</span>
                <span className="sys-value">{pendingTasks.length} PENDING</span>
              </div>
              <div className="sys-item sys-item-action" onClick={handleSetupEditors} title="Auto-register Cortex MCP in detected editors">
                <span className="sys-label">MCP</span>
                <span className="sys-value">{editorSetup ? `${editorSetup.filter(e => e.registered).length} EDITORS` : "SETUP"}</span>
              </div>
            </div>

            <div className="overview-grid">
              <div className="card">
                <div className="card-header">
                  <h2>Active Agents</h2>
                  <span className="badge">{normalizedSessions.length}</span>
                </div>
                <ul className="item-list">
                  {normalizedSessions.length ? normalizedSessions.map((session) => <AgentItem key={session.sessionId || session.agent} session={session} />) : <EmptyItem text="No agents online" />}
                </ul>
              </div>

              <div className="card">
                <div className="card-header">
                  <h2>Recent Tasks</h2>
                  <span className="badge">{pendingTasks.length + claimedTasks.length}</span>
                </div>
                <ul className="item-list">
                  {recentOverviewTasks.length ? recentOverviewTasks.map((task) => <TaskItem key={task.taskId} task={task} />) : <EmptyItem text="No tasks" />}
                </ul>
              </div>
            </div>
          </section>
        ) : null}

        {panel === "agents" ? (
          <section className="panel active">
            <div className="panel-header">
              <h1>Agents</h1>
            </div>
            <div className="card full">
              <ul className="item-list">
                {normalizedSessions.length ? normalizedSessions.map((session) => <AgentItem key={session.sessionId || session.agent} session={session} />) : <EmptyItem text="No agents online" />}
              </ul>
            </div>
          </section>
        ) : null}

        {panel === "tasks" ? (
          <section className="panel active">
            <div className="panel-header">
              <h1>Task Board</h1>
            </div>
            <div className="task-columns">
              <div className="card">
                <div className="card-header">
                  <h2>Pending</h2>
                  <span className="badge">{pendingTasks.length}</span>
                </div>
                <ul className="item-list">
                  {pendingTasks.length ? pendingTasks.map((task) => <TaskItem key={task.taskId} task={task} />) : <EmptyItem text="No pending tasks" />}
                </ul>
              </div>

              <div className="card">
                <div className="card-header">
                  <h2>In Progress</h2>
                  <span className="badge">{claimedTasks.length}</span>
                </div>
                <ul className="item-list">
                  {claimedTasks.length ? claimedTasks.map((task) => <TaskItem key={task.taskId} task={task} />) : <EmptyItem text="Nothing in progress" />}
                </ul>
              </div>

              <div className="card">
                <div className="card-header">
                  <h2>Done</h2>
                  <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
                    <span className="badge">{completedTasks.length}</span>
                    {completedTasks.length > 0 && (
                      <button
                        type="button"
                        className="btn-sm"
                        onClick={async () => {
                          try {
                            const results = await Promise.allSettled(
                              completedTasks.filter(t => t?.taskId).map(t => postApi("/tasks/delete", { taskId: t.taskId }))
                            );
                            const failed = results.filter(r => r.status === "rejected");
                            if (failed.length) setFeedbackMessage(`${failed.length} task delete(s) failed: ${failed[0].reason}`);
                            await refreshAll();
                          } catch (err) {
                            setFeedbackMessage(`Clear tasks failed: ${err.message || err}`);
                          }
                        }}
                      >
                        Clear
                      </button>
                    )}
                  </div>
                </div>
                <ul className="item-list">
                  {completedTasks.length ? completedTasks.slice(0, 10).map((task) => <TaskItem key={task.taskId} task={task} />) : <EmptyItem text="No completed tasks" />}
                </ul>
              </div>
            </div>
          </section>
        ) : null}

        {panel === "feed" ? (
          <section className="panel active">
            <div className="panel-header">
              <h1>Shared Feed</h1>
            </div>
            <div className="card full">
              <div className="feed-toolbar">
                <label className="feed-control">
                  <span>Since</span>
                  <select
                    value={feedFilters.since}
                    onChange={(event) =>
                      setFeedFilters((current) => ({ ...current, since: event.target.value }))
                    }
                  >
                    <option value="15m">15m</option>
                    <option value="1h">1h</option>
                    <option value="4h">4h</option>
                    <option value="1d">1d</option>
                  </select>
                </label>
                <label className="feed-control">
                  <span>Kind</span>
                  <select
                    value={feedFilters.kind}
                    onChange={(event) =>
                      setFeedFilters((current) => ({ ...current, kind: event.target.value }))
                    }
                  >
                    <option value="all">All</option>
                    <option value="prompt">Prompt</option>
                    <option value="completion">Completion</option>
                    <option value="task_complete">Task Complete</option>
                    <option value="system">System</option>
                  </select>
                </label>
                <label className="feed-control">
                  <span>Agent</span>
                  <input
                    type="text"
                    placeholder="factory-droid"
                    value={feedFilters.agent}
                    onChange={(event) =>
                      setFeedFilters((current) => ({ ...current, agent: event.target.value }))
                    }
                  />
                </label>
                <label className="feed-control feed-control-check">
                  <input
                    type="checkbox"
                    checked={feedFilters.unread}
                    onChange={(event) =>
                      setFeedFilters((current) => ({ ...current, unread: event.target.checked }))
                    }
                  />
                  <span>Unread only</span>
                </label>
                <div className="feed-actions">
                  <span className="badge">{feedEntries.length}</span>
                  <button type="button" className="btn-sm" onClick={() => refreshFeed().catch(err => setFeedbackMessage(`Feed: ${err.message || err}`))}>
                    Refresh Feed
                  </button>
                </div>
              </div>
              <ul className="item-list">
                {feedEntries.length ? feedEntries.map((entry) => <FeedItem key={entry.id} entry={entry} />) : <EmptyItem text="No feed entries" />}
              </ul>
            </div>
          </section>
        ) : null}

        {panel === "messages" ? (
          <section className="panel active">
            <div className="panel-header">
              <h1>Messages</h1>
            </div>
            <div className="card full">
              <div className="surface-toolbar">
                <label className="feed-control">
                  <span>Agent</span>
                  <input
                    type="text"
                    list="message-agent-list"
                    placeholder="factory-droid"
                    value={messageAgent}
                    onChange={(event) => setMessageAgent(event.target.value)}
                  />
                  <datalist id="message-agent-list">
                    {knownAgents.map((agent) => (
                      <option key={agent} value={agent} />
                    ))}
                  </datalist>
                </label>
                <div className="surface-actions">
                  <span className="badge">{messageEntries.length}</span>
                  <button type="button" className="btn-sm" onClick={() => refreshMessages().catch(err => setFeedbackMessage(`Messages: ${err.message || err}`))}>
                    Refresh Messages
                  </button>
                </div>
              </div>
              <ul className="item-list">
                {!messageAgent.trim() ? (
                  <EmptyItem text="Select an agent to view messages" />
                ) : messageEntries.length ? (
                  messageEntries.map((entry) => <MessageItem key={entry.id} entry={entry} />)
                ) : (
                  <EmptyItem text={`No messages for ${messageAgent.trim()}`} />
                )}
              </ul>
            </div>
          </section>
        ) : null}

        {panel === "activity" ? (
          <section className="panel active">
            <div className="panel-header">
              <h1>Activity</h1>
            </div>
            <div className="card full">
              <div className="surface-toolbar">
                <label className="feed-control">
                  <span>Since</span>
                  <select
                    value={activitySince}
                    onChange={(event) => setActivitySince(event.target.value)}
                  >
                    <option value="15m">15m</option>
                    <option value="1h">1h</option>
                    <option value="4h">4h</option>
                    <option value="1d">1d</option>
                  </select>
                </label>
                <div className="surface-actions">
                  <span className="badge">{activityEntries.length}</span>
                  <button type="button" className="btn-sm" onClick={() => refreshActivity().catch(err => setFeedbackMessage(`Activity: ${err.message || err}`))}>
                    Refresh Activity
                  </button>
                </div>
              </div>
              <ul className="item-list">
                {activityEntries.length ? (
                  activityEntries.map((entry) => <ActivityItem key={entry.id} entry={entry} />)
                ) : (
                  <EmptyItem text="No recent activity" />
                )}
              </ul>
            </div>
          </section>
        ) : null}

        {panel === "memory" ? (
          <section className="panel active">
            <div className="panel-header">
              <h1>Memory Explorer</h1>
            </div>
            <div className="card full">
              <form className="memory-search" onSubmit={handleMemorySearch}>
                <input
                  type="text"
                  className="memory-input"
                  placeholder="Search the brain... (uses cortex_peek)"
                  value={memoryQuery}
                  onChange={(e) => setMemoryQuery(e.target.value)}
                />
                <button type="submit" className="btn-sm btn-primary" disabled={memorySearching}>
                  {memorySearching ? "Searching..." : "Peek"}
                </button>
              </form>
              {memoryResults.length > 0 && (
                <div className="memory-stats">
                  <span className="badge">{memoryResults.length} matches</span>
                  <span className="muted-inline">via cortex_peek — click to expand full recall</span>
                </div>
              )}
              <ul className="item-list">
                {memoryResults.length ? memoryResults.map((match, i) => (
                  <li key={`${match.source}-${i}`} className="memory-item" onClick={() => !match.expanded && handleMemoryExpand(match.source)}>
                    <div className="memory-header">
                      <span className="memory-method">{match.method}</span>
                      <span className="memory-relevance">{(match.relevance * 100).toFixed(0)}%</span>
                    </div>
                    <div className="memory-source">{match.source}</div>
                    {match.expanded && match.excerpt && (
                      <div className="memory-excerpt">{match.excerpt}</div>
                    )}
                    {!match.expanded && <div className="memory-expand-hint">Click to expand</div>}
                  </li>
                )) : memoryQuery ? <EmptyItem text="No matches — try different keywords" /> : <EmptyItem text="Search to explore Cortex memories" />}
              </ul>
            </div>
          </section>
        ) : null}

        {panel === "analytics" ? (
          <section className="panel active">
            <div className="panel-header">
              <h1>Analytics</h1>
              <div style={{ display: "flex", alignItems: "center", gap: 12, flexWrap: "wrap" }}>
                <span className="panel-subtitle">Token savings & brain health</span>
                <label className="analytics-inline-control">
                  <span>Currency</span>
                  <select value={currency} onChange={(event) => setCurrency(event.target.value)}>
                    {CURRENCY_OPTIONS.map((code) => (
                      <option key={code} value={code}>{code}</option>
                    ))}
                  </select>
                </label>
                <div className="analytics-view-toggle" role="tablist" aria-label="Analytics view mode">
                  <button
                    type="button"
                    className={`btn-sm ${analyticsMode === "aggregate" ? "btn-primary" : ""}`}
                    onClick={() => setAnalyticsMode("aggregate")}
                  >
                    Aggregate
                  </button>
                  <button
                    type="button"
                    className={`btn-sm ${analyticsMode === "operations" ? "btn-primary" : ""}`}
                    onClick={() => setAnalyticsMode("operations")}
                  >
                    By Operation
                  </button>
                </div>
                <button type="button" className="btn-sm" onClick={() => refreshSavings().catch(err => setFeedbackMessage(`Savings: ${err.message || err}`))}>Refresh</button>
              </div>
            </div>
            {savings ? (
              <>
                <div className="metrics" style={{ gridTemplateColumns: "repeat(5, minmax(0, 1fr))" }}>
                  <div className="metric" data-accent="cyan">
                    <span className="metric-value"><AnimatedNumber value={savings.summary?.totalSaved || 0} duration={1000} /></span>
                    <span className="metric-label">Boot Tokens Saved</span>
                    <span className="metric-icon">↓</span>
                  </div>
                  <div className="metric" data-accent="green">
                    <span className="metric-value"><AnimatedNumber value={savings.summary?.avgPercent || 0} />%</span>
                    <span className="metric-label">Avg Compression</span>
                    <span className="metric-icon">◎</span>
                  </div>
                  <div className="metric" data-accent="blue">
                    <span className="metric-value"><AnimatedNumber value={savings.summary?.totalBoots || 0} /></span>
                    <span className="metric-label">Boot Compilations</span>
                    <span className="metric-icon">⟳</span>
                  </div>
                  <div className="metric" data-accent="purple">
                    <span className="metric-value"><AnimatedNumber value={savings.summary?.totalServed || 0} duration={1000} /></span>
                    <span className="metric-label">Boot Prompt Tokens</span>
                    <span className="metric-icon">→</span>
                  </div>
                  <div className="metric" data-accent="green">
                    <span className="metric-value">{formatCurrency(((savings.summary?.totalSaved || 0) * SAVINGS_USD_PER_MILLION) / 1000000)}</span>
                    <span className="metric-label">Est. {currency} Saved</span>
                    <span className="metric-icon">$</span>
                  </div>
                </div>

                {analyticsMode === "aggregate" ? (
                  <>
                    <div className="analytics-explainer">
                      <p>Cortex compiles a token-budgeted boot prompt instead of loading raw context. `baseline` is the estimated raw context tokens; `served` is the compiled boot prompt tokens. Aggregate view emphasizes total compounding impact; use "By Operation" for recall/store/boot/tool splits.</p>
                    </div>

                    <div className="overview-grid">
                      <div className="card">
                        <div className="card-header">
                          <h2>Daily Token Savings</h2>
                        </div>
                        <Sparkline
                          data={(savings.daily || []).map(d => d.saved)}
                          width={500}
                          height={80}
                        />
                        <div className="chart-legend">
                          {(savings.daily || []).slice(-7).map(d => (
                            <span key={d.date} className="chart-day">
                              <span className="chart-day-label">{d.date.slice(5)}</span>
                              <span className="chart-day-value">{d.saved.toLocaleString()}</span>
                            </span>
                          ))}
                        </div>
                      </div>

                      <div className="card">
                        <div className="card-header">
                          <h2>Boots Per Day</h2>
                        </div>
                        <Sparkline
                          data={(savings.daily || []).map(d => d.boots)}
                          width={500}
                          height={80}
                          color="var(--agent-claude)"
                        />
                        <div className="chart-legend">
                          {(savings.daily || []).slice(-7).map(d => (
                            <span key={d.date} className="chart-day">
                              <span className="chart-day-label">{d.date.slice(5)}</span>
                              <span className="chart-day-value">{d.boots}</span>
                            </span>
                          ))}
                        </div>
                      </div>
                    </div>

                    {(cumulativeSeries.length > 1 || recallTrendSeries.length > 1) && (
                      <div className="overview-grid" style={{ marginTop: 14 }}>
                        <div className="card">
                          <div className="card-header">
                            <h2>Cumulative Savings</h2>
                          </div>
                          <Sparkline
                            data={cumulativeSeries.map((point) => Number(point.savedTotal || 0))}
                            width={500}
                            height={80}
                            color="var(--green)"
                          />
                          <div className="chart-legend">
                            {cumulativeSeries.slice(-7).map((point) => (
                              <span key={point.date || point.timestamp} className="chart-day">
                                <span className="chart-day-label">{(point.date || "").slice(5)}</span>
                                <span className="chart-day-value">{Number(point.savedTotal || 0).toLocaleString()}</span>
                              </span>
                            ))}
                          </div>
                        </div>

                        <div className="card">
                          <div className="card-header">
                            <h2>Recall Hit Rate</h2>
                          </div>
                          <Sparkline
                            data={recallTrendSeries.map((point) => Number(point.hitRatePct || 0))}
                            width={500}
                            height={80}
                            color="var(--cyan)"
                          />
                          <div className="chart-legend">
                            {recallTrendSeries.slice(-7).map((point) => (
                              <span key={point.date} className="chart-day">
                                <span className="chart-day-label">{(point.date || "").slice(5)}</span>
                                <span className="chart-day-value">{Math.round(Number(point.hitRatePct || 0))}%</span>
                              </span>
                            ))}
                          </div>
                        </div>
                      </div>
                    )}

                    {activityHeatmap.length > 0 && (
                      <div className="card" style={{ marginTop: 14 }}>
                        <div className="card-header">
                          <h2>Agent Activity Heatmap</h2>
                        </div>
                        <div className="activity-heatmap">
                          {["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].map((day) => (
                            <div key={day} className="activity-heatmap-row">
                              <span className="activity-heatmap-day">{day}</span>
                              <div className="activity-heatmap-cells">
                                {Array.from({ length: 24 }).map((_, hour) => {
                                  const count = activityHeatmapLookup.get(`${day}:${hour}`) || 0;
                                  const alpha = count > 0 ? Math.max(0.15, count / activityHeatmapMax) : 0.05;
                                  return (
                                    <span
                                      key={`${day}-${hour}`}
                                      className="activity-heatmap-cell"
                                      title={`${day} ${hour.toString().padStart(2, "0")}:00 · ${count} events`}
                                      style={{ background: `rgba(0, 212, 255, ${alpha})` }}
                                    />
                                  );
                                })}
                              </div>
                            </div>
                          ))}
                        </div>
                      </div>
                    )}

                    {savings.byAgent?.length > 0 && (
                      <div className="card" style={{ marginTop: 14 }}>
                        <div className="card-header">
                          <h2>Boot Savings by Agent</h2>
                        </div>
                        <ul className="item-list">
                          {savings.byAgent
                            .slice()
                            .sort((a, b) => Number(b.saved || 0) - Number(a.saved || 0))
                            .slice(0, 8)
                            .map((row, i) => (
                              <li key={`${row.agent}-${i}`}>
                                <div className="item-meta">
                                  <span className="item-name" style={{ color: agentColor(row.agent) }}>{row.agent}</span>
                                  <span className="memory-method">{Number(row.percent || 0)}% saved</span>
                                  <span className="muted-inline">{Number(row.boots || 0)} boots</span>
                                </div>
                                <div className="item-detail">
                                  {`${Number(row.saved || 0).toLocaleString()}t saved · ${Number(row.served || 0).toLocaleString()}t served`}
                                </div>
                              </li>
                            ))}
                        </ul>
                      </div>
                    )}

                    {savings.recent?.length > 0 && (
                      <div className="card" style={{ marginTop: 14 }}>
                        <div className="card-header">
                          <h2>Recent Boot Savings</h2>
                          <span className="badge">{savings.recent.length}</span>
                        </div>
                        <ul className="item-list">
                          {savings.recent.slice(-10).reverse().map((s, i) => (
                            <li key={`${s.timestamp}-${i}`}>
                              <div className="item-meta">
                                <span className="item-name" style={{ color: agentColor(s.agent) }}>{s.agent}</span>
                                <span className="memory-method">{s.percent}% saved</span>
                                <span className="muted-inline">{timeAgo(s.timestamp)}</span>
                              </div>
                              <div className="item-detail">
                                {`boot prompt ${Number(s.served || 0).toLocaleString()}t from est. raw ${Number(s.baseline || 0).toLocaleString()}t (${Number(s.saved || 0).toLocaleString()}t saved)`}
                                {(Number(s.admitted || 0) > 0 || Number(s.rejected || 0) > 0)
                                  ? ` · capsules ${Number(s.admitted || 0)} in / ${Number(s.rejected || 0)} out`
                                  : ""}
                              </div>
                            </li>
                          ))}
                        </ul>
                      </div>
                    )}
                  </>
                ) : (
                  <>
                    <div className="analytics-explainer">
                      <p>Operation view breaks savings into recall, store, boot compression, and tool-call categories using local events. Hover each bar for raw vs compressed token counts.</p>
                    </div>
                    <div className="card">
                      <div className="card-header">
                        <h2>Savings by Operation</h2>
                      </div>
                      <div className="operation-bars">
                        {operationRows.length ? operationRows.map((row) => {
                          const saved = Number(row.saved || 0);
                          const served = Number(row.served || 0);
                          const baseline = Number(row.baseline || 0);
                          const width = Math.max(4, Math.round((saved / operationMaxSaved) * 100));
                          const label = SAVINGS_OPERATION_LABELS[row.operation] || row.operation;
                          return (
                            <div className="operation-bar-row" key={row.operation}>
                              <div className="operation-bar-header">
                                <span className="item-name">{label}</span>
                                <span className="muted-inline">{saved.toLocaleString()} tokens · {formatCurrency((saved * SAVINGS_USD_PER_MILLION) / 1000000)}</span>
                              </div>
                              <div className="operation-bar-track" title={`Raw ${baseline.toLocaleString()} · Compressed ${served.toLocaleString()}`}>
                                <span className="operation-bar-fill" style={{ width: `${width}%` }} />
                              </div>
                              <div className="item-detail">{`${Number(row.events || 0)} events · raw ${baseline.toLocaleString()} · compressed ${served.toLocaleString()}`}</div>
                            </div>
                          );
                        }) : <EmptyItem text="No operation breakdown data yet" />}
                      </div>
                    </div>
                  </>
                )}
              </>
            ) : (
              <div className="card full">
                <EmptyItem text="Loading savings data..." />
              </div>
            )}
          </section>
        ) : null}

        {panel === "locks" ? (
          <section className="panel active">
            <div className="panel-header">
              <h1>File Locks</h1>
            </div>
            <div className="card full">
              <ul className="item-list">
                {locks.length ? locks.map((lock) => <LockItem key={lock.id || `${lock.path}:${lock.agent}`} lock={lock} />) : <EmptyItem text="No active locks" />}
              </ul>
            </div>
          </section>
        ) : null}

        {panel === "visualizer" ? (
          <section className="panel active brain-panel">
            <BrainErrorBoundary>
              <BrainVisualizer api={api} cortexBase={cortexBase} authToken={tokenRef.current} />
            </BrainErrorBoundary>
          </section>
        ) : null}

        {panel === "conflicts" ? (
          <section className="panel active">
            <div className="panel-header">
              <h1>Conflict Resolution</h1>
              <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
                <span className="badge">{conflictPairs.length} dispute{conflictPairs.length !== 1 ? "s" : ""}</span>
                <button type="button" className="btn-sm" onClick={() => refreshConflicts().catch(err => setFeedbackMessage(`Conflicts: ${err.message || err}`))}>Refresh</button>
              </div>
            </div>
            {conflictPairs.length === 0 ? (
              <div className="card full">
                <ul><EmptyItem text="No active conflicts -- all decisions are in harmony" /></ul>
              </div>
            ) : (
              conflictPairs.map((pair) => (
                <div key={`${pair.left.id}-${pair.right.id}`} className="conflict-pair">
                  <div className="conflict-cards">
                    <div className="card conflict-card">
                      <div className="conflict-card-header">
                        <span className="conflict-id">#{pair.left.id}</span>
                        <span className="agent-indicator" style={{
                          background: agentColor(pair.left.source_agent),
                          boxShadow: `0 0 8px ${agentColor(pair.left.source_agent)}`,
                        }} />
                        <span className="item-name">{pair.left.source_agent || "unknown"}</span>
                        <span className="muted-inline">{timeAgo(pair.left.created_at)}</span>
                      </div>
                      <p className="conflict-text">{pair.left.decision}</p>
                      {pair.left.context && <p className="conflict-context">{pair.left.context}</p>}
                      <div className="conflict-meta">
                        <span>Confidence: {((pair.left.confidence || 0.8) * 100).toFixed(0)}%</span>
                      </div>
                    </div>
                    <div className="conflict-vs">VS</div>
                    <div className="card conflict-card">
                      <div className="conflict-card-header">
                        <span className="conflict-id">#{pair.right.id}</span>
                        <span className="agent-indicator" style={{
                          background: agentColor(pair.right.source_agent),
                          boxShadow: `0 0 8px ${agentColor(pair.right.source_agent)}`,
                        }} />
                        <span className="item-name">{pair.right.source_agent || "unknown"}</span>
                        <span className="muted-inline">{timeAgo(pair.right.created_at)}</span>
                      </div>
                      <p className="conflict-text">{pair.right.decision}</p>
                      {pair.right.context && <p className="conflict-context">{pair.right.context}</p>}
                      <div className="conflict-meta">
                        <span>Confidence: {((pair.right.confidence || 0.8) * 100).toFixed(0)}%</span>
                      </div>
                    </div>
                  </div>
                  <div className="conflict-actions">
                    <button className="btn-sm btn-primary" disabled={conflictLoading}
                      onClick={() => handleResolveConflict(pair.left.id, "keep", pair.right.id)}>
                      Keep Left
                    </button>
                    <button className="btn-sm btn-primary" disabled={conflictLoading}
                      onClick={() => handleResolveConflict(pair.right.id, "keep", pair.left.id)}>
                      Keep Right
                    </button>
                    <button className="btn-sm" disabled={conflictLoading}
                      onClick={() => handleResolveConflict(pair.left.id, "merge", pair.right.id)}>
                      Merge Both
                    </button>
                    <button className="btn-sm btn-danger" disabled={conflictLoading}
                      onClick={() => handleResolveConflict(pair.left.id, "archive", pair.right.id)}>
                      Archive Both
                    </button>
                  </div>
                </div>
              ))
            )}
          </section>
        ) : null}

        {panel === "about" ? (
          <section className="panel active">
            <div className="panel-header">
              <h1>About</h1>
            </div>
            <div className="card full">
              <div style={{ padding: "2rem", maxWidth: 640 }}>
                <div style={{ display: "flex", alignItems: "center", gap: "1rem", marginBottom: "1.5rem" }}>
                  <img
                    src={`${import.meta.env.BASE_URL}icons/icon.png`}
                    alt="Cortex"
                    style={{ width: 64, height: 64, borderRadius: "50%", objectFit: "cover", flexShrink: 0 }}
                    onError={(e) => { e.currentTarget.style.display = "none"; e.currentTarget.nextSibling.style.display = "flex"; }}
                  />
                  <div style={{
                    width: 64, height: 64, borderRadius: "50%",
                    background: "linear-gradient(135deg, var(--cyan), var(--blue))",
                    display: "none", alignItems: "center", justifyContent: "center",
                    fontSize: "2rem", flexShrink: 0,
                  }}>◈</div>
                  <div>
                    <h2 style={{ margin: 0, fontSize: "1.5rem" }}>Cortex Control Center</h2>
                    <p style={{ margin: "0.25rem 0 0", color: "var(--text-3)" }}>Created by @AdityaVG13 -- Version 0.4.0</p>
                  </div>
                </div>

                <p style={{ color: "var(--text-2)", lineHeight: 1.7, marginBottom: "1.5rem" }}>
                  A persistent, self-improving brain for AI coding agents. Single Rust binary,
                  zero runtime dependencies, in-process ONNX embeddings.
                </p>

                <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "0.75rem", marginBottom: "2rem" }}>
                  {[
                    ["Daemon", "Rust + Axum"],
                    ["Embeddings", "ONNX (all-MiniLM-L6-v2)"],
                    ["Storage", "SQLite (WAL)"],
                    ["Transport", "HTTP + MCP stdio"],
                    ["Port", "7437"],
                    ["License", "MIT"],
                  ].map(([label, value]) => (
                    <div key={label} style={{
                      background: "var(--surface)", border: "1px solid var(--border)",
                      borderRadius: 8, padding: "0.75rem 1rem",
                    }}>
                      <span style={{ color: "var(--text-3)", fontSize: "0.75rem", textTransform: "uppercase", letterSpacing: "0.05em" }}>{label}</span>
                      <div style={{ marginTop: 4, fontWeight: 500 }}>{value}</div>
                    </div>
                  ))}
                </div>

                <div style={{ marginBottom: "2rem" }}>
                  <h3 style={{ fontSize: "0.875rem", textTransform: "uppercase", letterSpacing: "0.08em", color: "var(--text-3)", marginBottom: "0.75rem" }}>App Lifecycle (Start/Stop/Restart)</h3>
                  <table className="about-lifecycle-table">
                    <thead>
                      <tr>
                        <th>Action</th>
                        <th>What happens</th>
                      </tr>
                    </thead>
                    <tbody>
                      <tr>
                        <td>Start</td>
                        <td>Launches the daemon sidecar and waits for a healthy API before reloading data.</td>
                      </tr>
                      <tr>
                        <td>Stop</td>
                        <td>Sends a graceful shutdown request, then tears down sidecar process handles.</td>
                      </tr>
                      <tr>
                        <td>Restart</td>
                        <td>Runs Stop then Start with timeout handling; retries when shutdown hangs.</td>
                      </tr>
                      <tr>
                        <td>Close Window (✕)</td>
                        <td>Minimizes to tray by default so Cortex keeps running in background.</td>
                      </tr>
                      <tr>
                        <td>Exit</td>
                        <td>Fully quits the app process and attempts daemon shutdown.</td>
                      </tr>
                    </tbody>
                  </table>
                </div>

                <div style={{ marginBottom: "2rem" }}>
                  <h3 style={{ fontSize: "0.875rem", textTransform: "uppercase", letterSpacing: "0.08em", color: "var(--text-3)", marginBottom: "0.75rem" }}>Contributors</h3>
                  <div style={{ display: "flex", flexDirection: "column", gap: "0.5rem" }}>
                    {[
                      { handle: "AdityaVG13", role: "Creator & maintainer" },
                      { handle: "Claude Code", role: "Core architecture & retrieval pipeline" },
                      { handle: "Factory Droid", role: "Desktop app, reconnection & telemetry" },
                    ].map(({ handle, role }) => (
                      <div key={handle} style={{
                        display: "flex", alignItems: "center", gap: "0.75rem",
                        background: "var(--surface)", border: "1px solid var(--border)",
                        borderRadius: 8, padding: "0.625rem 1rem",
                      }}>
                        <span className="agent-indicator" style={{ background: "var(--cyan)", boxShadow: "0 0 8px var(--cyan)" }} />
                        <span style={{ fontWeight: 500 }}>@{handle}</span>
                        <span style={{ color: "var(--text-3)", fontSize: "0.875rem", marginLeft: "auto" }}>{role}</span>
                      </div>
                    ))}
                  </div>
                </div>

                <div style={{ display: "flex", gap: "0.75rem", flexWrap: "wrap" }}>
                  <a
                    href="https://github.com/AdityaVG13/cortex"
                    target="_blank"
                    rel="noreferrer"
                    className="btn-sm"
                    style={{ textDecoration: "none" }}
                  >
                    GitHub
                  </a>
                  <a
                    href="https://github.com/AdityaVG13/cortex/releases/tag/v0.3.0"
                    target="_blank"
                    rel="noreferrer"
                    className="btn-sm"
                    style={{ textDecoration: "none" }}
                  >
                    Release Notes
                  </a>
                </div>
              </div>
            </div>
          </section>
        ) : null}
      </main>
    </div>
  );
}
