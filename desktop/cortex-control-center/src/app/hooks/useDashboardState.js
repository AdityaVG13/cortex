import { startTransition, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { checkForUpdates, installUpdate } from "../../updater.js";
import { MOTION_MS } from "../../design/motion.js";
import {
  createApi,
  createPostApi,
  isAuthFailure,
  settledCollectErrors,
  summarizeDashboardErrors,
} from "../../api-client.js";
import {
  CURRENCY_OPTIONS,
  USD_TO_CURRENCY_RATE,
  SAVINGS_OPERATION_LABELS,
  timeAgo,
} from "../../constants.js";
import {
  buildKnownAgents,
  filterFeedEntries,
  isTransportSession,
  nextFeedAckId,
  normalizeTask,
  resolveAgentName,
  sameAgent,
} from "../../live-surface.js";
import {
  buildFirstRunReadiness,
  computeStartupRetryStep,
  daemonStatusPill,
  daemonSystemStatus,
  daemonUtilityPill,
  isDaemonStartingState,
  shouldContinueStartupRecovery,
  isTransientDaemonFeedback,
} from "../../daemon-startup.js";
import { buildMonteCarloProjection } from "../../analytics-projection.js";
import { summarizeBootThroughput } from "../../analytics-metrics.js";
import { formatCompactNumber, formatSignedCompactNumber } from "../../number-format.js";
import { handleKeyboardActivation, shouldIgnoreGlobalShortcut, trapFocusInContainer } from "../../keyboard-access.js";
import {
  BUDGET_ENDPOINT_DEFINITIONS,
  createBudgetDraftFromStatus,
  readControlCenterSettings,
  resolveEffectiveReducedMotion,
  serializeBudgetDraftForSave,
  summarizeBudgetStatus,
  validateBudgetDraft,
  writeControlCenterSettings,
} from "../../settings/settings-state.js";
import {
  ANALYTICS_METRIC_LEGEND,
  ANALYTICS_REFRESH_MS,
  CONTROL_CENTER_VERSION,
  CORE_REFRESH_MIN_INTERVAL_MS,
  CORTEX_BASE_STORAGE_KEY,
  CORTEX_OPERATOR_STORAGE_KEY,
  CORTEX_PANEL_STORAGE_KEY,
  DAEMON_START_POLL_INTERVAL_MS,
  DAEMON_START_STILL_STARTING_GRACE_MS,
  DAEMON_START_WAIT_TIMEOUT_MS,
  DAEMON_STOP_HANG_TIMEOUT_MS,
  DAEMON_STOP_WAIT_TIMEOUT_MS,
  DEFAULT_CORTEX_BASE,
  DEV_RESTART_VERIFY_ENABLED,
  DEV_RESTART_VERIFY_TIMEOUT_MS,
  EMPTY_DAEMON,
  EMPTY_HEALTH_META,
  FALLBACK_REFRESH_MS,
  MISSION_METRIC_LEGEND,
  PANEL_SEQUENCE,
  PANEL_SEQUENCE_KEYS,
  PANEL_SEQUENCE_LABEL,
  RECALL_HEADLINE_MIN_QUERIES,
  SAVINGS_HISTORY_DAYS,
  SAVINGS_USD_PER_MILLION,
  SECONDARY_REFRESH_MIN_INTERVAL_MS,
  SIDEBAR_COLLAPSE_BREAKPOINT_PX,
  SSE_RECONNECT_BASE_MS,
  SSE_RECONNECT_MAX_MS,
  SSE_REFRESH_THROTTLE_MS,
  panelIndex,
} from "../constants.js";
import {
  readBrowserBootstrap,
  readLocalStorageValue,
  readPersistedBrowserAuthToken,
  readTauriInvoke,
  persistBrowserAuthToken,
} from "../browser-bootstrap.js";
import { normalizeCurrencyCode, formatDaemonEndpoint, getOsReducedMotionPreference, priorityRank } from "../utils/format.js";
import {
  isRouteMissingError,
  normalizeConflictPairsPayload,
} from "../normalize/conflicts.js";
import {
  normalizePermissionPayload,
} from "../normalize/permissions.js";
import {
  normalizeSession,
  sessionMatchesAgent,
} from "../normalize/sessions.js";
import {
  extractMcpToolError,
  isDaemonOfflineErrorMessage,
  isDaemonSuppressibleErrorMessage,
  isDaemonTimeoutErrorMessage,
  isReadyReadinessPayload,
  isReachableHealthPayload,
  parseMcpToolResult,
  setElementInert,
} from "../utils/daemon.js";

export function useDashboardState() {
  const browserBootstrap = useMemo(() => readBrowserBootstrap(), []);
  const isTauriRuntime = typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);
  const [panel, setPanel] = useState(() => browserBootstrap.panel || "overview");
  const [brainPanelMounted, setBrainPanelMounted] = useState(() => (browserBootstrap.panel || "overview") === "brain");
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
  const [daemonTimeoutStaleSummary, setDaemonTimeoutStaleSummary] = useState("");
  const [conflictPairs, setConflictPairs] = useState([]);
  const [resolveDrafts, setResolveDrafts] = useState({});
  const [conflictLoading, setConflictLoading] = useState(false);
  const [permissionGrants, setPermissionGrants] = useState([]);
  const [permissionLoading, setPermissionLoading] = useState(false);
  const [permissionAccessDenied, setPermissionAccessDenied] = useState(false);
  const [permissionsEndpointAvailable, setPermissionsEndpointAvailable] = useState(true);
  const [permissionDraft, setPermissionDraft] = useState({
    client: "",
    permission: "read",
    scope: "*",
  });
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
  const [showMissionMetricLegend, setShowMissionMetricLegend] = useState(false);
  const [showMissionCompactUnits, setShowMissionCompactUnits] = useState(true);
  const [hasVisitedAnalytics, setHasVisitedAnalytics] = useState(() => browserBootstrap.panel === "analytics");
  const [analyticsReady, setAnalyticsReady] = useState(() => browserBootstrap.panel === "analytics");
  const [startupCoreReadyState, setStartupCoreReadyState] = useState(false);
  const [isSettingUpEditors, setIsSettingUpEditors] = useState(false);
  const [controlSettings, setControlSettings] = useState(() => readControlCenterSettings());
  const [budgetConfigStatus, setBudgetConfigStatus] = useState(null);
  const [budgetDraft, setBudgetDraft] = useState(() => createBudgetDraftFromStatus(null));
  const [budgetDraftDirty, setBudgetDraftDirty] = useState(false);
  const [budgetConfigBusy, setBudgetConfigBusy] = useState(false);
  const [budgetConfigMessage, setBudgetConfigMessage] = useState("");
  const [ipcAvailable, setIpcAvailable] = useState(false);
  const [osReducedMotion, setOsReducedMotion] = useState(() => getOsReducedMotionPreference());
  const [currency, setCurrency] = useState(() => normalizeCurrencyCode(readLocalStorageValue("cortex_currency", "USD")));
  const [analyticsMode, setAnalyticsMode] = useState(() => {
    const stored = readLocalStorageValue("cortex_analytics_mode", "aggregate");
    return stored === "operations" ? "operations" : "aggregate";
  });
  const effectiveReducedMotion = useMemo(
    () => resolveEffectiveReducedMotion(controlSettings.reducedMotion, osReducedMotion),
    [controlSettings.reducedMotion, osReducedMotion],
  );

  const invokeRef = useRef(null);
  const tokenRef = useRef(browserBootstrap.authToken || "");
  const refreshAllRef = useRef(async () => {});
  const refreshAllInFlightRef = useRef(null);
  const refreshAllQueuedRef = useRef(false);
  const daemonTransitionRef = useRef(false);
  const recoveryRetryTimerRef = useRef(null);
  const startupRetryStateRef = useRef({ startedAtMs: 0, attempts: 0 });
  const startupCoreReadyRef = useRef(false);
  const lastCoreRefreshAtRef = useRef(0);
  const lastSecondaryRefreshAtRef = useRef(0);
  const startupSecondaryRefreshInFlightRef = useRef(false);
  const skipInitialFeedRefreshRef = useRef(true);
  const skipInitialMessagesRefreshRef = useRef(true);
  const skipInitialActivityRefreshRef = useRef(true);
  const connectionDialogRef = useRef(null);
  const connectionDialogTriggerRef = useRef(null);
  const editorSetupDialogRef = useRef(null);
  const editorSetupTriggerRef = useRef(null);
  const topbarRef = useRef(null);
  const analyticsPanelRef = useRef(null);
  const brainPanelRef = useRef(null);
  const analyticsTabRefs = useRef({});
  const sessionsRef = useRef([]);
  const daemonStateRef = useRef(EMPTY_DAEMON);
  const streamConnectedAtRef = useRef(0);
  const streamDisconnectedAtRef = useRef(0);
  const streamSessionEventCountRef = useRef(0);
  const devVerificationStartedRef = useRef(false);
  const permissionsEndpointAvailableRef = useRef(true);
  const browserHealthProbeRef = useRef(null);
  const connectionDialogAutoPromptSuppressedRef = useRef(false);
  const budgetConfigLoadAttemptedRef = useRef(false);

  const restoreFocusToTrigger = useCallback((triggerRef) => {
    if (typeof window === "undefined") return;
    window.requestAnimationFrame(() => {
      const target = triggerRef.current;
      triggerRef.current = null;
      if (target && typeof target.focus === "function" && document.contains(target)) {
        target.focus();
      }
    });
  }, []);

  const openConnectionDialog = useCallback((event) => {
    connectionDialogTriggerRef.current = event?.currentTarget || document.activeElement;
    connectionDialogAutoPromptSuppressedRef.current = false;
    setShowConnectionDialog(true);
  }, []);

  const dismissConnectionDialog = useCallback(() => {
    connectionDialogAutoPromptSuppressedRef.current = true;
    setShowConnectionDialog(false);
    restoreFocusToTrigger(connectionDialogTriggerRef);
  }, [restoreFocusToTrigger]);

  const closeConnectionDialog = useCallback(() => {
    connectionDialogAutoPromptSuppressedRef.current = false;
    setShowConnectionDialog(false);
    restoreFocusToTrigger(connectionDialogTriggerRef);
  }, [restoreFocusToTrigger]);

  const closeEditorSetupWizard = useCallback(() => {
    setShowEditorSetupWizard(false);
    restoreFocusToTrigger(editorSetupTriggerRef);
  }, [restoreFocusToTrigger]);

  const updateControlSetting = useCallback((key, value) => {
    setControlSettings((current) => ({ ...current, [key]: value }));
  }, []);

  const changePanel = useCallback((nextPanel) => {
    if (!PANEL_SEQUENCE_KEYS.has(nextPanel) || nextPanel === panel) {
      return;
    }

    if (nextPanel === "brain") {
      setBrainPanelMounted(true);
    }

    const currentIndex = panelIndex(panel);
    const nextIndex = panelIndex(nextPanel);
    setPanelMotionDirection(
      currentIndex >= 0 && nextIndex >= 0 && nextIndex < currentIndex ? "backward" : "forward"
    );
    setPanel(nextPanel);
  }, [panel]);

  const normalizedSessions = useMemo(() => {
    if (!Array.isArray(sessions)) return [];
    const sorted = sessions
      .map((session, index) => normalizeSession(session, index))
      .sort((a, b) => b.lastHeartbeatMs - a.lastHeartbeatMs);

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

  useEffect(() => {
    sessionsRef.current = normalizedSessions;
  }, [normalizedSessions]);

  useEffect(() => {
    daemonStateRef.current = daemonState;
  }, [daemonState]);

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
              args: ["mcp", "--agent", "codex"],
              env: {
                CORTEX_APP_REQUIRED: "1",
                CORTEX_DAEMON_OWNER_LOCAL_SPAWN: "0",
                CORTEX_APP_CLIENT: "codex",
              },
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

  const safeCurrency = normalizeCurrencyCode(currency);
  const currencyRate = USD_TO_CURRENCY_RATE[safeCurrency] ?? USD_TO_CURRENCY_RATE.USD;
  const activeBudgetStatus = budgetConfigStatus || healthMeta.budgets;
  const budgetSummary = useMemo(
    () => summarizeBudgetStatus(activeBudgetStatus),
    [activeBudgetStatus],
  );
  const budgetDraftError = useMemo(
    () => validateBudgetDraft(budgetDraft),
    [budgetDraft],
  );
  const budgetDraftEndpoints = budgetDraft?.endpoints || createBudgetDraftFromStatus(null).endpoints;
  const memoryLoad = useMemo(
    () =>
      (typeof stats.memories === "number" ? stats.memories : 0)
      + (typeof stats.decisions === "number" ? stats.decisions : 0),
    [stats]
  );

  const currencyFormatter = useMemo(() => {
    try {
      return new Intl.NumberFormat(undefined, {
        style: "currency",
        currency: safeCurrency,
        maximumFractionDigits: safeCurrency === "JPY" || safeCurrency === "KRW" ? 0 : 2,
      });
    } catch {
      return new Intl.NumberFormat(undefined, {
        style: "currency",
        currency: "USD",
        maximumFractionDigits: 2,
      });
    }
  }, [safeCurrency]);

  const formatCurrency = useCallback(
    (usdAmount) => currencyFormatter.format((Number(usdAmount) || 0) * currencyRate),
    [currencyFormatter, currencyRate]
  );
  const savingsEstimateLegend = useMemo(() => {
    const base = `Assumption: $${SAVINGS_USD_PER_MILLION} USD per 1M tokens saved`;
    return safeCurrency === "USD" ? base : `${base}, converted to ${safeCurrency}`;
  }, [safeCurrency]);

  const formatMissionTokenValue = useCallback((value, { signed = false, perDay = false } = {}) => {
    const numeric = Number(value || 0);
    if (!Number.isFinite(numeric)) {
      return perDay ? "0 tokens/day" : "0 tokens";
    }

    if (showMissionCompactUnits) {
      const compact = signed ? formatSignedCompactNumber(numeric) : formatCompactNumber(numeric);
      return `${compact}t${perDay ? "/day" : ""}`;
    }

    const absRounded = Math.round(Math.abs(numeric)).toLocaleString();
    const signPrefix = signed
      ? (numeric > 0 ? "+" : numeric < 0 ? "-" : "")
      : (numeric < 0 ? "-" : "");
    const valueWithSign = `${signPrefix}${absRounded}`;
    return perDay ? `${valueWithSign} tokens/day` : `${valueWithSign} tokens`;
  }, [showMissionCompactUnits]);

  const clearTransientFeedback = useCallback((fallback = "Connected to daemon.") => {
    setFeedbackMessage((current) => {
      if (isTransientDaemonFeedback(current)) {
        return fallback;
      }
      return current;
    });
  }, []);

  const setSecondaryAvailabilityFeedback = useCallback((errors) => {
    const summary = summarizeDashboardErrors(errors);
    const message = summary
      ? `Connected (core ready). Secondary panels may be stale: ${summary}`
      : "Connected (core ready). Secondary panels may be stale.";
    setFeedbackMessage((current) => {
      const text = String(current || "");
      if (
        !text ||
        text === "Checking daemon..." ||
        text.startsWith("Connected") ||
        text.startsWith("Waiting for daemon auth token")
      ) {
        return message;
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

  const resetStartupRetryState = useCallback(() => {
    startupRetryStateRef.current = { startedAtMs: 0, attempts: 0 };
  }, []);

  const scheduleStartupRecoveryRetry = useCallback((message) => {
    const step = computeStartupRetryStep(startupRetryStateRef.current);
    startupRetryStateRef.current = {
      startedAtMs: step.startedAtMs,
      attempts: step.attempts,
    };
    if (step.exhausted) {
      clearRecoveryRetry();
      const elapsedSeconds = Math.max(1, Math.ceil(step.elapsedMs / 1000));
      setFeedbackMessage(
        `Daemon startup timed out after ${elapsedSeconds}s. Check Cortex logs, then restart from Control Center.`
      );
      return false;
    }
    setFeedbackMessage(message);
    scheduleRecoveryRetry(step.nextDelayMs);
    return true;
  }, [clearRecoveryRetry, scheduleRecoveryRetry]);

  const clearDisconnectedData = useCallback(() => {
    setSessions([]);
    setLocks([]);
    setTasks([]);
    setFeedEntries([]);
    setMessageEntries([]);
    setActivityEntries([]);
    setConflictPairs([]);
    setResolveDrafts({});
    setPermissionGrants([]);
    setPermissionAccessDenied(false);
    setPermissionsEndpointAvailable(true);
    permissionsEndpointAvailableRef.current = true;
    setSavings(null);
    setStats({
      memories: "--",
      decisions: "--",
      events: "--",
    });
  }, []);

  return {
    browserBootstrap,
    isTauriRuntime,
    panel,
    setPanel,
    brainPanelMounted,
    setBrainPanelMounted,
    panelMotionDirection,
    setPanelMotionDirection,
    daemonState,
    setDaemonState,
    healthMeta,
    setHealthMeta,
    stats,
    setStats,
    sessions,
    setSessions,
    tasks,
    setTasks,
    locks,
    setLocks,
    feedEntries,
    setFeedEntries,
    messageEntries,
    setMessageEntries,
    activityEntries,
    setActivityEntries,
    sidebarCollapsed,
    setSidebarCollapsed,
    isNarrowViewport,
    setIsNarrowViewport,
    savings,
    setSavings,
    memoryQuery,
    setMemoryQuery,
    memoryResults,
    setMemoryResults,
    memorySearching,
    setMemorySearching,
    feedFilters,
    setFeedFilters,
    selectedOperator,
    setSelectedOperator,
    messageTarget,
    setMessageTarget,
    messageDraft,
    setMessageDraft,
    taskCompletionDrafts,
    setTaskCompletionDrafts,
    completionTaskId,
    setCompletionTaskId,
    busyActionKey,
    setBusyActionKey,
    activitySince,
    setActivitySince,
    feedbackMessage,
    setFeedbackMessage,
    daemonTimeoutStaleSummary,
    setDaemonTimeoutStaleSummary,
    conflictPairs,
    setConflictPairs,
    resolveDrafts,
    setResolveDrafts,
    conflictLoading,
    setConflictLoading,
    permissionGrants,
    setPermissionGrants,
    permissionLoading,
    setPermissionLoading,
    permissionAccessDenied,
    setPermissionAccessDenied,
    permissionsEndpointAvailable,
    setPermissionsEndpointAvailable,
    permissionDraft,
    setPermissionDraft,
    editorSetup,
    setEditorSetup,
    editorDetections,
    setEditorDetections,
    selectedEditorIds,
    setSelectedEditorIds,
    cortexBase,
    setCortexBase,
    showConnectionDialog,
    setShowConnectionDialog,
    showEditorSetupWizard,
    setShowEditorSetupWizard,
    availableUpdate,
    setAvailableUpdate,
    updateInstalling,
    setUpdateInstalling,
    restartingDaemon,
    setRestartingDaemon,
    restartError,
    setRestartError,
    showMissionMetricLegend,
    setShowMissionMetricLegend,
    showMissionCompactUnits,
    setShowMissionCompactUnits,
    hasVisitedAnalytics,
    setHasVisitedAnalytics,
    analyticsReady,
    setAnalyticsReady,
    startupCoreReadyState,
    setStartupCoreReadyState,
    isSettingUpEditors,
    setIsSettingUpEditors,
    controlSettings,
    setControlSettings,
    budgetConfigStatus,
    setBudgetConfigStatus,
    budgetDraft,
    setBudgetDraft,
    budgetDraftDirty,
    setBudgetDraftDirty,
    budgetConfigBusy,
    setBudgetConfigBusy,
    budgetConfigMessage,
    setBudgetConfigMessage,
    ipcAvailable,
    setIpcAvailable,
    osReducedMotion,
    setOsReducedMotion,
    currency,
    setCurrency,
    analyticsMode,
    setAnalyticsMode,
    effectiveReducedMotion,
    invokeRef,
    tokenRef,
    refreshAllRef,
    refreshAllInFlightRef,
    refreshAllQueuedRef,
    daemonTransitionRef,
    recoveryRetryTimerRef,
    startupRetryStateRef,
    startupCoreReadyRef,
    lastCoreRefreshAtRef,
    lastSecondaryRefreshAtRef,
    startupSecondaryRefreshInFlightRef,
    skipInitialFeedRefreshRef,
    skipInitialMessagesRefreshRef,
    skipInitialActivityRefreshRef,
    connectionDialogRef,
    connectionDialogTriggerRef,
    editorSetupDialogRef,
    editorSetupTriggerRef,
    topbarRef,
    analyticsPanelRef,
    brainPanelRef,
    analyticsTabRefs,
    sessionsRef,
    daemonStateRef,
    streamConnectedAtRef,
    streamDisconnectedAtRef,
    streamSessionEventCountRef,
    devVerificationStartedRef,
    permissionsEndpointAvailableRef,
    browserHealthProbeRef,
    connectionDialogAutoPromptSuppressedRef,
    budgetConfigLoadAttemptedRef,
    restoreFocusToTrigger,
    openConnectionDialog,
    dismissConnectionDialog,
    closeConnectionDialog,
    closeEditorSetupWizard,
    updateControlSetting,
    changePanel,
    normalizedSessions,
    knownAgents,
    editorSetupSummary,
    editorDetectionSummary,
    setupCommandPath,
    manualMcpSnippet,
    selectedOperatorName,
    messageTargetName,
    safeCurrency,
    currencyRate,
    activeBudgetStatus,
    budgetSummary,
    budgetDraftError,
    budgetDraftEndpoints,
    memoryLoad,
    currencyFormatter,
    formatCurrency,
    savingsEstimateLegend,
    formatMissionTokenValue,
    clearTransientFeedback,
    setSecondaryAvailabilityFeedback,
    clearRecoveryRetry,
    scheduleRecoveryRetry,
    resetStartupRetryState,
    scheduleStartupRecoveryRetry,
    clearDisconnectedData,
  };
}
