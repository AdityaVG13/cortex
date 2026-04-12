import { startTransition, useCallback, useEffect, useId, useMemo, useRef, useState, Component } from "react";
import { BrainVisualizer } from "./BrainVisualizer.jsx";
import { checkForUpdates, installUpdate } from "./updater.js";
import {
  createApi,
  createPostApi,
  isAuthFailure,
  settledWithRethrow,
  settledCollectErrors,
  summarizeDashboardErrors,
} from "./api-client.js";
import { CURRENCY_OPTIONS, USD_TO_CURRENCY_RATE, SAVINGS_OPERATION_LABELS } from "./constants.js";
import {
  buildKnownAgents,
  canClaimTask,
  canFinalizeTask,
  canUnlockLock,
  filterFeedEntries,
  isTransportSession,
  nextFeedAckId,
  normalizeTask,
  resolveAgentName,
  sameAgent,
} from "./live-surface.js";
import { AppIcon } from "./ui-icons.jsx";

class BrainErrorBoundary extends Component {
  constructor(props) { super(props); this.state = { crashed: false, error: "" }; }
  static getDerivedStateFromError(err) { return { crashed: true, error: err?.message || "Unknown error" }; }
  render() {
    if (this.state.crashed) return (
      <div className="brain-loading">
        <div className="coming-icon"><AppIcon name="brain" size={48} /></div>
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
const SIDEBAR_COLLAPSE_BREAKPOINT_PX = 1100;

const FEED_KIND_LABEL = {
  prompt: "Prompt",
  completion: "Completion",
  task_complete: "Task Complete",
  system: "System",
};

const PANELS = [
  { key: "overview", label: "Overview", icon: "overview" },
  { key: "memory", label: "Memory", icon: "memory" },
  { key: "analytics", label: "Analytics", icon: "analytics" },
  { key: "agents", label: "Agents", icon: "agents" },
  { key: "tasks", label: "Tasks", icon: "tasks" },
  { key: "feed", label: "Feed", icon: "feed" },
  { key: "messages", label: "Messages", icon: "messages" },
  { key: "activity", label: "Activity", icon: "activity" },
  { key: "locks", label: "Locks", icon: "locks" },
  { key: "visualizer", label: "Brain", icon: "brain" },
  { key: "conflicts", label: "Conflicts", icon: "conflicts" },
  { key: "about", label: "About", icon: "about" },
];

const PANEL_SEQUENCE = [
  { key: "overview", label: "Overview", icon: "overview" },
  { key: "analytics", label: "Analytics", icon: "analytics" },
  { key: "agents", label: "Agents", icon: "agents" },
  { key: "work", label: "Work", icon: "work" },
  { key: "memory", label: "Memory", icon: "memory" },
  { key: "brain", label: "Brain", icon: "brain" },
  { key: "about", label: "About", icon: "about" },
];

function panelIndex(panelKey) {
  return PANEL_SEQUENCE.findIndex((entry) => entry.key === panelKey);
}

const EMPTY_DAEMON = {
  running: false,
  reachable: false,
  managed: false,
  authTokenReady: false,
  pid: null,
  message: "Checking daemon...",
};

const EMPTY_HEALTH_META = {
  status: "unknown",
  degraded: false,
  dbCorrupted: false,
  runtimeVersion: "",
};

const CONTROL_CENTER_VERSION = "0.5.0";
const RECALL_HEADLINE_MIN_QUERIES = 20;
const CORTEX_BASE_STORAGE_KEY = "cortex_base";
const CORTEX_AUTH_STORAGE_KEY = "cortex_auth_token";
const LEGACY_CORTEX_AUTH_STORAGE_KEYS = ["cortex_token"];
const CORTEX_OPERATOR_STORAGE_KEY = "cortex_operator";
const CORTEX_PANEL_STORAGE_KEY = "cortex_panel";
function clearLegacyBrowserAuthTokens() {
  if (typeof window === "undefined") return;
  try {
    for (const key of LEGACY_CORTEX_AUTH_STORAGE_KEYS) {
      window.sessionStorage.removeItem(key);
      window.localStorage.removeItem(key);
    }
  } catch {
    // Ignore storage failures in restricted browser contexts.
  }
}

function readPersistedBrowserAuthToken() {
  if (typeof window === "undefined") return "";
  try {
    const sessionToken = window.sessionStorage.getItem(CORTEX_AUTH_STORAGE_KEY) || "";
    if (sessionToken) return sessionToken;

    for (const key of LEGACY_CORTEX_AUTH_STORAGE_KEYS) {
      const legacySessionToken = window.sessionStorage.getItem(key) || "";
      if (legacySessionToken) {
        window.sessionStorage.setItem(CORTEX_AUTH_STORAGE_KEY, legacySessionToken);
        clearLegacyBrowserAuthTokens();
        return legacySessionToken;
      }
    }

    const legacyToken = window.localStorage.getItem(CORTEX_AUTH_STORAGE_KEY) || "";
    if (legacyToken) {
      window.sessionStorage.setItem(CORTEX_AUTH_STORAGE_KEY, legacyToken);
      window.localStorage.removeItem(CORTEX_AUTH_STORAGE_KEY);
      clearLegacyBrowserAuthTokens();
      return legacyToken;
    }

    for (const key of LEGACY_CORTEX_AUTH_STORAGE_KEYS) {
      const legacyLocalToken = window.localStorage.getItem(key) || "";
      if (legacyLocalToken) {
        window.sessionStorage.setItem(CORTEX_AUTH_STORAGE_KEY, legacyLocalToken);
        clearLegacyBrowserAuthTokens();
        return legacyLocalToken;
      }
    }
  } catch {
    return "";
  }
  return "";
}

function readBrowserBootstrap() {
  if (typeof window === "undefined") {
    return { cortexBase: "", authToken: "", panel: "overview" };
  }

  const params = new URLSearchParams(window.location.search);
  let storedPanel = "";
  let storedBase = DEFAULT_CORTEX_BASE;
  try {
    storedPanel = window.localStorage.getItem(CORTEX_PANEL_STORAGE_KEY) || "";
    storedBase = window.localStorage.getItem(CORTEX_BASE_STORAGE_KEY) || DEFAULT_CORTEX_BASE;
  } catch {
    // Ignore storage failures in restricted browser contexts.
  }

  const requestedPanel = params.get("panel") || storedPanel || "";
  const panel = PANEL_SEQUENCE.some((entry) => entry.key === requestedPanel) ? requestedPanel : "overview";
  const cortexBase = params.get("cortexBase") || storedBase || DEFAULT_CORTEX_BASE;
  const authTokenFromParams = params.get("authToken") || "";
  const authToken = authTokenFromParams || readPersistedBrowserAuthToken();

  try {
    if (params.get("panel")) {
      window.localStorage.setItem(CORTEX_PANEL_STORAGE_KEY, panel);
    }
    if (params.get("cortexBase")) {
      window.localStorage.setItem(CORTEX_BASE_STORAGE_KEY, cortexBase);
    }
  } catch {
    // Ignore storage failures in restricted browser contexts.
  }
  if (authTokenFromParams) {
    try {
      window.sessionStorage.setItem(CORTEX_AUTH_STORAGE_KEY, authToken);
      window.localStorage.removeItem(CORTEX_AUTH_STORAGE_KEY);
    } catch {
      // Ignore storage failures in restricted browser contexts.
    }
    params.delete("authToken");
    const nextQuery = params.toString();
    const nextUrl = `${window.location.pathname}${nextQuery ? `?${nextQuery}` : ""}${window.location.hash}`;
    window.history.replaceState({}, "", nextUrl);
  }

  return { cortexBase, authToken, panel };
}

function persistBrowserAuthToken(token) {
  if (typeof window === "undefined") return;
  try {
    if (token) {
      window.sessionStorage.setItem(CORTEX_AUTH_STORAGE_KEY, token);
      window.localStorage.removeItem(CORTEX_AUTH_STORAGE_KEY);
      clearLegacyBrowserAuthTokens();
    } else {
      window.sessionStorage.removeItem(CORTEX_AUTH_STORAGE_KEY);
      window.localStorage.removeItem(CORTEX_AUTH_STORAGE_KEY);
      clearLegacyBrowserAuthTokens();
    }
  } catch {
    // Ignore storage failures in restricted browser contexts.
  }
}

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
  if (typeof window === "undefined" || !window.__TAURI_INTERNALS__) {
    return null;
  }
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke;
  } catch {
    return null;
  }
}

function formatDaemonEndpoint(cortexBase) {
  try {
    const url = new URL(cortexBase);
    const port = url.port || (url.protocol === "https:" ? "443" : "80");
    return `${url.hostname}:${port}`;
  } catch {
    return "127.0.0.1:7437";
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

function clampNumber(value, min, max) {
  return Math.min(Math.max(value, min), max);
}

function formatCompactNumber(value) {
  if (!Number.isFinite(value)) return "0";
  if (Math.abs(value) >= 1000000) return `${(value / 1000000).toFixed(1)}M`;
  if (Math.abs(value) >= 1000) return `${(value / 1000).toFixed(1)}K`;
  return Math.round(value).toString();
}

function formatSignedCompactNumber(value) {
  const numeric = Number(value || 0);
  if (!Number.isFinite(numeric)) return "0";
  const prefix = numeric > 0 ? "+" : numeric < 0 ? "-" : "";
  return `${prefix}${formatCompactNumber(Math.abs(numeric))}`;
}

function buildLineGeometry(data, width, height, padding = 8) {
  if (!data || data.length < 2) return null;
  const numeric = data.map((value) => Number(value || 0));
  const max = Math.max(...numeric, 1);
  const min = Math.min(...numeric, 0);
  const range = max - min || 1;
  const innerWidth = width - padding * 2;
  const innerHeight = height - padding * 2;
  const points = numeric.map((value, index) => {
    const x = padding + (index / (numeric.length - 1)) * innerWidth;
    const y = padding + innerHeight - ((value - min) / range) * innerHeight;
    return { x, y, value };
  });
  const line = points.map((point, index) => `${index === 0 ? "M" : "L"} ${point.x} ${point.y}`).join(" ");
  const area = `${line} L ${points[points.length - 1].x} ${height - padding} L ${points[0].x} ${height - padding} Z`;
  return { points, line, area, min, max, padding };
}

function createSeededRng(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = Math.imul(state ^ (state >>> 15), 1 | state);
    t ^= t + Math.imul(t ^ (t >>> 7), 61 | t);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function gaussianRandom(rng) {
  let u = 0;
  let v = 0;
  while (u === 0) u = rng();
  while (v === 0) v = rng();
  return Math.sqrt(-2.0 * Math.log(u)) * Math.cos(2.0 * Math.PI * v);
}

function percentileFromSorted(sorted, percentile) {
  if (!sorted.length) return 0;
  const index = (sorted.length - 1) * percentile;
  const lower = Math.floor(index);
  const upper = Math.ceil(index);
  if (lower === upper) return sorted[lower];
  const weight = index - lower;
  return sorted[lower] * (1 - weight) + sorted[upper] * weight;
}

function buildMonteCarloProjection(dailySeries, cumulativeSeries, horizonDays = 30, simulationCount = 180) {
  const basis = dailySeries
    .map((point) => Number(point?.saved || 0))
    .filter((value) => Number.isFinite(value) && value > 0);
  if (basis.length < 4) return null;

  const recent = basis.slice(-14);
  const logReturns = [];
  for (let index = 1; index < recent.length; index += 1) {
    const previous = Math.max(recent[index - 1], 1);
    const current = Math.max(recent[index], 1);
    logReturns.push(Math.log(current / previous));
  }

  const drift = logReturns.length
    ? logReturns.reduce((sum, value) => sum + value, 0) / logReturns.length
    : 0.02;
  const variance = logReturns.length
    ? logReturns.reduce((sum, value) => sum + (value - drift) ** 2, 0) / logReturns.length
    : 0.04;
  const volatility = Math.max(Math.sqrt(variance), 0.08);
  const lastDaily = Math.max(recent[recent.length - 1], 1);
  const startTotal = Number(cumulativeSeries.at(-1)?.savedTotal || basis.reduce((sum, value) => sum + value, 0));
  const rng = createSeededRng(Math.round(startTotal + lastDaily + recent.length * 13));

  const runs = Array.from({ length: simulationCount }, (_, simIndex) => {
    let dailyValue = lastDaily;
    let cumulativeValue = startTotal;
    const series = [];
    for (let day = 0; day < horizonDays; day += 1) {
      const shock = gaussianRandom(rng) * volatility;
      const meanReversion = ((recent.reduce((sum, value) => sum + value, 0) / recent.length) - dailyValue) / Math.max(dailyValue, 1) * 0.04;
      const growth = Math.exp(drift + meanReversion + shock);
      dailyValue = Math.max(0, dailyValue * growth);
      cumulativeValue += dailyValue;
      series.push({
        day: day + 1,
        daily: dailyValue,
        cumulative: cumulativeValue,
        gain: cumulativeValue - startTotal,
      });
    }
    return {
      key: `sim-${simIndex}`,
      series,
      final: cumulativeValue - startTotal,
    };
  });

  const bandSeries = Array.from({ length: horizonDays }, (_, dayIndex) => {
    const values = runs
      .map((run) => run.series[dayIndex]?.gain || 0)
      .sort((left, right) => left - right);
    return {
      day: dayIndex + 1,
      p10: percentileFromSorted(values, 0.1),
      p25: percentileFromSorted(values, 0.25),
      p50: percentileFromSorted(values, 0.5),
      p75: percentileFromSorted(values, 0.75),
      p90: percentileFromSorted(values, 0.9),
    };
  });

  const samples = runs
    .filter((_, index) => index % Math.ceil(simulationCount / 14) === 0)
    .slice(0, 14)
    .map((run) => run.series.map((point) => point.gain));

  const endingValues = runs.map((run) => run.final).sort((left, right) => left - right);
  const summary = {
    startTotal,
    p10Gain: percentileFromSorted(endingValues, 0.1),
    p50Gain: percentileFromSorted(endingValues, 0.5),
    p90Gain: percentileFromSorted(endingValues, 0.9),
    avgDaily: recent.reduce((sum, value) => sum + value, 0) / recent.length,
  };

  summary.p10Total = startTotal + summary.p10Gain;
  summary.p50Total = startTotal + summary.p50Gain;
  summary.p90Total = startTotal + summary.p90Gain;

  return { bandSeries, samples, summary, horizonDays, simulationCount };
}

function Sparkline({
  data,
  width = 280,
  height = 60,
  color = "var(--cyan)",
  showArea = true,
  showEndDot = true,
  className = "",
}) {
  const [id] = useState(() => `spark-fill-${++sparklineCounter}`);
  const geometry = buildLineGeometry(data, width, height, 8);
  if (!geometry) return <div className="sparkline-empty">No data yet</div>;
  const lastPoint = geometry.points.at(-1);
  const gridLines = Array.from({ length: 4 }, (_, index) => {
    const y = 8 + (index * (height - 16)) / 3;
    return <line key={`grid-${index}`} x1="8" x2={width - 8} y1={y} y2={y} className="sparkline-grid-line" />;
  });

  return (
    <svg
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="xMidYMid meet"
      className={`sparkline ${className}`}
    >
      <defs>
        <linearGradient id={id} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity="0.22" />
          <stop offset="70%" stopColor={color} stopOpacity="0.08" />
          <stop offset="100%" stopColor={color} stopOpacity="0" />
        </linearGradient>
      </defs>
      <g className="sparkline-grid">{gridLines}</g>
      {showArea ? <path d={geometry.area} fill={`url(#${id})`} className="sparkline-area" /> : null}
      <path d={geometry.line} fill="none" stroke={color} strokeWidth="2.25" strokeLinejoin="round" strokeLinecap="round" className="sparkline-line" />
      {showEndDot && lastPoint ? (
        <>
          <circle cx={lastPoint.x} cy={lastPoint.y} r="6" fill={color} fillOpacity="0.18" />
          <circle cx={lastPoint.x} cy={lastPoint.y} r="2.75" fill={color} className="sparkline-end-dot" />
        </>
      ) : null}
    </svg>
  );
}

function MonteCarloProjectionChart({ projection, width = 820, height = 280 }) {
  if (!projection?.bandSeries?.length) return <div className="sparkline-empty">Not enough data for a projection yet</div>;

  const bandValues = projection.bandSeries.flatMap((point) => [point.p10, point.p25, point.p50, point.p75, point.p90]);
  const minValue = 0;
  const maxValue = Math.max(...bandValues, 1);
  const maxWithHeadroom = maxValue * 1.14;
  const padding = { top: 16, right: 18, bottom: 30, left: 18 };
  const innerWidth = width - padding.left - padding.right;
  const innerHeight = height - padding.top - padding.bottom;
  const valueRange = maxWithHeadroom - minValue || 1;
  const toX = (index) => padding.left + (index / (projection.bandSeries.length - 1)) * innerWidth;
  const toY = (value) => padding.top + innerHeight - ((value - minValue) / valueRange) * innerHeight;
  const areaPath = (upperKey, lowerKey) => {
    const top = projection.bandSeries.map((point, index) => `${index === 0 ? "M" : "L"} ${toX(index)} ${toY(point[upperKey])}`).join(" ");
    const bottom = [...projection.bandSeries]
      .reverse()
      .map((point, reverseIndex) => {
        const index = projection.bandSeries.length - 1 - reverseIndex;
        return `L ${toX(index)} ${toY(point[lowerKey])}`;
      })
      .join(" ");
    return `${top} ${bottom} Z`;
  };
  const linePath = (key) => projection.bandSeries.map((point, index) => `${index === 0 ? "M" : "L"} ${toX(index)} ${toY(point[key])}`).join(" ");
  const samplePaths = projection.samples.map((sample) => sample.map((value, index) => `${index === 0 ? "M" : "L"} ${toX(index)} ${toY(value)}`).join(" "));
  const endPoint = projection.bandSeries.at(-1);
  const summaryX = width - padding.right - 138;
  const summaryY = padding.top + 10;

  return (
    <svg
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="xMidYMid meet"
      className="projection-chart"
      role="img"
      aria-label="30-day Monte Carlo projection for cumulative savings gains"
    >
      <defs>
        <linearGradient id="projectionBandWide" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#4f7cff" stopOpacity="0.24" />
          <stop offset="100%" stopColor="#4f7cff" stopOpacity="0.02" />
        </linearGradient>
        <linearGradient id="projectionBandCore" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#4af2a1" stopOpacity="0.28" />
          <stop offset="100%" stopColor="#4af2a1" stopOpacity="0.04" />
        </linearGradient>
      </defs>
      <g className="projection-grid">
        {Array.from({ length: 4 }, (_, index) => {
          const y = padding.top + (index * innerHeight) / 3;
          return <line key={`y-${index}`} x1={padding.left} x2={width - padding.right} y1={y} y2={y} className="projection-grid-line" />;
        })}
        {Array.from({ length: 6 }, (_, index) => {
          const x = padding.left + (index * innerWidth) / 5;
          return <line key={`x-${index}`} y1={padding.top} y2={height - padding.bottom} x1={x} x2={x} className="projection-grid-line projection-grid-line-vertical" />;
        })}
        <line x1={padding.left} x2={width - padding.right} y1={toY(0)} y2={toY(0)} className="projection-baseline" />
      </g>
      <path d={areaPath("p90", "p10")} className="projection-band projection-band-wide" />
      <path d={areaPath("p75", "p25")} className="projection-band projection-band-core" />
      {samplePaths.map((path, index) => (
        <path key={`sample-${index}`} d={path} className="projection-sample" style={{ animationDelay: `${index * 70}ms` }} />
      ))}
      <path d={linePath("p50")} className="projection-line" />
      {endPoint ? (
        <>
          <circle cx={toX(projection.bandSeries.length - 1)} cy={toY(endPoint.p50)} r="9" className="projection-end-halo" />
          <circle cx={toX(projection.bandSeries.length - 1)} cy={toY(endPoint.p50)} r="3.5" className="projection-end-dot" />
          <g className="projection-summary" transform={`translate(${summaryX} ${summaryY})`}>
            <rect width="120" height="58" rx="10" className="projection-summary-panel" />
            <text x="12" y="18" className="projection-annotation projection-annotation-high">
              p90 {formatSignedCompactNumber(endPoint.p90)}
            </text>
            <text x="12" y="34" className="projection-annotation">
              p50 {formatSignedCompactNumber(endPoint.p50)}
            </text>
            <text x="12" y="50" className="projection-annotation projection-annotation-low">
              p10 {formatSignedCompactNumber(endPoint.p10)}
            </text>
          </g>
        </>
      ) : null}
      <text x={padding.left} y={height - 8} className="projection-axis-label">today</text>
      <text x={width - padding.right - 62} y={height - 8} className="projection-axis-label">+30d gain</text>
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
        <div className="coming-icon"><AppIcon name="brain" size={64} /></div>
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

function OperatorSelector({ value, knownAgents, onChange, label = "Operator", placeholder = "codex" }) {
  const datalistId = useId();
  return (
    <label className="feed-control">
      <span>{label}</span>
      <input
        type="text"
        list={datalistId}
        placeholder={placeholder}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
      <datalist id={datalistId}>
        {knownAgents.map((agent) => (
          <option key={agent} value={agent} />
        ))}
      </datalist>
    </label>
  );
}

function TaskItem({
  task,
  selectedOperator = "",
  completionDraft = "",
  completionExpanded = false,
  onClaim = null,
  onAbandon = null,
  onComplete = null,
  onDelete = null,
  onCompletionDraftChange = null,
  onToggleComplete = null,
  busyActionKey = "",
}) {
  const operator = String(selectedOperator || "").trim();
  const claimBusy = busyActionKey === `claim:${task.taskId}`;
  const abandonBusy = busyActionKey === `abandon:${task.taskId}`;
  const completeBusy = busyActionKey === `complete:${task.taskId}`;
  const deleteBusy = busyActionKey === `delete:${task.taskId}`;
  const operatorOwnsTask = canFinalizeTask(task, operator);
  const files = Array.isArray(task.files) ? task.files.slice(0, 4) : [];
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
      {task.description ? <div className="item-detail">{task.description}</div> : null}
      {files.length ? (
        <div className="feed-files">
          {files.map((file) => (
            <span key={`${task.taskId}-${file}`} className="lock-path">
              {file}
            </span>
          ))}
        </div>
      ) : null}
      <div className="task-actions">
        {canClaimTask(task, operator) && onClaim ? (
          <button
            type="button"
            className="btn-sm btn-primary"
            aria-label={`Claim task ${task.title}`}
            disabled={claimBusy}
            onClick={() => onClaim(task)}
          >
            {claimBusy ? "Claiming..." : "Claim"}
          </button>
        ) : null}
        {task.status === "claimed" && operatorOwnsTask && onToggleComplete ? (
          <button
            type="button"
            className="btn-sm"
            aria-label={`${completionExpanded ? "Cancel completion for" : "Complete task"} ${task.title}`}
            disabled={completeBusy}
            onClick={() => onToggleComplete(task.taskId)}
          >
            {completionExpanded ? "Cancel Complete" : "Complete"}
          </button>
        ) : null}
        {task.status === "claimed" && operatorOwnsTask && onAbandon ? (
          <button
            type="button"
            className="btn-sm btn-danger"
            aria-label={`Abandon task ${task.title}`}
            disabled={abandonBusy}
            onClick={() => onAbandon(task)}
          >
            {abandonBusy ? "Abandoning..." : "Abandon"}
          </button>
        ) : null}
        {task.status === "claimed" && !operatorOwnsTask && task.claimedBy ? (
          <span className="surface-inline-hint">Held by {task.claimedBy}</span>
        ) : null}
        {task.status === "completed" && onDelete ? (
          <button
            type="button"
            className="btn-sm"
            aria-label={`Delete task ${task.title}`}
            disabled={deleteBusy}
            onClick={() => onDelete(task)}
          >
            {deleteBusy ? "Deleting..." : "Delete"}
          </button>
        ) : null}
      </div>
      {completionExpanded && operatorOwnsTask && onComplete && onCompletionDraftChange ? (
        <div className="task-complete-panel">
          <textarea
            value={completionDraft}
            onChange={(event) => onCompletionDraftChange(task.taskId, event.target.value)}
            placeholder="Optional completion summary for the task feed"
            rows={3}
          />
          <div className="surface-actions">
            <button
              type="button"
              className="btn-sm"
              aria-label={`Keep task ${task.title} open`}
              onClick={() => onToggleComplete?.(task.taskId)}
            >
              Keep Open
            </button>
            <button
              type="button"
              className="btn-sm btn-primary"
              aria-label={`Confirm complete task ${task.title}`}
              disabled={completeBusy}
              onClick={() => onComplete(task, completionDraft)}
            >
              {completeBusy ? "Completing..." : "Confirm Complete"}
            </button>
          </div>
        </div>
      ) : null}
    </li>
  );
}

function LockItem({ lock, selectedOperator = "", onUnlock = null, busyActionKey = "" }) {
  const expiryMinutes = Math.max(
    0,
    Math.ceil((new Date(lock.expiresAt).getTime() - Date.now()) / 60000)
  );
  const unlockBusy = busyActionKey === `unlock:${lock.path}`;
  const unlockable = canUnlockLock(lock, selectedOperator);

  return (
    <li>
      <div className="lock-path">{lock.path}</div>
      <div className="item-meta">
        <span className="lock-agent">{lock.agent}</span>
        <span className="lock-expiry">{expiryMinutes}m remaining</span>
      </div>
      {unlockable && onUnlock ? (
        <div className="task-actions">
          <button
            type="button"
            className="btn-sm"
            disabled={unlockBusy}
            onClick={() => onUnlock(lock)}
          >
            {unlockBusy ? "Unlocking..." : "Unlock"}
          </button>
        </div>
      ) : null}
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
        <span className="msg-arrow"><AppIcon name="outbound" /></span>
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

function isReachableHealthPayload(health) {
  const status = String(health?.status || "").toLowerCase();
  return (status === "ok" || status === "degraded") && Boolean(health?.runtime) && Boolean(health?.stats);
}

export function App() {
  const browserBootstrap = useMemo(() => readBrowserBootstrap(), []);
  const [panel, setPanel] = useState(() => browserBootstrap.panel || "overview");
  const [panelMotionDirection, setPanelMotionDirection] = useState("forward");
  const [daemonState, setDaemonState] = useState(EMPTY_DAEMON);
  const [healthMeta, setHealthMeta] = useState(EMPTY_HEALTH_META);
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
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
    if (typeof window === "undefined") return false;
    return window.innerWidth <= SIDEBAR_COLLAPSE_BREAKPOINT_PX;
  });
  const [isNarrowViewport, setIsNarrowViewport] = useState(() => {
    if (typeof window === "undefined") return false;
    return window.innerWidth <= SIDEBAR_COLLAPSE_BREAKPOINT_PX;
  });
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
  const [selectedOperator, setSelectedOperator] = useState(() => {
    if (typeof window === "undefined") return "";
    try {
      return window.localStorage.getItem(CORTEX_OPERATOR_STORAGE_KEY) || "";
    } catch {
      return "";
    }
  });
  const [messageTarget, setMessageTarget] = useState("");
  const [messageDraft, setMessageDraft] = useState("");
  const [taskCompletionDrafts, setTaskCompletionDrafts] = useState({});
  const [completionTaskId, setCompletionTaskId] = useState("");
  const [busyActionKey, setBusyActionKey] = useState("");
  const [activitySince, setActivitySince] = useState("1h");
  const [feedbackMessage, setFeedbackMessage] = useState("Checking daemon...");
  const [conflictPairs, setConflictPairs] = useState([]);
  const [conflictLoading, setConflictLoading] = useState(false);
  const [editorSetup, setEditorSetup] = useState(null);
  const [editorDetections, setEditorDetections] = useState([]);
  const [selectedEditorIds, setSelectedEditorIds] = useState([]);
  const [cortexBase, setCortexBase] = useState(() => browserBootstrap.cortexBase || DEFAULT_CORTEX_BASE);
  const [showConnectionDialog, setShowConnectionDialog] = useState(false);
  const [showEditorSetupWizard, setShowEditorSetupWizard] = useState(false);
  const [availableUpdate, setAvailableUpdate] = useState(null);
  const [updateInstalling, setUpdateInstalling] = useState(false);
  const [restartingDaemon, setRestartingDaemon] = useState(false);
  const [restartError, setRestartError] = useState("");
  const [hasVisitedBrain, setHasVisitedBrain] = useState(() => browserBootstrap.panel === "brain");
  const [hasVisitedAnalytics, setHasVisitedAnalytics] = useState(() => browserBootstrap.panel === "analytics");
  const [analyticsReady, setAnalyticsReady] = useState(() => browserBootstrap.panel === "analytics");
  const [isSettingUpEditors, setIsSettingUpEditors] = useState(false);
  const [currency, setCurrency] = useState(() => localStorage.getItem("cortex_currency") || "USD");
  const [analyticsMode, setAnalyticsMode] = useState(() => localStorage.getItem("cortex_analytics_mode") || "aggregate");

  const invokeRef = useRef(null);
  const tokenRef = useRef(browserBootstrap.authToken || "");
  const refreshAllRef = useRef(async () => {});
  const daemonTransitionRef = useRef(false);
  const recoveryRetryTimerRef = useRef(null);
  const skipInitialMessagesRefreshRef = useRef(true);
  const skipInitialActivityRefreshRef = useRef(true);

  const changePanel = useCallback((nextPanel) => {
    if (!PANEL_SEQUENCE.some((entry) => entry.key === nextPanel) || nextPanel === panel) {
      return;
    }

    const currentIndex = panelIndex(panel);
    const nextIndex = panelIndex(nextPanel);
    setPanelMotionDirection(
      currentIndex >= 0 && nextIndex >= 0 && nextIndex < currentIndex ? "backward" : "forward"
    );
    startTransition(() => setPanel(nextPanel));
  }, [panel]);

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

    return Array.from(deduped.values()).filter((session) => !isTransportSession(session));
  }, [sessions]);

  const knownAgents = useMemo(() => {
    const extras = [
      selectedOperator.trim(),
      messageTarget.trim(),
      ...tasks.map((task) => task?.claimedBy),
      ...locks.map((lock) => lock?.agent),
      ...feedEntries.map((entry) => entry?.agent),
      ...messageEntries.flatMap((entry) => [entry?.from, entry?.to]),
    ].filter(Boolean);
    return buildKnownAgents(normalizedSessions, extras);
  }, [feedEntries, locks, messageEntries, messageTarget, normalizedSessions, selectedOperator, tasks]);

  const editorSetupSummary = useMemo(() => {
    const results = Array.isArray(editorSetup) ? editorSetup : [];
    return {
      results,
      detected: results.filter((entry) => entry.detected).length,
      registered: results.filter((entry) => entry.registered).length,
      failed: results.filter((entry) => entry.detected && !entry.registered).length,
    };
  }, [editorSetup]);

  const editorDetectionSummary = useMemo(() => {
    const results = Array.isArray(editorDetections) ? editorDetections : [];
    return {
      results,
      detected: results.filter((entry) => entry.detected).length,
      registered: results.filter((entry) => entry.registered).length,
    };
  }, [editorDetections]);

  const setupCommandPath = useMemo(() => {
    const current = editorDetectionSummary.results.find((entry) => entry.commandPath)?.commandPath;
    const previous = editorSetupSummary.results.find((entry) => entry.commandPath)?.commandPath;
    return current || previous || "C:\\Users\\<you>\\.cortex\\bin\\cortex.exe";
  }, [editorDetectionSummary.results, editorSetupSummary.results]);

  const manualMcpSnippet = useMemo(
    () =>
      JSON.stringify(
        {
          mcpServers: {
            cortex: {
              command: setupCommandPath,
              args: ["mcp"],
            },
          },
        },
        null,
        2,
      ),
    [setupCommandPath],
  );

  const selectedOperatorName = useMemo(
    () => resolveAgentName(selectedOperator, knownAgents),
    [knownAgents, selectedOperator],
  );
  const messageTargetName = useMemo(
    () => resolveAgentName(messageTarget, knownAgents),
    [knownAgents, messageTarget],
  );

  const currencyRate = USD_TO_CURRENCY_RATE[currency] ?? USD_TO_CURRENCY_RATE.USD;
  const memoryLoad = useMemo(
    () =>
      (typeof stats.memories === "number" ? stats.memories : 0)
      + (typeof stats.decisions === "number" ? stats.decisions : 0),
    [stats]
  );

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

  const clearTransientFeedback = useCallback((fallback = "Connected to daemon.") => {
    setFeedbackMessage((current) => {
      const text = String(current || "");
      if (
        text === "Checking daemon..." ||
        text.includes("could not authenticate") ||
        text.startsWith("Auth token read failed:") ||
        text.startsWith("Waiting for daemon auth token") ||
        text.includes(": HTTP 401") ||
        text.includes(": HTTP 403")
      ) {
        return fallback;
      }
      return current;
    });
  }, []);

  const clearRecoveryRetry = useCallback(() => {
    if (typeof window === "undefined" || !recoveryRetryTimerRef.current) {
      return;
    }

    window.clearTimeout(recoveryRetryTimerRef.current);
    recoveryRetryTimerRef.current = null;
  }, []);

  const scheduleRecoveryRetry = useCallback((delay = 1000) => {
    if (typeof window === "undefined" || recoveryRetryTimerRef.current) {
      return;
    }

    recoveryRetryTimerRef.current = window.setTimeout(() => {
      recoveryRetryTimerRef.current = null;
      refreshAllRef.current();
    }, delay);
  }, []);

  const clearDisconnectedData = useCallback(() => {
    setSessions([]);
    setLocks([]);
    setTasks([]);
    setFeedEntries([]);
    setMessageEntries([]);
    setActivityEntries([]);
    setConflictPairs([]);
    setSavings(null);
    setStats({
      memories: "--",
      decisions: "--",
      events: "--",
    });
  }, []);

  const refreshTokenForApi = useCallback(async () => {
    if (!invokeRef.current) {
      tokenRef.current = readPersistedBrowserAuthToken();
      return tokenRef.current;
    }
    try {
      const token = await invokeRef.current("read_auth_token");
      tokenRef.current = token || "";
      persistBrowserAuthToken(tokenRef.current);
    } catch { /* ignore */ }
    return tokenRef.current;
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

  const readAuthToken = useCallback(async ({ suppressFeedback = false } = {}) => {
    if (!invokeRef.current) {
      tokenRef.current = readPersistedBrowserAuthToken();
      return tokenRef.current;
    }

    if (invokeRef.current) {
      try {
        const token = await call("read_auth_token");
        tokenRef.current = token || "";
        persistBrowserAuthToken(tokenRef.current);
        return tokenRef.current;
      } catch (err) {
        tokenRef.current = "";
        persistBrowserAuthToken("");
        const message = err?.message || String(err);
        if (!suppressFeedback && (!daemonTransitionRef.current || !isDaemonOfflineErrorMessage(message))) {
          setFeedbackMessage(`Auth token read failed: ${message}`);
        }
      }
    }
    return tokenRef.current;
  }, [call]);

  const refreshDaemonState = useCallback(async () => {
    if (invokeRef.current) {
      try {
        const state = { ...EMPTY_DAEMON, ...(await call("daemon_status")) };
        setDaemonState(state);
        return state;
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
    if (isReachableHealthPayload(health)) {
      const nextState = {
        running: true,
        reachable: true,
        managed: false,
        authTokenReady: Boolean(tokenRef.current),
        pid: null,
        message: `Connected -- ${health.stats?.memories ?? 0} memories`,
      };
      setDaemonState(nextState);
      return nextState;
    } else {
      const nextState = {
        running: false,
        reachable: false,
        managed: false,
        authTokenReady: false,
        pid: null,
        message: `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`,
      };
      setDaemonState(nextState);
      return nextState;
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
      setHealthMeta(EMPTY_HEALTH_META);
      setStats({
        memories: "--",
        decisions: "--",
        events: "--",
      });
      return;
    }

    const next = health.stats;
    setHealthMeta({
      status: String(health?.status || "unknown").toLowerCase(),
      degraded: Boolean(health?.degraded),
      dbCorrupted: Boolean(health?.db_corrupted),
      runtimeVersion: String(health?.runtime?.version || ""),
    });
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
        apply: (v) => setTasks(Array.isArray(v?.tasks) ? v.tasks.map(normalizeTask) : []),
      },
    ]);
    clearTransientFeedback();
  }, [api, clearTransientFeedback]);

  const refreshFeed = useCallback(async () => {
    const query = new URLSearchParams();
    query.set("since", feedFilters.since);
    if (feedFilters.kind !== "all") query.set("kind", feedFilters.kind);
    if (feedFilters.unread && selectedOperatorName) {
      query.set("agent", selectedOperatorName);
      query.set("unread", "true");
    }

    const feedResult = await api(`/feed?${query.toString()}`, true);
    const entries = Array.isArray(feedResult?.entries) ? [...feedResult.entries].reverse() : [];
    setFeedEntries(filterFeedEntries(entries, feedFilters.agent));
    clearTransientFeedback();
  }, [api, clearTransientFeedback, feedFilters, selectedOperatorName]);

  const refreshMessages = useCallback(async () => {
    const operator = selectedOperatorName;
    if (!operator) {
      setMessageEntries([]);
      return;
    }

    const query = new URLSearchParams();
    query.set("agent", operator);
    const result = await api(`/messages?${query.toString()}`, true);
    const entries = Array.isArray(result?.messages) ? [...result.messages].reverse() : [];
    setMessageEntries(entries);
    clearTransientFeedback();
  }, [api, clearTransientFeedback, selectedOperatorName]);

  const refreshActivity = useCallback(async () => {
    const query = new URLSearchParams();
    query.set("since", activitySince);
    const result = await api(`/activity?${query.toString()}`, true);
    const entries = Array.isArray(result?.activities) ? [...result.activities].reverse() : [];
    setActivityEntries(entries);
    clearTransientFeedback();
  }, [activitySince, api, clearTransientFeedback]);

  const refreshSavings = useCallback(async () => {
    const result = await api("/savings", true);
    if (result) setSavings(result);
    clearTransientFeedback();
  }, [api, clearTransientFeedback]);

  const refreshConflicts = useCallback(async () => {
    const result = await api("/conflicts", true);
    setConflictPairs(Array.isArray(result?.pairs) ? result.pairs : []);
    clearTransientFeedback();
  }, [api, clearTransientFeedback]);

  const refreshProtectedData = useCallback(
    () => settledCollectErrors([
      refreshCoreData,
      refreshFeed,
      refreshMessages,
      refreshActivity,
      refreshSavings,
      refreshConflicts,
    ]),
    [
      refreshCoreData,
      refreshFeed,
      refreshMessages,
      refreshActivity,
      refreshSavings,
      refreshConflicts,
    ]
  );

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

  const openEditorSetupWizard = useCallback(async () => {
    setIsSettingUpEditors(true);
    try {
      const result = await call("detect_editors");
      setEditorDetections(result);
      setSelectedEditorIds(result.filter((entry) => entry.detected).map((entry) => entry.id));
      setShowEditorSetupWizard(true);
      const detected = result.filter((entry) => entry.detected).length;
      if (!detected) {
        setFeedbackMessage("Setup MCP found no supported clients. Use the manual snippet for other MCP-capable tools.");
      } else {
        setFeedbackMessage(`Setup MCP found ${detected} supported client(s). Review and apply the selections.`);
      }
    } catch (err) {
      setFeedbackMessage(`MCP setup scan: ${String(err)}`);
    } finally {
      setIsSettingUpEditors(false);
    }
  }, [call]);

  const toggleEditorSelection = useCallback((editorId) => {
    setSelectedEditorIds((current) =>
      current.includes(editorId)
        ? current.filter((id) => id !== editorId)
        : [...current, editorId],
    );
  }, []);

  const applyEditorSetup = useCallback(async () => {
    if (!selectedEditorIds.length) {
      setFeedbackMessage("Select at least one detected client before applying MCP setup.");
      return;
    }

    setIsSettingUpEditors(true);
    try {
      const result = await call("setup_editors", { editorIds: selectedEditorIds });
      setEditorSetup(result);
      setShowEditorSetupWizard(false);
      const detected = result.filter((entry) => entry.detected).length;
      const registered = result.filter((entry) => entry.registered).length;
      const failed = result.filter((entry) => entry.detected && !entry.registered).length;
      if (!detected) {
        setFeedbackMessage("Setup MCP found no supported clients on this machine.");
      } else if (failed) {
        setFeedbackMessage(`Setup MCP finished with ${failed} issue(s). Review client details in Overview.`);
      } else {
        setFeedbackMessage(`Setup MCP configured ${registered} client(s).`);
      }
    } catch (err) {
      setFeedbackMessage(`Editor setup: ${String(err)}`);
    } finally {
      setIsSettingUpEditors(false);
    }
  }, [call, selectedEditorIds]);

  const refreshAll = useCallback(async () => {
    try {
      invokeRef.current = await readTauriInvoke();
    } catch {
      invokeRef.current = null;
    }

    const nextDaemonState = await refreshDaemonState();
    await refreshHealth();

    if (daemonTransitionRef.current) {
      return;
    }

    if (invokeRef.current && nextDaemonState?.managed && !nextDaemonState?.reachable) {
      clearDisconnectedData();
      setFeedbackMessage("Daemon is still starting. Reconnect will continue automatically.");
      scheduleRecoveryRetry(1000);
      return;
    }

    if (!nextDaemonState?.reachable) {
      clearRecoveryRetry();
      if (invokeRef.current) {
        tokenRef.current = "";
        persistBrowserAuthToken("");
      }
      clearDisconnectedData();
      clearTransientFeedback(nextDaemonState?.message || `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`);
      return;
    }

    const authToken = await readAuthToken({ suppressFeedback: true });
    if (invokeRef.current && !authToken) {
      clearDisconnectedData();
      setFeedbackMessage("Waiting for daemon auth token to finish rotating...");
      scheduleRecoveryRetry(1000);
      return;
    }

    let errors = await refreshProtectedData();
    if (invokeRef.current && errors.length && errors.every((error) => isAuthFailure(error))) {
      const refreshedToken = await readAuthToken({ suppressFeedback: true });
      if (refreshedToken) {
        errors = await refreshProtectedData();
      }
    }

    if (errors.length) {
      const unique = [...new Set(errors)];
      if (unique.every((error) => isDaemonOfflineErrorMessage(error))) {
        clearDisconnectedData();
        clearTransientFeedback(nextDaemonState?.message || `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`);
        scheduleRecoveryRetry(1000);
      } else if (invokeRef.current && unique.every((error) => isAuthFailure(error))) {
        setFeedbackMessage("Waiting for daemon auth token to finish rotating...");
        scheduleRecoveryRetry(1000);
      } else {
        clearRecoveryRetry();
        setFeedbackMessage(summarizeDashboardErrors(unique));
        if (!invokeRef.current && unique.every((error) => isAuthFailure(error))) {
          setShowConnectionDialog(true);
        }
      }
    } else {
      clearRecoveryRetry();
      clearTransientFeedback();
    }
  }, [
    clearRecoveryRetry,
    clearTransientFeedback,
    readAuthToken,
    refreshDaemonState,
    refreshHealth,
    refreshProtectedData,
    clearDisconnectedData,
    cortexBase,
    scheduleRecoveryRetry,
  ]);

  useEffect(() => {
    localStorage.setItem(CORTEX_BASE_STORAGE_KEY, cortexBase);
    refreshAllRef.current();
  }, [cortexBase]);

  useEffect(() => {
    localStorage.setItem("cortex_currency", currency);
  }, [currency]);

  useEffect(() => {
    localStorage.setItem("cortex_analytics_mode", analyticsMode);
  }, [analyticsMode]);

  useEffect(() => {
    if (typeof window === "undefined") return undefined;
    const syncViewport = () => {
      setIsNarrowViewport(window.innerWidth <= SIDEBAR_COLLAPSE_BREAKPOINT_PX);
    };
    syncViewport();
    window.addEventListener("resize", syncViewport);
    return () => window.removeEventListener("resize", syncViewport);
  }, []);

  useEffect(() => {
    try {
      if (selectedOperatorName) {
        localStorage.setItem(CORTEX_OPERATOR_STORAGE_KEY, selectedOperatorName);
      } else {
        localStorage.removeItem(CORTEX_OPERATOR_STORAGE_KEY);
      }
    } catch {
      // Ignore storage failures in restricted browser contexts.
    }
  }, [selectedOperatorName]);

  useEffect(() => {
    try {
      localStorage.setItem(CORTEX_PANEL_STORAGE_KEY, panel);
    } catch {
      // Ignore storage failures in restricted browser contexts.
    }
  }, [panel]);

  useEffect(() => {
    if (panel === "brain") {
      setHasVisitedBrain(true);
    }
    if (panel === "analytics") {
      setHasVisitedAnalytics(true);
    }
  }, [panel]);

  useEffect(() => {
    if (hasVisitedAnalytics) return;

    const warmupTimer = window.setTimeout(() => {
      startTransition(() => {
        setHasVisitedAnalytics(true);
        setAnalyticsReady(true);
      });
    }, 250);

    return () => {
      window.clearTimeout(warmupTimer);
    };
  }, [hasVisitedAnalytics]);

  useEffect(() => {
    if (panel !== "analytics" || analyticsReady) {
      return;
    }

    let frameOne = 0;
    let frameTwo = 0;
    frameOne = requestAnimationFrame(() => {
      frameTwo = requestAnimationFrame(() => {
        setAnalyticsReady(true);
      });
    });

    return () => {
      cancelAnimationFrame(frameOne);
      cancelAnimationFrame(frameTwo);
    };
  }, [analyticsReady, panel]);

  useEffect(() => {
    refreshAllRef.current = refreshAll;
  }, [refreshAll]);

  useEffect(() => () => {
    clearRecoveryRetry();
  }, [clearRecoveryRetry]);

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
    if (selectedOperator.trim()) return;
    const defaultAgent = knownAgents[0];
    if (defaultAgent) setSelectedOperator(defaultAgent);
  }, [knownAgents, selectedOperator]);

  useEffect(() => {
    if (messageTarget.trim()) return;
    const fallbackTarget = knownAgents.find((agent) => !sameAgent(agent, selectedOperator));
    if (fallbackTarget) setMessageTarget(fallbackTarget);
  }, [knownAgents, messageTarget, selectedOperator]);

  useEffect(() => {
    if (skipInitialMessagesRefreshRef.current) {
      skipInitialMessagesRefreshRef.current = false;
      return;
    }
    refreshMessages().catch((error) => {
      const message = error?.message || String(error);
      if (!message || isDaemonOfflineErrorMessage(message)) return;
      setFeedbackMessage(summarizeDashboardErrors([message]) || message);
    });
  }, [refreshMessages]);

  useEffect(() => {
    if (skipInitialActivityRefreshRef.current) {
      skipInitialActivityRefreshRef.current = false;
      return;
    }
    refreshActivity().catch((error) => {
      const message = error?.message || String(error);
      if (!message || isDaemonOfflineErrorMessage(message)) return;
      setFeedbackMessage(summarizeDashboardErrors([message]) || message);
    });
  }, [refreshActivity]);

  useEffect(() => {
    if (panel !== "analytics" || !analyticsReady) return;
    refreshSavings().catch((error) => {
      const message = error?.message || String(error);
      if (!message || isDaemonOfflineErrorMessage(message)) return;
      setFeedbackMessage(summarizeDashboardErrors([message]) || message);
    });
    const timer = setInterval(() => {
      refreshSavings().catch((error) => {
        const message = error?.message || String(error);
        if (!message || isDaemonOfflineErrorMessage(message)) return;
        setFeedbackMessage(summarizeDashboardErrors([message]) || message);
      });
    }, ANALYTICS_REFRESH_MS);
    return () => clearInterval(timer);
  }, [analyticsReady, panel, refreshSavings]);

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

  const dailySeries = useMemo(
    () => (Array.isArray(savings?.daily) ? savings.daily : []),
    [savings]
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

  const bootSavingsMomentum = useMemo(() => {
    if (dailySeries.length < 4) return null;
    const recent = dailySeries.slice(-4);
    const previous = dailySeries.slice(-8, -4);
    if (!previous.length) return null;
    const recentAverage = recent.reduce((sum, point) => sum + Number(point.saved || 0), 0) / recent.length;
    const previousAverage = previous.reduce((sum, point) => sum + Number(point.saved || 0), 0) / previous.length;
    if (previousAverage <= 0) return null;
    return Math.round(((recentAverage - previousAverage) / previousAverage) * 100);
  }, [dailySeries]);

  const recentRecallWindow = useMemo(
    () => recallTrendSeries.slice(-7),
    [recallTrendSeries]
  );

  const latestRecallPoint = useMemo(
    () => recallTrendSeries.at(-1) || null,
    [recallTrendSeries]
  );

  const stableRecallHeadlinePoint = useMemo(() => {
    if (!latestRecallPoint) return null;
    if (Number(latestRecallPoint.queries || 0) >= RECALL_HEADLINE_MIN_QUERIES) {
      return latestRecallPoint;
    }
    return [...recentRecallWindow]
      .reverse()
      .find((point) => Number(point?.queries || 0) >= RECALL_HEADLINE_MIN_QUERIES)
      || latestRecallPoint;
  }, [latestRecallPoint, recentRecallWindow]);

  const latestRecallHitRate = useMemo(
    () => Math.round(Number(stableRecallHeadlinePoint?.hitRatePct || latestRecallPoint?.hitRatePct || 0)),
    [latestRecallPoint, stableRecallHeadlinePoint]
  );

  const latestRecallSampleSize = useMemo(
    () => Number(latestRecallPoint?.queries || 0),
    [latestRecallPoint]
  );

  const recallHeadlineUsesFallback = useMemo(
    () => Boolean(
      latestRecallPoint
        && stableRecallHeadlinePoint
        && stableRecallHeadlinePoint !== latestRecallPoint
        && latestRecallSampleSize < RECALL_HEADLINE_MIN_QUERIES
    ),
    [latestRecallPoint, latestRecallSampleSize, stableRecallHeadlinePoint]
  );

  const recallWindowAverage = useMemo(() => {
    if (!recentRecallWindow.length) return 0;
    return Math.round(
      recentRecallWindow.reduce((sum, point) => sum + Number(point.hitRatePct || 0), 0) / recentRecallWindow.length
    );
  }, [recentRecallWindow]);

  const recallWindowSpread = useMemo(() => {
    if (!recentRecallWindow.length) return 0;
    const values = recentRecallWindow.map((point) => Number(point.hitRatePct || 0));
    return Math.round(Math.max(...values) - Math.min(...values));
  }, [recentRecallWindow]);

  const monteCarloProjection = useMemo(
    () => buildMonteCarloProjection(dailySeries, cumulativeSeries),
    [dailySeries, cumulativeSeries]
  );

  const topFeedEntries = useMemo(
    () => feedEntries.slice(0, 5),
    [feedEntries]
  );

  const topActivityEntries = useMemo(
    () => activityEntries.slice(0, 5),
    [activityEntries]
  );

  const sidebarUtilityStats = useMemo(
    () => [
      { label: "Queue", value: pendingTasks.length, tone: pendingTasks.length ? "warning" : "calm" },
      { label: "Locks", value: locks.length, tone: locks.length ? "cyan" : "calm" },
      { label: "Recall", value: `${latestRecallHitRate || 0}%`, tone: latestRecallHitRate >= 85 ? "green" : "warning" },
      { label: "Conflicts", value: conflictPairs.length, tone: conflictPairs.length ? "danger" : "calm" },
    ],
    [pendingTasks.length, locks.length, latestRecallHitRate, conflictPairs.length]
  );

  const runtimeVersionMismatch = useMemo(
    () => Boolean(healthMeta.runtimeVersion) && healthMeta.runtimeVersion !== CONTROL_CENTER_VERSION,
    [healthMeta.runtimeVersion]
  );

  const daemonStatusBadge = useMemo(() => {
    if (!daemonState.reachable) {
      return {
        className: "offline",
        label: "○ OFFLINE",
        title: daemonState.message || `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`,
      };
    }
    if (healthMeta.dbCorrupted) {
      return {
        className: "warning",
        label: "▲ DB WARN",
        title: "Database integrity checks are failing. Restart Cortex to trigger repair.",
      };
    }
    if (healthMeta.degraded) {
      return {
        className: "warning",
        label: "▲ DEGRADED",
        title: "Semantic search is in fallback mode. Restart Cortex if this persists.",
      };
    }
    return {
      className: "online",
      label: "● ONLINE",
      title: daemonState.message || "Cortex daemon reachable.",
    };
  }, [cortexBase, daemonState.message, daemonState.reachable, healthMeta.dbCorrupted, healthMeta.degraded]);

  const daemonRecoveryHint = useMemo(() => {
    if (!daemonState.reachable) {
      return "";
    }
    if (healthMeta.dbCorrupted) {
      return "Database integrity checks are failing. Restart Cortex to trigger repair and inspect the daemon if it stays degraded.";
    }
    if (runtimeVersionMismatch) {
      return `Connected to daemon v${healthMeta.runtimeVersion}. Restart from Control Center to switch to v${CONTROL_CENTER_VERSION}.`;
    }
    if (healthMeta.degraded) {
      return "Semantic search is using keyword fallback right now. Restart Cortex if this state does not clear.";
    }
    return "";
  }, [daemonState.reachable, healthMeta.dbCorrupted, healthMeta.degraded, healthMeta.runtimeVersion, runtimeVersionMismatch]);

  const reportSurfaceError = useCallback((error) => {
    const message = error?.message || String(error);
    if (!message || isDaemonOfflineErrorMessage(message)) return;
    setFeedbackMessage(summarizeDashboardErrors([message]) || message);
  }, []);

  const handleTaskClaim = useCallback(async (task) => {
    const operator = selectedOperatorName;
    if (!operator) {
      setFeedbackMessage("Select an operator before claiming tasks.");
      return;
    }

    setBusyActionKey(`claim:${task.taskId}`);
    try {
      await postApi("/tasks/claim", { taskId: task.taskId, agent: operator });
      setFeedbackMessage(`Claimed ${task.title}.`);
      await refreshCoreData();
    } catch (error) {
      reportSurfaceError(error);
    } finally {
      setBusyActionKey("");
    }
  }, [postApi, refreshCoreData, reportSurfaceError, selectedOperatorName]);

  const handleTaskAbandon = useCallback(async (task) => {
    const operator = selectedOperatorName;
    if (!operator) {
      setFeedbackMessage("Select an operator before abandoning tasks.");
      return;
    }

    setBusyActionKey(`abandon:${task.taskId}`);
    try {
      await postApi("/tasks/abandon", { taskId: task.taskId, agent: operator });
      setFeedbackMessage(`Returned ${task.title} to pending.`);
      setCompletionTaskId("");
      await refreshCoreData();
    } catch (error) {
      reportSurfaceError(error);
    } finally {
      setBusyActionKey("");
    }
  }, [postApi, refreshCoreData, reportSurfaceError, selectedOperatorName]);

  const handleTaskComplete = useCallback(async (task, summary) => {
    const operator = selectedOperatorName;
    if (!operator) {
      setFeedbackMessage("Select an operator before completing tasks.");
      return;
    }

    setBusyActionKey(`complete:${task.taskId}`);
    try {
      await postApi("/tasks/complete", {
        taskId: task.taskId,
        agent: operator,
        summary: summary.trim() || undefined,
      });
      setFeedbackMessage(`Completed ${task.title}.`);
      setCompletionTaskId("");
      setTaskCompletionDrafts((current) => ({ ...current, [task.taskId]: "" }));
      await Promise.all([refreshCoreData(), refreshFeed()]);
    } catch (error) {
      reportSurfaceError(error);
    } finally {
      setBusyActionKey("");
    }
  }, [postApi, refreshCoreData, refreshFeed, reportSurfaceError, selectedOperatorName]);

  const handleTaskDelete = useCallback(async (task) => {
    setBusyActionKey(`delete:${task.taskId}`);
    try {
      await postApi("/tasks/delete", { taskId: task.taskId });
      setFeedbackMessage(`Deleted ${task.title}.`);
      await refreshCoreData();
    } catch (error) {
      reportSurfaceError(error);
    } finally {
      setBusyActionKey("");
    }
  }, [postApi, refreshCoreData, reportSurfaceError]);

  const handleUnlock = useCallback(async (lock) => {
    const operator = selectedOperatorName;
    if (!operator) {
      setFeedbackMessage("Select an operator before unlocking files.");
      return;
    }

    setBusyActionKey(`unlock:${lock.path}`);
    try {
      await postApi("/unlock", { path: lock.path, agent: operator });
      setFeedbackMessage(`Unlocked ${lock.path}.`);
      await refreshCoreData();
    } catch (error) {
      reportSurfaceError(error);
    } finally {
      setBusyActionKey("");
    }
  }, [postApi, refreshCoreData, reportSurfaceError, selectedOperatorName]);

  const handleSendMessage = useCallback(async (event) => {
    event?.preventDefault();
    const operator = selectedOperatorName;
    const recipient = messageTargetName;
    const message = messageDraft.trim();

    if (!operator) {
      setFeedbackMessage("Select an operator before sending messages.");
      return;
    }
    if (!recipient) {
      setFeedbackMessage("Choose a recipient before sending a message.");
      return;
    }
    if (!message) {
      setFeedbackMessage("Write a message before sending it.");
      return;
    }

    setBusyActionKey("message:send");
    try {
      await postApi("/message", { from: operator, to: recipient, message });
      setMessageDraft("");
      setFeedbackMessage(`Sent message from ${operator} to ${recipient}.`);
      await refreshMessages();
    } catch (error) {
      reportSurfaceError(error);
    } finally {
      setBusyActionKey("");
    }
  }, [messageDraft, messageTargetName, postApi, refreshMessages, reportSurfaceError, selectedOperatorName]);

  const handleFeedAck = useCallback(async () => {
    const operator = selectedOperatorName;
    const lastSeenId = nextFeedAckId(feedEntries, operator);

    if (!operator) {
      setFeedbackMessage("Select an operator before acknowledging feed entries.");
      return;
    }
    if (!lastSeenId) {
      setFeedbackMessage("No visible teammate feed entries to acknowledge.");
      return;
    }

    setBusyActionKey("feed:ack");
    try {
      await postApi("/feed/ack", { agent: operator, lastSeenId });
      setFeedbackMessage(`Acknowledged the visible feed for ${operator}.`);
      await refreshFeed();
    } catch (error) {
      reportSurfaceError(error);
    } finally {
      setBusyActionKey("");
    }
  }, [feedEntries, postApi, refreshFeed, reportSurfaceError, selectedOperatorName]);

  const waitForDaemonReachable = useCallback(async () => {
    const started = Date.now();
    while (Date.now() - started < DAEMON_START_WAIT_TIMEOUT_MS) {
      try {
        if (invokeRef.current) {
          const state = { ...EMPTY_DAEMON, ...(await call("daemon_status")) };
          setDaemonState(state);
          if (state?.reachable) return true;
        } else {
          const health = await api("/health");
          if (isReachableHealthPayload(health)) return true;
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
      await readAuthToken({ suppressFeedback: true });
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
      const offline = await waitForDaemonOffline();
      tokenRef.current = "";
      persistBrowserAuthToken("");
      if (offline) {
        clearDisconnectedData();
        setDaemonState({
          running: false,
          reachable: false,
          managed: false,
          authTokenReady: false,
          pid: null,
          message: `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`,
        });
        setFeedbackMessage(result.message || "Stopped Cortex daemon.");
      } else {
        setFeedbackMessage("Shutdown is taking longer than expected. Waiting for daemon to go offline...");
        await refreshAll();
      }
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
        const stopPromise = call("stop_daemon")
          .then((result) => ({ ok: true, result }))
          .catch((error) => ({ ok: false, error: error?.message || String(error) }));
        const stopResult = await Promise.race([
          stopPromise,
          new Promise((resolve) => setTimeout(() => resolve({ timedOut: true }), DAEMON_STOP_HANG_TIMEOUT_MS)),
        ]);
        let stopFailure = "";
        if (stopResult?.timedOut) {
          setFeedbackMessage("Shutdown is taking longer than expected. Waiting for daemon to go offline...");
        } else if (!stopResult?.ok) {
          stopFailure = stopResult?.error || "Existing daemon rejected shutdown.";
        }
        const stopped = await waitForDaemonOffline();
        if (!stopped) {
          throw new Error(stopFailure || "Existing daemon did not stop cleanly.");
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
      await readAuthToken({ suppressFeedback: true });
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
      const idx = PANEL_SEQUENCE.findIndex(p => p.key === panel);
      if (e.key === "ArrowDown" || e.key === "j") {
        e.preventDefault();
        changePanel(PANEL_SEQUENCE[(idx + 1) % PANEL_SEQUENCE.length].key);
      } else if (e.key === "ArrowUp" || e.key === "k") {
        e.preventDefault();
        changePanel(PANEL_SEQUENCE[(idx - 1 + PANEL_SEQUENCE.length) % PANEL_SEQUENCE.length].key);
      } else {
        const num = parseInt(e.key);
        if (num >= 1 && num <= PANEL_SEQUENCE.length) {
          e.preventDefault();
          changePanel(PANEL_SEQUENCE[num - 1].key);
        }
      }
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [changePanel, panel]);

  const effectiveSidebarCollapsed = sidebarCollapsed || isNarrowViewport;
  const canStartDaemon = Boolean(invokeRef.current && !restartingDaemon && !daemonState.reachable);
  const canStopDaemon = Boolean(invokeRef.current && !restartingDaemon && (daemonState.reachable || daemonState.running));

  return (
    <div className={`app ${effectiveSidebarCollapsed ? "sidebar-collapsed" : ""}`}>
      <aside className={`sidebar ${effectiveSidebarCollapsed ? "collapsed" : ""}`}>
        <div className="sidebar-header">
          <div className="logo">
            <span>Cortex</span>
          </div>
          <div className={pill.className}>{pill.label}</div>
        </div>

        <nav className="sidebar-nav">
          {PANEL_SEQUENCE.map((item, idx) => (
            <button
              key={item.key}
              type="button"
              className={`nav-item ${panel === item.key ? "active" : ""}`}
              onClick={() => changePanel(item.key)}
              data-key={idx + 1}
            >
              <span style={{ opacity: 0.5, fontSize: "12px" }}><AppIcon name={item.icon} /></span>
              {item.label}
            </button>
          ))}
        </nav>

        <div className="sidebar-utility">
          <div className="sidebar-utility-header">
            <span className="sidebar-utility-kicker">Mission status</span>
            <span className={`sidebar-utility-pill ${daemonState.reachable ? "online" : "offline"}`}>
              {daemonState.reachable ? "Live" : "Wait"}
            </span>
          </div>
          <div className="sidebar-utility-grid">
            {sidebarUtilityStats.map((item) => (
              <div key={item.label} className={`sidebar-utility-card tone-${item.tone}`}>
                <span className="sidebar-utility-label">{item.label}</span>
                <strong className="sidebar-utility-value">{item.value}</strong>
              </div>
            ))}
          </div>
          <div className="sidebar-utility-note">
            <span className="sidebar-utility-note-label">Focus</span>
            <strong>{PANEL_SEQUENCE.find((item) => item.key === panel)?.label || "Overview"}</strong>
            <p>{daemonState.message}</p>
            {daemonRecoveryHint ? <p className="sidebar-utility-alert">{daemonRecoveryHint}</p> : null}
          </div>
        </div>

        <div className="sidebar-footer">
          <div className="daemon-restart-row">
            <button
              type="button"
              className="btn-ctrl btn-restart"
              onClick={handleRestartDaemon}
              disabled={restartingDaemon || !invokeRef.current}
            >
              {restartingDaemon ? "Restarting..." : "Restart"}
            </button>
          </div>
          <div className="daemon-controls-grid">
            <button type="button" className="btn-ctrl btn-primary" onClick={handleStartDaemon} disabled={!canStartDaemon}>Start</button>
            <button type="button" className="btn-ctrl" onClick={handleStopDaemon} disabled={!canStopDaemon}>Stop</button>
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
          <button
            type="button"
            className="btn-sidebar-collapse"
            aria-label={effectiveSidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
            title={effectiveSidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
            onClick={() => setSidebarCollapsed(c => !c)}
          >
            <AppIcon name={effectiveSidebarCollapsed ? "chevron-right" : "chevron-left"} size={16} />
          </button>
        </div>
      </aside>

      <main className="content">
        <div className={`topbar ${panel === "overview" ? "topbar-hidden" : ""}`} aria-hidden={panel === "overview" ? true : undefined}>
          <div className="topbar-left">
            <span className="topbar-path">CORTEX</span>
            <span className="topbar-sep">/</span>
            <span className="topbar-current">{PANEL_SEQUENCE.find(p => p.key === panel)?.label.toUpperCase()}</span>
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
            <span className={`topbar-status ${daemonStatusBadge.className}`} title={daemonStatusBadge.title}>
              {daemonStatusBadge.label}
            </span>
          </div>
        </div>

        {showEditorSetupWizard && (
          <div className="connection-overlay" onClick={() => !isSettingUpEditors && setShowEditorSetupWizard(false)}>
            <div className="connection-dialog editor-setup-dialog" onClick={(e) => e.stopPropagation()}>
              <div className="editor-setup-dialog-header">
                <div>
                  <span className="editor-setup-kicker">Shared MCP Registration</span>
                  <h2>Setup MCP</h2>
                </div>
                <span className="badge">
                  {editorDetectionSummary.detected}/{editorDetectionSummary.results.length}
                </span>
              </div>
              <p className="connection-subtitle">
                Choose which supported clients should receive the shared Cortex attach-only MCP entry. Every client points at the same
                app-owned daemon command.
              </p>
              <div className="editor-setup-choice-list">
                {editorDetectionSummary.results.map((entry) => {
                  const tone = !entry.detected ? "idle" : entry.registered ? "ok" : "warn";
                  const stateLabel = !entry.detected ? "Not detected" : entry.registered ? "Configured" : "Detected";
                  const selected = selectedEditorIds.includes(entry.id);
                  return (
                    <label key={entry.id} className={`editor-setup-choice ${tone} ${!entry.detected ? "disabled" : ""}`}>
                      <input
                        type="checkbox"
                        checked={selected}
                        disabled={!entry.detected || isSettingUpEditors}
                        onChange={() => toggleEditorSelection(entry.id)}
                      />
                      <div className="editor-setup-choice-body">
                        <div className="editor-setup-item-head">
                          <span className="editor-setup-name">{entry.name}</span>
                          <span className="editor-setup-state">{stateLabel}</span>
                        </div>
                        {entry.configPath ? <code>{entry.configPath}</code> : null}
                        <p>{entry.message || "No detail provided."}</p>
                      </div>
                    </label>
                  );
                })}
              </div>
              <div className="editor-setup-manual">
                <span className="editor-setup-kicker">Manual Fallback</span>
                <p>If a client is missing from the supported list, register this MCP server manually or paste it into that AI’s setup flow:</p>
                <pre>{manualMcpSnippet}</pre>
              </div>
              <div className="connection-actions">
                <button type="button" className="btn-sm" onClick={() => setShowEditorSetupWizard(false)} disabled={isSettingUpEditors}>
                  Cancel
                </button>
                <button
                  type="button"
                  className="btn-sm btn-primary"
                  onClick={applyEditorSetup}
                  disabled={isSettingUpEditors || !selectedEditorIds.length}
                >
                  {isSettingUpEditors ? "Applying..." : `Apply to ${selectedEditorIds.length} Client${selectedEditorIds.length === 1 ? "" : "s"}`}
                </button>
              </div>
            </div>
          </div>
        )}

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
                tokenRef.current = token || "";
                persistBrowserAuthToken(token || "");
                setShowConnectionDialog(false);
                queueMicrotask(() => refreshAllRef.current());
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
                    persistBrowserAuthToken("");
                    setShowConnectionDialog(false);
                    readAuthToken({ suppressFeedback: true });
                    queueMicrotask(() => refreshAllRef.current());
                  }}>Reset to Local</button>
                  <button type="submit" className="btn-sm btn-primary">Connect</button>
                </div>
              </form>
            </div>
          </div>
        )}

        <div className="panel-stage" data-panel-direction={panelMotionDirection}>
        {panel === "overview" ? (
          <section className="panel active">
            <div className="panel-header overview-panel-header">
              <div>
                <h1>Overview</h1>
                <p className="panel-subtitle">Command center for analytics, live agent traffic, and memory quality.</p>
              </div>
              <div className="surface-actions">
                <button type="button" className="btn-sm" onClick={refreshAll}>
                  Refresh
                </button>
                <button
                  type="button"
                  className="btn-sm btn-primary"
                  onClick={openEditorSetupWizard}
                  disabled={isSettingUpEditors}
                >
                  {isSettingUpEditors ? "Setting Up..." : "Setup MCP"}
                </button>
              </div>
            </div>

            <div className="metrics overview-metrics">
              <div className="metric" data-accent="cyan">
                <span className="metric-value"><AnimatedNumber value={typeof stats.memories === "number" ? stats.memories : 0} /></span>
                <span className="metric-label">Memories</span>
                <span className="metric-icon"><AppIcon name="memory" /></span>
              </div>
              <div className="metric" data-accent="blue">
                <span className="metric-value"><AnimatedNumber value={typeof stats.decisions === "number" ? stats.decisions : 0} /></span>
                <span className="metric-label">Decisions</span>
                <span className="metric-icon"><AppIcon name="decision" /></span>
              </div>
              <div className="metric" data-accent="purple">
                <span className="metric-value"><AnimatedNumber value={typeof stats.events === "number" ? stats.events : 0} /></span>
                <span className="metric-label">Events</span>
                <span className="metric-icon"><AppIcon name="event" /></span>
              </div>
              <div className="metric" data-accent="green">
                <span className="metric-value"><AnimatedNumber value={normalizedSessions.length} /></span>
                <span className="metric-label">Active Agents</span>
                <span className="metric-icon"><AppIcon name="agents" /></span>
              </div>
              <div className="metric" data-accent="blue">
                <span className="metric-value">{formatCompactNumber(Number(savings?.summary?.totalSaved || 0))}</span>
                <span className="metric-label">Saved Tokens</span>
                <span className="metric-icon"><AppIcon name="token" /></span>
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
                <span className="sys-label">HOST</span>
                <span className="sys-value">
                  {cortexBase === DEFAULT_CORTEX_BASE ? "LOCAL" : (() => { try { return new URL(cortexBase).hostname; } catch { return "?"; } })()}
                </span>
              </div>
              <div className="sys-item">
                <span className="sys-label">LOCKS</span>
                <span className="sys-value">{locks.length} ACTIVE</span>
              </div>
              <div className="sys-item">
                <span className="sys-label">TASKS</span>
                <span className="sys-value">{pendingTasks.length} PENDING</span>
              </div>
              <div
                className={`sys-item sys-item-action ${isSettingUpEditors ? "sys-item-disabled" : ""}`}
                  onClick={isSettingUpEditors ? undefined : openEditorSetupWizard}
                  title="Preview and register Cortex MCP in supported clients"
              >
                <span className="sys-label">MCP</span>
                <span className="sys-value">
                  {isSettingUpEditors ? "WORKING" : editorSetup ? `${editorSetupSummary.registered} EDITORS` : "SETUP"}
                </span>
              </div>
              <div className="sys-item sys-item-action" onClick={() => changePanel("memory")} title="Open memory health and conflict resolution">
                <span className="sys-label">RECALL</span>
                <span className={`sys-value ${latestRecallHitRate >= 85 ? "sys-ok" : ""}`}>{latestRecallHitRate || 0}%</span>
              </div>
            </div>

            {editorSetupSummary.results.length ? (
              <div className="editor-setup-panel">
                <div className="editor-setup-header">
                  <div>
                    <span className="editor-setup-kicker">MCP Registration</span>
                    <h2>Editor setup results</h2>
                  </div>
                  <span className="badge">
                    {editorSetupSummary.registered}/{editorSetupSummary.detected || editorSetupSummary.results.length}
                  </span>
                </div>
                <div className="editor-setup-grid">
                  {editorSetupSummary.results.map((entry) => {
                    const tone = !entry.detected ? "idle" : entry.registered ? "ok" : "warn";
                    const stateLabel = !entry.detected ? "Not detected" : entry.registered ? "Configured" : "Needs attention";
                    return (
                      <div key={entry.name} className={`editor-setup-item ${tone}`}>
                        <div className="editor-setup-item-head">
                          <span className="editor-setup-name">{entry.name}</span>
                          <span className="editor-setup-state">{stateLabel}</span>
                        </div>
                        <p>{entry.message || "No detail provided."}</p>
                      </div>
                    );
                  })}
                </div>
              </div>
            ) : null}

            <div className="overview-dashboard-grid">
              <div className="card overview-hero-card overview-span-2">
                <div className="card-header">
                  <h2>Mission Control</h2>
                  <span className="badge">{formatCurrency(((savings?.summary?.totalSaved || 0) * SAVINGS_USD_PER_MILLION) / 1000000)}</span>
                </div>
                <p className="chart-summary">
                  Overview now behaves like a command deck instead of a spacer page: analytics, work, and memory quality are visible immediately.
                </p>
                <div className="overview-summary-grid">
                  <div className="overview-summary-card">
                    <span className="overview-summary-label">30d median gain</span>
                    <strong>{formatSignedCompactNumber(Number(monteCarloProjection?.summary?.p50Gain || 0))}t</strong>
                    <span>{monteCarloProjection ? `${monteCarloProjection.simulationCount} deterministic sims` : "Waiting for more history"}</span>
                  </div>
                  <div className="overview-summary-card">
                    <span className="overview-summary-label">Current run-rate</span>
                    <strong>{formatCompactNumber(Number(monteCarloProjection?.summary?.avgDaily || 0))}t/day</strong>
                    <span>{bootSavingsMomentum === null ? "Momentum pending" : `${bootSavingsMomentum >= 0 ? "+" : ""}${bootSavingsMomentum}% vs prior window`}</span>
                  </div>
                  <div className="overview-summary-card">
                    <span className="overview-summary-label">Work in flight</span>
                    <strong>{claimedTasks.length + pendingTasks.length}</strong>
                    <span>{claimedTasks.length} claimed / {pendingTasks.length} pending</span>
                  </div>
                  <div className="overview-summary-card">
                    <span className="overview-summary-label">Memory load</span>
                    <strong>{memoryLoad}</strong>
                    <span>{stats.memories} memories / {stats.decisions} decisions</span>
                  </div>
                </div>
                <div className="overview-hero-actions">
                  <button type="button" className="btn-sm btn-primary" onClick={() => changePanel("analytics")}>Open Analytics</button>
                  <button type="button" className="btn-sm" onClick={() => changePanel("brain")}>Open Brain</button>
                  <button type="button" className="btn-sm" onClick={() => changePanel("work")}>Open Work</button>
                </div>
              </div>

              <div className="card overview-status-card">
                <div className="card-header">
                  <h2>Memory Health</h2>
                  <span className="badge">{latestRecallHitRate || 0}%</span>
                </div>
                <div className="overview-status-list">
                  <div className="overview-status-row">
                    <span>Latest recall hit rate</span>
                    <strong>{latestRecallHitRate || 0}%</strong>
                  </div>
                  <div className="overview-status-row">
                    <span>7-day average</span>
                    <strong>{recallWindowAverage || 0}%</strong>
                  </div>
                  <div className="overview-status-row">
                    <span>Spread</span>
                    <strong>{recallWindowSpread || 0} pts</strong>
                  </div>
                  <div className="overview-status-row">
                    <span>Conflict pairs</span>
                    <strong>{conflictPairs.length}</strong>
                  </div>
                </div>
                    <button type="button" className="btn-sm" onClick={() => changePanel("memory")}>
                  Open Memory Surface
                </button>
              </div>

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
                  <h2>Recent Activity</h2>
                  <span className="badge">{topActivityEntries.length}</span>
                </div>
                <ul className="item-list">
                  {topActivityEntries.length ? topActivityEntries.map((entry) => <ActivityItem key={entry.id} entry={entry} />) : <EmptyItem text="No recent activity" />}
                </ul>
              </div>

              <div className="card">
                <div className="card-header">
                  <h2>Recent Feed</h2>
                  <span className="badge">{topFeedEntries.length}</span>
                </div>
                <ul className="item-list">
                  {topFeedEntries.length ? topFeedEntries.map((entry) => <FeedItem key={entry.id} entry={entry} />) : <EmptyItem text="No feed entries" />}
                </ul>
              </div>

              <div className="card">
                <div className="card-header">
                  <h2>Queue & Locks</h2>
                  <span className="badge">{pendingTasks.length + locks.length}</span>
                </div>
                <div className="overview-dual-stack">
                  <div>
                    <div className="overview-stack-title">Work Queue</div>
                    <ul className="item-list compact-list">
                      {recentOverviewTasks.length ? recentOverviewTasks.map((task) => <TaskItem key={task.taskId} task={task} />) : <EmptyItem text="No active tasks" />}
                    </ul>
                  </div>
                  <div>
                    <div className="overview-stack-title">File Locks</div>
                    <ul className="item-list compact-list">
                      {locks.length ? locks.slice(0, 4).map((lock) => <LockItem key={lock.id || `${lock.path}:${lock.agent}`} lock={lock} />) : <EmptyItem text="No active locks" />}
                    </ul>
                  </div>
                </div>
              </div>
            </div>
          </section>
        ) : null}

        {panel === "__legacy_overview" ? (
          <section className="panel active">
            <div className="panel-header">
              <div>
                <h1>Overview</h1>
                <span className="panel-subtitle">Command center for live work, recall health, and the brain surface.</span>
              </div>
              <div className="surface-actions">
                <button type="button" className="btn-sm" onClick={refreshAll}>
                  Refresh
                </button>
                  <button type="button" className="btn-sm" onClick={() => changePanel("analytics")}>
                  Analytics
                </button>
                  <button type="button" className="btn-sm btn-primary" onClick={() => changePanel("brain")}>
                  Open Brain
                </button>
              </div>
            </div>

            <div className="metrics">
              <div className="metric" data-accent="cyan">
                <span className="metric-value"><AnimatedNumber value={typeof stats.memories === "number" ? stats.memories : 0} /></span>
                <span className="metric-label">Memories</span>
                <span className="metric-icon"><AppIcon name="memory" /></span>
              </div>
              <div className="metric" data-accent="blue">
                <span className="metric-value"><AnimatedNumber value={typeof stats.decisions === "number" ? stats.decisions : 0} /></span>
                <span className="metric-label">Decisions</span>
                <span className="metric-icon"><AppIcon name="decision" /></span>
              </div>
              <div className="metric" data-accent="purple">
                <span className="metric-value"><AnimatedNumber value={typeof stats.events === "number" ? stats.events : 0} /></span>
                <span className="metric-label">Events</span>
                <span className="metric-icon"><AppIcon name="event" /></span>
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
              <div className="sys-item sys-item-action" onClick={openEditorSetupWizard} title="Preview and register Cortex MCP in supported clients">
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
              <div>
                <h1>Agents</h1>
                <p className="panel-subtitle">Sessions, messages, and recent activity in one place.</p>
              </div>
              <div className="surface-actions">
                <button type="button" className="btn-sm" onClick={refreshAll}>Refresh</button>
                  <button type="button" className="btn-sm" onClick={() => changePanel("brain")}>Brain View</button>
              </div>
            </div>
            <div className="surface-grid agents-grid">
              <div className="card agents-card-span-2">
                <div className="card-header">
                  <h2>Active Sessions</h2>
                  <span className="badge">{normalizedSessions.length}</span>
                </div>
                <ul className="item-list">
                  {normalizedSessions.length ? normalizedSessions.map((session) => <AgentItem key={session.sessionId || session.agent} session={session} />) : <EmptyItem text="No agents online" />}
                </ul>
              </div>

                <div className="card">
                  <div className="card-header">
                    <h2>Operator Inbox</h2>
                    <span className="badge">{messageEntries.length}</span>
                  </div>
                  <div className="surface-toolbar">
                    <OperatorSelector
                      value={selectedOperator}
                      knownAgents={knownAgents}
                      onChange={setSelectedOperator}
                    />
                    <div className="surface-actions">
                      <button type="button" className="btn-sm" onClick={() => refreshMessages().catch(reportSurfaceError)}>
                        Refresh
                      </button>
                    </div>
                  </div>
                  <ul className="item-list">
                    {!selectedOperator.trim() ? (
                      <EmptyItem text="Select an operator to view the inbox" />
                    ) : messageEntries.length ? (
                      messageEntries.map((entry) => <MessageItem key={entry.id} entry={entry} />)
                    ) : (
                      <EmptyItem text={`No inbox messages for ${selectedOperator.trim()}`} />
                    )}
                  </ul>
                </div>

              <div className="card">
                <div className="card-header">
                  <h2>Recent Activity</h2>
                  <span className="badge">{activityEntries.length}</span>
                </div>
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
                    <button type="button" className="btn-sm" onClick={() => refreshActivity().catch(reportSurfaceError)}>
                      Refresh
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
            </div>
          </section>
        ) : null}

        {panel === "work" ? (
          <section className="panel active">
            <div className="panel-header">
              <div>
                <h1>Work</h1>
                <p className="panel-subtitle">Queue, inbox, locks, and shared feed run through the same live operator surface.</p>
              </div>
              <div className="surface-actions">
                <button type="button" className="btn-sm" onClick={refreshAll}>Refresh</button>
                <button type="button" className="btn-sm" onClick={() => changePanel("agents")}>Agents</button>
              </div>
            </div>

            <div className="surface-toolbar work-operator-toolbar">
              <OperatorSelector
                value={selectedOperator}
                knownAgents={knownAgents}
                onChange={setSelectedOperator}
              />
              <div className="surface-actions">
                <span className="badge">{selectedOperator.trim() || "Unset"}</span>
                <span className="surface-inline-hint">Live actions use the selected operator label.</span>
              </div>
            </div>

            <div className="surface-stat-grid">
              <div className="surface-stat-card">
                <span className="surface-stat-label">Pending</span>
                <strong>{pendingTasks.length}</strong>
              </div>
              <div className="surface-stat-card">
                <span className="surface-stat-label">Claimed</span>
                <strong>{claimedTasks.length}</strong>
              </div>
              <div className="surface-stat-card">
                <span className="surface-stat-label">Completed</span>
                <strong>{completedTasks.length}</strong>
              </div>
              <div className="surface-stat-card">
                <span className="surface-stat-label">Locks</span>
                <strong>{locks.length}</strong>
              </div>
            </div>

            <div className="work-grid">
              <div className="task-columns work-task-columns">
                <div className="card">
                  <div className="card-header">
                    <h2>Pending</h2>
                    <span className="badge">{pendingTasks.length}</span>
                  </div>
                  <ul className="item-list">
                    {pendingTasks.length ? pendingTasks.map((task) => (
                      <TaskItem
                        key={task.taskId}
                        task={task}
                        selectedOperator={selectedOperator}
                        onClaim={handleTaskClaim}
                        busyActionKey={busyActionKey}
                      />
                    )) : <EmptyItem text="No pending tasks" />}
                  </ul>
                </div>

                <div className="card">
                  <div className="card-header">
                    <h2>In Progress</h2>
                    <span className="badge">{claimedTasks.length}</span>
                  </div>
                  <ul className="item-list">
                    {claimedTasks.length ? claimedTasks.map((task) => (
                      <TaskItem
                        key={task.taskId}
                        task={task}
                        selectedOperator={selectedOperator}
                        completionDraft={taskCompletionDrafts[task.taskId] || ""}
                        completionExpanded={completionTaskId === task.taskId}
                        onAbandon={handleTaskAbandon}
                        onComplete={handleTaskComplete}
                        onCompletionDraftChange={(taskId, value) => {
                          setTaskCompletionDrafts((current) => ({ ...current, [taskId]: value }));
                        }}
                        onToggleComplete={(taskId) => {
                          setCompletionTaskId((current) => (current === taskId ? "" : taskId));
                        }}
                        busyActionKey={busyActionKey}
                      />
                    )) : <EmptyItem text="Nothing in progress" />}
                  </ul>
                </div>

                <div className="card">
                  <div className="card-header">
                    <h2>Done</h2>
                    <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
                      <span className="badge">{completedTasks.length}</span>
                      {completedTasks.length > 0 ? (
                        <button
                          type="button"
                          className="btn-sm"
                          onClick={async () => {
                            try {
                              const results = await Promise.allSettled(
                                completedTasks.filter((task) => task?.taskId).map((task) => postApi("/tasks/delete", { taskId: task.taskId }))
                              );
                              const failed = results.filter((result) => result.status === "rejected");
                              if (failed.length) {
                                setFeedbackMessage(`${failed.length} task delete(s) failed: ${failed[0].reason}`);
                              }
                              await refreshAll();
                            } catch (error) {
                              reportSurfaceError(error);
                            }
                          }}
                        >
                          Clear
                        </button>
                      ) : null}
                    </div>
                  </div>
                  <ul className="item-list">
                    {completedTasks.length ? completedTasks.slice(0, 10).map((task) => (
                      <TaskItem
                        key={task.taskId}
                        task={task}
                        selectedOperator={selectedOperator}
                        onDelete={handleTaskDelete}
                        busyActionKey={busyActionKey}
                      />
                    )) : <EmptyItem text="No completed tasks" />}
                  </ul>
                </div>
              </div>

              <div className="work-side-stack">
                <div className="card">
                  <div className="card-header">
                    <h2>Operator Inbox</h2>
                    <span className="badge">{messageEntries.length}</span>
                  </div>
                  <div className="surface-toolbar">
                    <OperatorSelector
                      value={selectedOperator}
                      knownAgents={knownAgents}
                      onChange={setSelectedOperator}
                    />
                    <label className="feed-control">
                      <span>Recipient</span>
                      <input
                        type="text"
                        list="message-recipient-list"
                        placeholder="factory-droid"
                        value={messageTarget}
                        onChange={(event) => setMessageTarget(event.target.value)}
                      />
                      <datalist id="message-recipient-list">
                        {knownAgents
                          .filter((agent) => !sameAgent(agent, selectedOperator))
                          .map((agent) => (
                            <option key={agent} value={agent} />
                          ))}
                      </datalist>
                    </label>
                    <div className="surface-actions">
                      <button type="button" className="btn-sm" onClick={() => refreshMessages().catch(reportSurfaceError)}>
                        Refresh Inbox
                      </button>
                    </div>
                  </div>
                  <form className="surface-compose" onSubmit={handleSendMessage}>
                    <textarea
                      value={messageDraft}
                      onChange={(event) => setMessageDraft(event.target.value)}
                      placeholder={selectedOperator.trim() ? `Message from ${selectedOperator.trim()}` : "Select an operator to send messages"}
                      rows={3}
                    />
                    <div className="surface-actions">
                      <button type="submit" className="btn-sm btn-primary" disabled={busyActionKey === "message:send"}>
                        {busyActionKey === "message:send" ? "Sending..." : "Send Message"}
                      </button>
                    </div>
                  </form>
                  <ul className="item-list compact-list">
                    {!selectedOperator.trim() ? (
                      <EmptyItem text="Select an operator to view the inbox" />
                    ) : messageEntries.length ? (
                      messageEntries.map((entry) => <MessageItem key={entry.id} entry={entry} />)
                    ) : (
                      <EmptyItem text={`No inbox messages for ${selectedOperator.trim()}`} />
                    )}
                  </ul>
                </div>

                <div className="card">
                  <div className="card-header">
                    <h2>Locks</h2>
                    <span className="badge">{locks.length}</span>
                  </div>
                  <ul className="item-list">
                    {locks.length ? locks.map((lock) => (
                      <LockItem
                        key={lock.id || `${lock.path}:${lock.agent}`}
                        lock={lock}
                        selectedOperator={selectedOperator}
                        onUnlock={handleUnlock}
                        busyActionKey={busyActionKey}
                      />
                    )) : <EmptyItem text="No active locks" />}
                  </ul>
                </div>

                <div className="card">
                  <div className="card-header">
                    <h2>Shared Feed</h2>
                    <span className="badge">{feedEntries.length}</span>
                  </div>
                  <div className="feed-toolbar work-feed-toolbar">
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
                    <div className="surface-actions">
                      <button
                        type="button"
                        className="btn-sm"
                        disabled={busyActionKey === "feed:ack" || !selectedOperator.trim()}
                        onClick={() => handleFeedAck().catch(reportSurfaceError)}
                      >
                        {busyActionKey === "feed:ack" ? "Acking..." : "Acknowledge Visible"}
                      </button>
                      <button type="button" className="btn-sm" onClick={() => refreshFeed().catch(reportSurfaceError)}>
                        Refresh
                      </button>
                    </div>
                  </div>
                  <ul className="item-list">
                    {feedEntries.length ? feedEntries.map((entry) => <FeedItem key={entry.id} entry={entry} />) : <EmptyItem text="No feed entries" />}
                  </ul>
                </div>
              </div>
            </div>
          </section>
        ) : null}

        {panel === "memory" ? (
          <section className="panel active">
            <div className="panel-header">
              <div>
                <h1>Memory</h1>
                <p className="panel-subtitle">Search the brain, inspect recall health, and resolve conflicts without leaving the same tab.</p>
              </div>
              <div className="surface-actions">
                <button type="button" className="btn-sm" onClick={() => refreshConflicts().catch(reportSurfaceError)}>Refresh Conflicts</button>
                <button type="button" className="btn-sm" onClick={() => changePanel("analytics")}>Analytics</button>
              </div>
            </div>

            <div className="memory-layout">
              <div className="card full">
                <div className="card-header">
                  <h2>Memory Explorer</h2>
                  <span className="badge">{memoryResults.length}</span>
                </div>
                <form className="memory-search" onSubmit={handleMemorySearch}>
                  <input
                    type="text"
                    className="memory-input"
                    placeholder="Search the brain... (uses cortex_peek)"
                    value={memoryQuery}
                    onChange={(event) => setMemoryQuery(event.target.value)}
                  />
                  <button type="submit" className="btn-sm btn-primary" disabled={memorySearching}>
                    {memorySearching ? "Searching..." : "Peek"}
                  </button>
                </form>
                {memoryResults.length > 0 ? (
                  <div className="memory-stats">
                    <span className="badge">{memoryResults.length} matches</span>
                    <span className="muted-inline">via cortex_peek -- click to expand full recall</span>
                  </div>
                ) : null}
                <ul className="item-list">
                  {memoryResults.length ? memoryResults.map((match, index) => (
                    <li key={`${match.source}-${index}`} className="memory-item" onClick={() => !match.expanded && handleMemoryExpand(match.source)}>
                      <div className="memory-header">
                        <span className="memory-method">{match.method}</span>
                        <span className="memory-relevance">{(match.relevance * 100).toFixed(0)}%</span>
                      </div>
                      <div className="memory-source">{match.source}</div>
                      {match.expanded && match.excerpt ? (
                        <div className="memory-excerpt">{match.excerpt}</div>
                      ) : null}
                      {!match.expanded ? <div className="memory-expand-hint">Click to expand</div> : null}
                    </li>
                  )) : memoryQuery ? <EmptyItem text="No matches -- try different keywords" /> : <EmptyItem text="Search to explore Cortex memories" />}
                </ul>
              </div>

              <div className="memory-side-stack">
                <div className="card">
                  <div className="card-header">
                    <h2>Memory Health</h2>
                    <span className="badge">{latestRecallHitRate || 0}%</span>
                  </div>
                  <div className="overview-status-list">
                    <div className="overview-status-row">
                      <span>Memories</span>
                      <strong>{stats.memories}</strong>
                    </div>
                    <div className="overview-status-row">
                      <span>Decisions</span>
                      <strong>{stats.decisions}</strong>
                    </div>
                    <div className="overview-status-row">
                      <span>7-day recall avg</span>
                      <strong>{recallWindowAverage || 0}%</strong>
                    </div>
                    <div className="overview-status-row">
                      <span>Open conflicts</span>
                      <strong>{conflictPairs.length}</strong>
                    </div>
                  </div>
                </div>

                <div className="card">
                  <div className="card-header">
                    <h2>Conflict Radar</h2>
                    <span className="badge">{conflictPairs.length}</span>
                  </div>
                  <ul className="item-list compact-list">
                    {conflictPairs.length ? conflictPairs.slice(0, 4).map((pair) => (
                      <li key={`${pair.left.id}-${pair.right.id}`}>
                        <div className="item-meta">
                          <span className="item-name">#{pair.left.id} vs #{pair.right.id}</span>
                          <span className="muted-inline">{Math.round(((pair.left.confidence || 0.8) + (pair.right.confidence || 0.8)) * 50)}%</span>
                        </div>
                        <div className="item-detail">
                          {pair.left.source_agent || "unknown"} / {pair.right.source_agent || "unknown"}
                        </div>
                      </li>
                    )) : <EmptyItem text="No active conflicts" />}
                  </ul>
                </div>
              </div>
            </div>

            <div className="memory-conflicts-section">
              <div className="panel-header panel-header-inline">
                <h2>Conflict Resolution</h2>
                <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
                  <span className="badge">{conflictPairs.length} dispute{conflictPairs.length !== 1 ? "s" : ""}</span>
                  <button type="button" className="btn-sm" onClick={() => refreshConflicts().catch(reportSurfaceError)}>Refresh</button>
                </div>
              </div>
              {conflictPairs.length === 0 ? (
                <div className="card full">
                  <ul className="item-list">
                    <EmptyItem text="No active conflicts -- all decisions are in harmony" />
                  </ul>
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
                        {pair.left.context ? <p className="conflict-context">{pair.left.context}</p> : null}
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
                        {pair.right.context ? <p className="conflict-context">{pair.right.context}</p> : null}
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
            </div>
          </section>
        ) : null}

        {panel === "__legacy_agents" ? (
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

        {panel === "__legacy_tasks" ? (
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

        {panel === "__legacy_feed" ? (
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
                  <button type="button" className="btn-sm" onClick={() => refreshFeed().catch(reportSurfaceError)}>
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

        {panel === "__legacy_messages" ? (
          <section className="panel active">
            <div className="panel-header">
              <h1>Messages</h1>
            </div>
            <div className="card full">
              <div className="surface-toolbar">
                <label className="feed-control">
                    <span>Recipient</span>
                    <input
                      type="text"
                      list="message-recipient-list"
                      placeholder="factory-droid"
                      value={messageTarget}
                      onChange={(event) => setMessageTarget(event.target.value)}
                    />
                    <datalist id="message-recipient-list">
                      {knownAgents.map((agent) => (
                        <option key={agent} value={agent} />
                      ))}
                    </datalist>
                  </label>
                <div className="surface-actions">
                  <span className="badge">{messageEntries.length}</span>
                  <button type="button" className="btn-sm" onClick={() => refreshMessages().catch(reportSurfaceError)}>
                    Refresh Messages
                  </button>
                </div>
              </div>
              <ul className="item-list">
                {!selectedOperator.trim() ? (
                  <EmptyItem text="Select an operator to view the inbox" />
                ) : messageEntries.length ? (
                  messageEntries.map((entry) => <MessageItem key={entry.id} entry={entry} />)
                ) : (
                  <EmptyItem text={`No inbox messages for ${selectedOperator.trim()}`} />
                )}
              </ul>
            </div>
          </section>
        ) : null}

        {panel === "__legacy_activity" ? (
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
                  <button type="button" className="btn-sm" onClick={() => refreshActivity().catch(reportSurfaceError)}>
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

        {panel === "__legacy_memory" ? (
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

        {panel === "analytics" || hasVisitedAnalytics ? (
          <section
            className={`panel analytics-panel ${panel === "analytics" ? "active" : "panel-hidden"}`}
            aria-hidden={panel === "analytics" ? undefined : true}
          >
            <div className="analytics-panel-header">
              <div className="analytics-header-copy">
                <span className="analytics-kicker">Cortex / Analytics</span>
                <h1>Compounding Memory Economics</h1>
                <p>
                  Track how Cortex turns raw recall pressure into a smaller boot prompt, compounding token savings over time instead of replaying the whole brain on every boot.
                </p>
              </div>
              <div className="analytics-toolbar">
                <span className="panel-subtitle">Token savings and brain health</span>
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
                <button type="button" className="btn-sm" onClick={() => refreshSavings().catch(reportSurfaceError)}>
                  Refresh
                </button>
              </div>
            </div>
            {!analyticsReady ? (
              <div className="card full analytics-loading-card">
                <EmptyItem text="Preparing analytics surface..." />
              </div>
            ) : savings ? (
              <>
                <div className="analytics-metrics-grid">
                  <div className="metric metric-featured" data-accent="cyan">
                    <span className="metric-kicker">Compounding return</span>
                    <span className="metric-value"><AnimatedNumber value={savings.summary?.totalSaved || 0} duration={1000} /></span>
                    <span className="metric-label">Boot Tokens Saved</span>
                    <span className="metric-footnote">
                      {bootSavingsMomentum === null
                        ? "Collecting enough history for a momentum read."
                        : `${bootSavingsMomentum >= 0 ? "+" : ""}${bootSavingsMomentum}% vs previous 4-day window`}
                    </span>
                    <span className="metric-icon"><AppIcon name="savings" /></span>
                  </div>
                  <div className="metric" data-accent="green">
                    <span className="metric-kicker">Efficiency</span>
                    <span className="metric-value"><AnimatedNumber value={savings.summary?.avgPercent || 0} />%</span>
                    <span className="metric-label">Avg Compression</span>
                    <span className="metric-footnote">
                      Avg saved per boot {formatCompactNumber(Number(savings.summary?.avgSavedPerBoot || 0))} tokens
                    </span>
                    <span className="metric-icon"><AppIcon name="efficiency" /></span>
                  </div>
                  <div className="metric" data-accent="blue">
                    <span className="metric-kicker">Throughput</span>
                    <span className="metric-value"><AnimatedNumber value={savings.summary?.totalBoots || 0} /></span>
                    <span className="metric-label">Boot Compilations</span>
                    <span className="metric-footnote">
                      Avg boot prompt {formatCompactNumber(Number(savings.summary?.avgServedPerBoot || 0))} tokens served
                    </span>
                    <span className="metric-icon"><AppIcon name="refresh" /></span>
                  </div>
                  <div className="metric" data-accent="purple">
                    <span className="metric-kicker">Compiled context</span>
                    <span className="metric-value"><AnimatedNumber value={savings.summary?.totalServed || 0} duration={1000} /></span>
                    <span className="metric-label">Boot Prompt Tokens</span>
                    <span className="metric-footnote">
                      Baseline replay pressure {formatCompactNumber(Number(savings.summary?.totalBaseline || 0))} tokens
                    </span>
                    <span className="metric-icon"><AppIcon name="outbound" /></span>
                  </div>
                  <div className="metric" data-accent="green">
                    <span className="metric-kicker">Economic value</span>
                    <span className="metric-value">{formatCurrency(((savings.summary?.totalSaved || 0) * SAVINGS_USD_PER_MILLION) / 1000000)}</span>
                    <span className="metric-label">Est. {currency} Saved</span>
                    <span className="metric-footnote">
                      Latest recall hit rate {latestRecallHitRate || 0}% with local-first memory
                    </span>
                    <span className="metric-icon">$</span>
                  </div>
                </div>

                {analyticsMode === "aggregate" ? (
                  <>
                    <div className="analytics-explainer analytics-explainer-rich">
                      <div className="analytics-explainer-title">How to read this</div>
                      <p>
                        Cortex compiles a budgeted boot prompt instead of replaying raw memory. <code>baseline</code> is estimated raw context load, <code>served</code> is the compiled prompt, and <code>saved</code> is the difference. Aggregate mode shows the compounding system view. By Operation isolates where those savings come from.
                      </p>
                      <div className="analytics-stat-strip">
                        <div className="analytics-stat-chip">
                          <span className="analytics-stat-chip-label">Avg raw per boot</span>
                          <strong>{formatCompactNumber(Number(savings.summary?.avgBaselinePerBoot || 0))}t</strong>
                        </div>
                        <div className="analytics-stat-chip">
                          <span className="analytics-stat-chip-label">Avg served per boot</span>
                          <strong>{formatCompactNumber(Number(savings.summary?.avgServedPerBoot || 0))}t</strong>
                        </div>
                        <div className="analytics-stat-chip">
                          <span className="analytics-stat-chip-label">Median 30d gain</span>
                          <strong>
                            {monteCarloProjection
                              ? `${formatSignedCompactNumber(Number(monteCarloProjection.summary?.p50Gain || 0))}t`
                              : "Pending"}
                          </strong>
                        </div>
                      </div>
                    </div>

                    <div className="analytics-stage-grid">
                      <div className="card analytics-hero-card analytics-card-span-2">
                        <div className="analytics-card-header-tight">
                          <div>
                            <span className="analytics-card-kicker">Projection</span>
                            <h2>Monte Carlo Savings Horizon</h2>
                          </div>
                          <span className="badge">
                            {monteCarloProjection ? `${monteCarloProjection.simulationCount} sims / 30 days` : "Waiting for more history"}
                          </span>
                        </div>
                        <p className="chart-summary">
                          A deterministic Monte Carlo projection built from recent daily savings. It estimates the likely additional savings band over the next 30 days so the trajectory reads as future lift, not replayed lifetime totals.
                        </p>
                        <MonteCarloProjectionChart projection={monteCarloProjection} />
                        {monteCarloProjection ? (
                          <div className="analytics-stat-strip analytics-stat-strip-tight">
                            <div className="analytics-stat-chip">
                              <span className="analytics-stat-chip-label">p10</span>
                              <strong>{formatSignedCompactNumber(Number(monteCarloProjection.summary?.p10Gain || 0))}t</strong>
                            </div>
                            <div className="analytics-stat-chip">
                              <span className="analytics-stat-chip-label">p50</span>
                              <strong>{formatSignedCompactNumber(Number(monteCarloProjection.summary?.p50Gain || 0))}t</strong>
                            </div>
                            <div className="analytics-stat-chip">
                              <span className="analytics-stat-chip-label">p90</span>
                              <strong>{formatSignedCompactNumber(Number(monteCarloProjection.summary?.p90Gain || 0))}t</strong>
                            </div>
                            <div className="analytics-stat-chip">
                              <span className="analytics-stat-chip-label">Current run-rate</span>
                              <strong>{formatCompactNumber(Number(monteCarloProjection.summary?.avgDaily || 0))}t/day</strong>
                            </div>
                          </div>
                        ) : null}
                      </div>

                      <div className="card analytics-chart-card analytics-health-card">
                        <div className="analytics-card-header-tight">
                          <div>
                            <span className="analytics-card-kicker">Live health</span>
                            <h2>Recall Quality</h2>
                          </div>
                          <span className="badge">{latestRecallHitRate || 0}%</span>
                        </div>
                        <p className="chart-summary">
                          Recall quality is tracked as a health box because the current signal is usually flat. What matters here is whether it is stable, drifting, or falling behind token savings.
                        </p>
                        <div className="analytics-stat-strip analytics-stat-strip-tight">
                          <div className="analytics-stat-chip">
                            <span className="analytics-stat-chip-label">Latest</span>
                            <strong>{latestRecallHitRate || 0}%</strong>
                          </div>
                          <div className="analytics-stat-chip">
                            <span className="analytics-stat-chip-label">7-day avg</span>
                            <strong>{recallWindowAverage || 0}%</strong>
                          </div>
                          <div className="analytics-stat-chip">
                            <span className="analytics-stat-chip-label">Spread</span>
                            <strong>{recallWindowSpread || 0} pts</strong>
                          </div>
                          <div className="analytics-stat-chip">
                            <span className="analytics-stat-chip-label">Assessment</span>
                            <strong>{recallWindowSpread <= 2 ? "Stable" : latestRecallHitRate >= 90 ? "Strong" : "Watch"}</strong>
                          </div>
                        </div>
                        {recallHeadlineUsesFallback ? (
                          <p className="analytics-inline-note">
                            Headline is pinned to the last full sample day until live recall reaches {RECALL_HEADLINE_MIN_QUERIES} queries.
                            Today&apos;s live sample is {Math.round(Number(latestRecallPoint?.hitRatePct || 0))}% on {latestRecallSampleSize} queries.
                          </p>
                        ) : null}
                        <div className="chart-legend analytics-quality-strip">
                          {recentRecallWindow.length ? recentRecallWindow.map((point) => (
                            <span key={point.date} className="chart-day">
                              <span className="chart-day-label">{(point.date || "").slice(5)}</span>
                              <span className="chart-day-value">{Math.round(Number(point.hitRatePct || 0))}%</span>
                            </span>
                          )) : (
                            <span className="sparkline-empty">Recall metrics will appear after recent boots.</span>
                          )}
                        </div>
                      </div>
                    </div>

                    <div className="overview-grid analytics-secondary-grid">
                      <div className="card analytics-chart-card">
                        <div className="analytics-card-header-tight">
                          <div>
                            <span className="analytics-card-kicker">Short-term movement</span>
                            <h2>Daily Token Savings</h2>
                          </div>
                          <span className="badge">{dailySeries.length} days</span>
                        </div>
                        <Sparkline
                          data={(savings.daily || []).map(d => d.saved)}
                          width={520}
                          height={120}
                          className="sparkline-tall"
                        />
                        <div className="chart-legend">
                          {(savings.daily || []).slice(-7).map(d => (
                            <span key={d.date} className="chart-day">
                              <span className="chart-day-label">{d.date.slice(5)}</span>
                              <span className="chart-day-value">{formatCompactNumber(Number(d.saved || 0))}</span>
                            </span>
                          ))}
                        </div>
                      </div>

                      <div className="card analytics-chart-card">
                        <div className="analytics-card-header-tight">
                          <div>
                            <span className="analytics-card-kicker">System load</span>
                            <h2>Boots Per Day</h2>
                          </div>
                          <span className="badge">{formatCompactNumber(Number(savings.summary?.totalBoots || 0))} total</span>
                        </div>
                        <Sparkline
                          data={(savings.daily || []).map(d => d.boots)}
                          width={520}
                          height={120}
                          color="var(--agent-claude)"
                          className="sparkline-tall"
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
                      <div className="card analytics-chart-card">
                        <div className="analytics-card-header-tight">
                          <div>
                            <span className="analytics-card-kicker">Long-term impact</span>
                            <h2>Cumulative Savings</h2>
                          </div>
                          <span className="badge">{formatCompactNumber(Number(savings.summary?.totalSaved || 0))}t</span>
                        </div>
                        <Sparkline
                          data={cumulativeSeries.map((point) => Number(point.savedTotal || 0))}
                          width={520}
                          height={120}
                          color="var(--green)"
                          className="sparkline-tall"
                        />
                        <div className="chart-legend">
                          {cumulativeSeries.slice(-7).map((point) => (
                            <span key={point.date || point.timestamp} className="chart-day">
                              <span className="chart-day-label">{(point.date || "").slice(5)}</span>
                              <span className="chart-day-value">{formatCompactNumber(Number(point.savedTotal || 0))}</span>
                            </span>
                          ))}
                        </div>
                      </div>
                    </div>

                    {activityHeatmap.length > 0 && (
                      <div className="card analytics-heatmap-card">
                        <div className="analytics-card-header-tight">
                          <div>
                            <span className="analytics-card-kicker">Behavioral map</span>
                            <h2>Agent Activity Heatmap</h2>
                          </div>
                          <div className="heatmap-legend-scale" aria-hidden="true">
                            <span>Low</span>
                            <span className="heatmap-legend-bar" />
                            <span>High</span>
                          </div>
                        </div>
                        <div className="activity-heatmap">
                          {["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].map((day) => (
                            <div key={day} className="activity-heatmap-row">
                              <span className="activity-heatmap-day">{day}</span>
                              <div className="activity-heatmap-cells">
                                {Array.from({ length: 24 }).map((_, hour) => {
                                  const count = activityHeatmapLookup.get(`${day}:${hour}`) || 0;
                                  const alpha = count > 0 ? clampNumber(count / activityHeatmapMax, 0.12, 1) : 0.04;
                                  return (
                                    <span
                                      key={`${day}-${hour}`}
                                      className="activity-heatmap-cell"
                                      title={`${day} ${hour.toString().padStart(2, "0")}:00 · ${count} events`}
                                      style={{ background: `linear-gradient(180deg, rgba(67, 234, 255, ${alpha}), rgba(58, 109, 255, ${alpha * 0.72}))` }}
                                    />
                                  );
                                })}
                              </div>
                            </div>
                          ))}
                        </div>
                      </div>
                    )}

                    <div className="analytics-lists-grid">
                      <div className="card analytics-list-card">
                        <div className="analytics-card-header-tight">
                          <div>
                            <span className="analytics-card-kicker">Who is creating lift</span>
                            <h2>Boot Savings by Agent</h2>
                          </div>
                          <span className="badge">{savings.byAgent?.length || 0}</span>
                        </div>
                        <ul className="item-list analytics-list">
                          {savings.byAgent?.length ? savings.byAgent
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
                            )) : <EmptyItem text="No per-agent savings data yet" />}
                        </ul>
                      </div>

                      <div className="card analytics-list-card">
                        <div className="analytics-card-header-tight">
                          <div>
                            <span className="analytics-card-kicker">Latest savings events</span>
                            <h2>Recent Boot Savings</h2>
                          </div>
                          <span className="badge">{savings.recent?.length || 0}</span>
                        </div>
                        <ul className="item-list analytics-list">
                          {savings.recent?.length ? savings.recent.slice(-10).reverse().map((s, i) => (
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
                          )) : <EmptyItem text="No recent boot savings events yet" />}
                        </ul>
                      </div>
                    </div>
                  </>
                ) : (
                  <>
                    <div className="analytics-explainer analytics-explainer-rich">
                      <div className="analytics-explainer-title">Operation view</div>
                      <p>Operation view breaks savings into recall, store, boot compression, and tool-call categories using local events. Use it to see where the system is earning margin, not just how much it saved overall.</p>
                    </div>
                    <div className="card analytics-operations-card">
                      <div className="analytics-card-header-tight">
                        <div>
                          <span className="analytics-card-kicker">Attribution</span>
                          <h2>Savings by Operation</h2>
                        </div>
                        <span className="badge">{operationRows.length} categories</span>
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

        {panel === "__legacy_locks" ? (
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

        {panel === "brain" || hasVisitedBrain ? (
          <section
            className={`panel brain-panel ${panel === "brain" ? "active" : "panel-hidden"}`}
            aria-hidden={panel === "brain" ? undefined : true}
          >
            <BrainErrorBoundary>
              <BrainVisualizer
                api={api}
                cortexBase={cortexBase}
                authToken={tokenRef.current}
                active={panel === "brain"}
              />
            </BrainErrorBoundary>
          </section>
        ) : null}

        {panel === "conflicts" ? (
          <section className="panel active">
            <div className="panel-header">
              <h1>Conflict Resolution</h1>
              <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
                <span className="badge">{conflictPairs.length} dispute{conflictPairs.length !== 1 ? "s" : ""}</span>
                <button type="button" className="btn-sm" onClick={() => refreshConflicts().catch(reportSurfaceError)}>Refresh</button>
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
              <div>
                <h1>About</h1>
                <p className="panel-subtitle">Shipping surface, runtime contract, and contributor credits for Cortex Control Center.</p>
              </div>
            </div>
            <div className="card full">
              <div style={{ padding: "2rem", maxWidth: 760 }}>
                <div style={{ display: "flex", alignItems: "center", gap: "1rem", marginBottom: "1.5rem" }}>
                  <img
                    src={`${import.meta.env.BASE_URL}icons/icon.png`}
                    alt="Cortex"
                    style={{ width: 64, height: 64, borderRadius: "50%", objectFit: "cover", flexShrink: 0 }}
                    onError={(event) => { event.currentTarget.style.display = "none"; event.currentTarget.nextSibling.style.display = "flex"; }}
                  />
                  <div style={{
                    width: 64, height: 64, borderRadius: "50%",
                    background: "linear-gradient(135deg, var(--cyan), var(--blue))",
                    display: "none", alignItems: "center", justifyContent: "center",
                    fontSize: "2rem", flexShrink: 0,
                  }}>CC</div>
                  <div>
                    <h2 style={{ margin: 0, fontSize: "1.5rem" }}>Cortex Control Center</h2>
          <p style={{ margin: "0.25rem 0 0", color: "var(--text-3)" }}>Created by @AdityaVG13 -- Version {CONTROL_CENTER_VERSION}</p>
                  </div>
                </div>

                <p style={{ color: "var(--text-2)", lineHeight: 1.7, marginBottom: "1.5rem" }}>
                  A desktop command surface for the Cortex daemon. The app is built to be production-safe for shipped users first:
                  auth-aware startup, daemon lifecycle control, live telemetry, and a brain view that can double as a showpiece.
                </p>

                <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "0.75rem", marginBottom: "2rem" }}>
                  {[
                    ["Daemon", "Rust + Axum"],
                    ["Desktop shell", "Tauri + React"],
                    ["Embeddings", "ONNX (all-MiniLM-L6-v2)"],
                    ["Storage", "SQLite (WAL)"],
                    ["Transport", "HTTP + MCP stdio"],
                    ["Port", "7437"],
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
                  <h3 style={{ fontSize: "0.875rem", textTransform: "uppercase", letterSpacing: "0.08em", color: "var(--text-3)", marginBottom: "0.75rem" }}>App Lifecycle</h3>
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
                        <td>Runs Stop then Start with timeout handling so the UI can recover when shutdown hangs.</td>
                      </tr>
                      <tr>
                        <td>Close Window</td>
                        <td>Minimizes to tray by default so Cortex keeps running in the background.</td>
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
                      { handle: "Codex", role: "Desktop rewrite, auth hardening, analytics and brain UX" },
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
              </div>
            </div>
          </section>
        ) : null}

        {panel === "__legacy_about" ? (
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
                  }}><AppIcon name="overview" size={28} /></div>
                  <div>
                    <h2 style={{ margin: 0, fontSize: "1.5rem" }}>Cortex Control Center</h2>
                    <p style={{ margin: "0.25rem 0 0", color: "var(--text-3)" }}>Created by @AdityaVG13 -- Version {CONTROL_CENTER_VERSION}</p>
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
        </div>
      </main>
    </div>
  );
}
