import { useCallback, useEffect, useMemo, useRef, useState, Component } from "react";
import { BrainVisualizer } from "./BrainVisualizer.jsx";
import { checkForUpdates, installUpdate } from "./updater.js";

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
const FALLBACK_REFRESH_MS = 15000;
const ANALYTICS_REFRESH_MS = 60000;
const SSE_REFRESH_THROTTLE_MS = 300;
const SSE_RECONNECT_BASE_MS = 1000;
const SSE_RECONNECT_MAX_MS = 30000;

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

function readTauriInvoke() {
  return window.__TAURI__?.core?.invoke || null;
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
  if (n.includes("ollama") || n.includes("qwen") || n.includes("deepseek")) return "var(--agent-ollama)";
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

  const invokeRef = useRef(null);
  const tokenRef = useRef("");
  const refreshAllRef = useRef(async () => {});

  const knownAgents = useMemo(() => {
    const allAgents = new Set(sessions.map((session) => session.agent).filter(Boolean));
    if (messageAgent.trim()) allAgents.add(messageAgent.trim());
    return Array.from(allAgents).sort((a, b) => a.localeCompare(b));
  }, [sessions, messageAgent]);

  const api = useCallback(async (path, withAuth = false) => {
    const headers = {};
    if (withAuth) {
      if (!tokenRef.current) return null;
      headers.Authorization = `Bearer ${tokenRef.current}`;
    }

    try {
      const response = await fetch(`${cortexBase}${path}`, { headers });
      if (!response.ok) return null;
      return await response.json();
    } catch {
      return null;
    }
  }, [cortexBase]);

  const postApi = useCallback(async (path, body = {}) => {
    if (!tokenRef.current) return null;
    try {
      const response = await fetch(`${cortexBase}${path}`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${tokenRef.current}`,
        },
        body: JSON.stringify(body),
      });
      if (!response.ok) return null;
      return await response.json();
    } catch {
      return null;
    }
  }, [cortexBase]);

  const call = useCallback(async (command, args = {}) => {
    if (!invokeRef.current) throw new Error("No Tauri IPC available");
    return invokeRef.current(command, args);
  }, []);

  const readAuthToken = useCallback(async () => {
    if (!invokeRef.current) return;
    try {
      const token = await call("read_auth_token");
      tokenRef.current = token || "";
    } catch {
      tokenRef.current = "";
    }
  }, [call]);

  const refreshDaemonState = useCallback(async () => {
    if (invokeRef.current) {
      try {
        const state = await call("daemon_status");
        setDaemonState(state);
        setFeedbackMessage(state.message || "Daemon status updated.");
        return;
      } catch {
        // fallback to HTTP health
      }
    }

    const health = await api("/health");
    if (health?.status === "ok") {
      const nextState = {
        running: true,
        reachable: true,
        pid: null,
        message: `Connected — ${health.stats?.memories ?? 0} memories`,
      };
      setDaemonState(nextState);
      setFeedbackMessage(nextState.message);
    } else {
      const nextState = {
        running: false,
        reachable: false,
        pid: null,
        message: "Cannot reach daemon on :7437",
      };
      setDaemonState(nextState);
      setFeedbackMessage(nextState.message);
    }
  }, [api, call]);

  const refreshHealth = useCallback(async () => {
    const health = await api("/health");
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
    const [sessionsResult, locksResult, tasksResult] = await Promise.all([
      api("/sessions", true),
      api("/locks", true),
      api("/tasks?status=all", true),
    ]);

    setSessions(Array.isArray(sessionsResult?.sessions) ? sessionsResult.sessions : []);
    setLocks(Array.isArray(locksResult?.locks) ? locksResult.locks : []);
    setTasks(Array.isArray(tasksResult?.tasks) ? tasksResult.tasks : []);
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
    const result = await api("/savings");
    if (result) setSavings(result);
  }, [api]);

  const refreshConflicts = useCallback(async () => {
    const result = await api("/conflicts", true);
    setConflictPairs(Array.isArray(result?.pairs) ? result.pairs : []);
  }, [api]);

  const handleResolveConflict = useCallback(async (keepId, action, supersededId) => {
    setConflictLoading(true);
    await postApi("/resolve", { keepId, action, supersededId });
    await refreshConflicts();
    setConflictLoading(false);
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
    invokeRef.current = readTauriInvoke();
    await readAuthToken();
    await Promise.all([
      refreshDaemonState(),
      refreshHealth(),
      refreshCoreData(),
      refreshFeed(),
      refreshMessages(),
      refreshActivity(),
      refreshSavings(),
      refreshConflicts(),
    ]);
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
    refreshAllRef.current = refreshAll;
  }, [refreshAll]);

  useEffect(() => {
    refreshAllRef.current();
    const interval = setInterval(() => {
      refreshAllRef.current();
    }, FALLBACK_REFRESH_MS);
    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    checkForUpdates().then((update) => {
      if (update) setAvailableUpdate(update);
    });
  }, []);

  useEffect(() => {
    if (messageAgent.trim()) return;
    const defaultAgent = sessions.find((session) => session.agent)?.agent;
    if (defaultAgent) setMessageAgent(defaultAgent);
  }, [sessions, messageAgent]);

  useEffect(() => {
    refreshMessages();
  }, [refreshMessages]);

  useEffect(() => {
    refreshActivity();
  }, [refreshActivity]);

  useEffect(() => {
    if (panel !== "analytics") return;
    refreshSavings();
    const timer = setInterval(() => {
      refreshSavings();
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
          .catch(() => {})
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
    const recallResult = await api(`/recall?q=${encodeURIComponent(source)}&k=3`);
    const match = recallResult?.results?.find(r => r.source === source);
    if (match) {
      setMemoryResults(prev => prev.map(m =>
        m.source === source ? { ...m, excerpt: match.excerpt, expanded: true } : m
      ));
    }
  }

  async function handleStartDaemon() {
    if (!invokeRef.current) return;
    try {
      const result = await call("start_daemon");
      setFeedbackMessage(result.message || "Daemon start requested.");
    } catch (error) {
      setFeedbackMessage(`Start failed: ${String(error)}`);
    }
    await refreshAll();
  }

  async function handleStopDaemon() {
    if (!invokeRef.current) return;
    try {
      const result = await call("stop_daemon");
      setFeedbackMessage(result.message || "Daemon stop requested.");
    } catch (error) {
      setFeedbackMessage(`Stop failed: ${String(error)}`);
    }
    await refreshAll();
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
          <div className="daemon-controls-grid">
            <button type="button" className="btn-ctrl btn-primary" onClick={handleStartDaemon}>Start</button>
            <button type="button" className="btn-ctrl" onClick={handleStopDaemon}>Stop</button>
            <button type="button" className="btn-ctrl btn-danger" onClick={async () => {
              await handleStopDaemon();
              if (window.__TAURI__?.core?.invoke) {
                window.__TAURI__.core.invoke("stop_daemon");
                window.__TAURI__.process?.exit?.(0);
              }
            }}>Exit</button>
          </div>
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
            <span className="topbar-stat"><span className="topbar-label">AGENTS</span> {sessions.length}</span>
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
                <span className="sys-value sys-ok">{sessions.length} CONNECTED</span>
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
                  <span className="badge">{sessions.length}</span>
                </div>
                <ul className="item-list">
                  {sessions.length ? sessions.map((session) => <AgentItem key={session.sessionId || session.agent} session={session} />) : <EmptyItem text="No agents online" />}
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
                {sessions.length ? sessions.map((session) => <AgentItem key={session.sessionId || session.agent} session={session} />) : <EmptyItem text="No agents online" />}
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
                  <span className="badge">{completedTasks.length}</span>
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
                  <button type="button" className="btn-sm" onClick={refreshFeed}>
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
                  <button type="button" className="btn-sm" onClick={refreshMessages}>
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
                  <button type="button" className="btn-sm" onClick={refreshActivity}>
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
              <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
                <span className="panel-subtitle">Token savings & brain health</span>
                <button type="button" className="btn-sm" onClick={refreshSavings}>Refresh</button>
              </div>
            </div>
            {savings ? (
              <>
                <div className="metrics" style={{ gridTemplateColumns: "repeat(5, minmax(0, 1fr))" }}>
                  <div className="metric" data-accent="cyan">
                    <span className="metric-value"><AnimatedNumber value={savings.summary?.totalSaved || 0} duration={1000} /></span>
                    <span className="metric-label">Tokens Saved</span>
                    <span className="metric-icon">↓</span>
                  </div>
                  <div className="metric" data-accent="green">
                    <span className="metric-value"><AnimatedNumber value={savings.summary?.avgPercent || 0} />%</span>
                    <span className="metric-label">Avg Compression</span>
                    <span className="metric-icon">◎</span>
                  </div>
                  <div className="metric" data-accent="blue">
                    <span className="metric-value"><AnimatedNumber value={savings.summary?.totalBoots || 0} /></span>
                    <span className="metric-label">Total Boots</span>
                    <span className="metric-icon">⟳</span>
                  </div>
                  <div className="metric" data-accent="purple">
                    <span className="metric-value"><AnimatedNumber value={savings.summary?.totalServed || 0} duration={1000} /></span>
                    <span className="metric-label">Tokens Served</span>
                    <span className="metric-icon">→</span>
                  </div>
                  <div className="metric" data-accent="green">
                    <span className="metric-value">${((savings.summary?.totalSaved || 0) * 15 / 1000000).toFixed(2)}</span>
                    <span className="metric-label">Est. USD Saved</span>
                    <span className="metric-icon">$</span>
                  </div>
                </div>

                <div className="analytics-explainer">
                  <p>Cortex compiles a token-budgeted boot prompt instead of reading raw files. Each boot saves ~96% of tokens that would otherwise be spent on raw file reads. The savings compound across every AI session — Claude, Droid, Gemini, and any agent connecting to this brain.</p>
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
                            {s.baseline.toLocaleString()} baseline → {s.served.toLocaleString()} served ({s.saved.toLocaleString()} saved)
                          </div>
                        </li>
                      ))}
                    </ul>
                  </div>
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
              <BrainVisualizer />
            </BrainErrorBoundary>
          </section>
        ) : null}

        {panel === "conflicts" ? (
          <section className="panel active">
            <div className="panel-header">
              <h1>Conflict Resolution</h1>
              <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
                <span className="badge">{conflictPairs.length} dispute{conflictPairs.length !== 1 ? "s" : ""}</span>
                <button type="button" className="btn-sm" onClick={refreshConflicts}>Refresh</button>
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
                    src="/icons/icon.png"
                    alt="Creator"
                    style={{ width: 64, height: 64, borderRadius: "50%", objectFit: "cover", flexShrink: 0 }}
                    onError={(e) => { e.currentTarget.style.display = "none"; e.currentTarget.nextSibling.style.display = "flex"; }}
                  />
                  <div style={{
                    width: 64, height: 64, borderRadius: "50%",
                    background: "linear-gradient(135deg, var(--accent-cyan), var(--accent-blue))",
                    display: "none", alignItems: "center", justifyContent: "center",
                    fontSize: "2rem", flexShrink: 0,
                  }}>◈</div>
                  <div>
                    <h2 style={{ margin: 0, fontSize: "1.5rem" }}>Cortex Control Center</h2>
                    <p style={{ margin: "0.25rem 0 0", color: "var(--muted)" }}>Created by @AdityaVG13 &mdash; Version 0.3.0</p>
                  </div>
                </div>

                <p style={{ color: "var(--text-secondary)", lineHeight: 1.7, marginBottom: "1.5rem" }}>
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
                      background: "var(--card-bg)", border: "1px solid var(--border)",
                      borderRadius: 8, padding: "0.75rem 1rem",
                    }}>
                      <span style={{ color: "var(--muted)", fontSize: "0.75rem", textTransform: "uppercase", letterSpacing: "0.05em" }}>{label}</span>
                      <div style={{ marginTop: 4, fontWeight: 500 }}>{value}</div>
                    </div>
                  ))}
                </div>

                <div style={{ marginBottom: "2rem" }}>
                  <h3 style={{ fontSize: "0.875rem", textTransform: "uppercase", letterSpacing: "0.08em", color: "var(--muted)", marginBottom: "0.75rem" }}>Contributors</h3>
                  <div style={{ display: "flex", flexDirection: "column", gap: "0.5rem" }}>
                    {[
                      { handle: "AdityaVG13", role: "Creator & maintainer" },
                    ].map(({ handle, role }) => (
                      <div key={handle} style={{
                        display: "flex", alignItems: "center", gap: "0.75rem",
                        background: "var(--card-bg)", border: "1px solid var(--border)",
                        borderRadius: 8, padding: "0.625rem 1rem",
                      }}>
                        <span className="agent-indicator" style={{ background: "var(--accent-cyan)", boxShadow: "0 0 8px var(--accent-cyan)" }} />
                        <span style={{ fontWeight: 500 }}>@{handle}</span>
                        <span style={{ color: "var(--muted)", fontSize: "0.875rem", marginLeft: "auto" }}>{role}</span>
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
