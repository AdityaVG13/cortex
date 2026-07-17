import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { summarizeDashboardErrors } from "../../api-client.js";
import { USD_TO_CURRENCY_RATE } from "../../constants.js";
import { buildKnownAgents, isTransportSession, resolveAgentName } from "../../live-surface.js";
import { computeStartupRetryStep, isTransientDaemonFeedback } from "../../daemon-startup.js";
import { formatCompactNumber, formatSignedCompactNumber } from "../../number-format.js";
import {
  createBudgetDraftFromStatus,
  readControlCenterSettings,
  resolveEffectiveReducedMotion,
  summarizeBudgetStatus,
  validateBudgetDraft,
} from "../../settings/settings-state.js";
import {
  CORTEX_OPERATOR_STORAGE_KEY,
  DEFAULT_CORTEX_BASE,
  EMPTY_DAEMON,
  EMPTY_HEALTH_META,
  PANEL_SEQUENCE_KEYS,
  SAVINGS_USD_PER_MILLION,
  SIDEBAR_COLLAPSE_BREAKPOINT_PX,
  panelIndex,
} from "../constants.js";
import { readBrowserBootstrap, readLocalStorageValue } from "../browser-bootstrap.js";
import { normalizeCurrencyCode, getOsReducedMotionPreference } from "../utils/format.js";
import { normalizeSession } from "../normalize/sessions.js";
function useDashboardState() {
  const browserBootstrap = useMemo(() => readBrowserBootstrap(), []),
    isTauriRuntime = typeof window < "u" && !!window.__TAURI_INTERNALS__,
    [panel, setPanel] = useState(() => browserBootstrap.panel || "overview"),
    [brainPanelMounted, setBrainPanelMounted] = useState(() => (browserBootstrap.panel || "overview") === "brain"),
    [panelMotionDirection, setPanelMotionDirection] = useState("forward"),
    [daemonState, setDaemonState] = useState(EMPTY_DAEMON),
    [healthMeta, setHealthMeta] = useState(EMPTY_HEALTH_META),
    [stats, setStats] = useState({
      memories: "--",
      decisions: "--",
      events: "--",
    }),
    [sessions, setSessions] = useState([]),
    [tasks, setTasks] = useState([]),
    [locks, setLocks] = useState([]),
    [feedEntries, setFeedEntries] = useState([]),
    [messageEntries, setMessageEntries] = useState([]),
    [activityEntries, setActivityEntries] = useState([]),
    [sidebarCollapsed, setSidebarCollapsed] = useState(() =>
      typeof window > "u" ? !1 : window.innerWidth <= SIDEBAR_COLLAPSE_BREAKPOINT_PX,
    ),
    [isNarrowViewport, setIsNarrowViewport] = useState(() =>
      typeof window > "u" ? !1 : window.innerWidth <= SIDEBAR_COLLAPSE_BREAKPOINT_PX,
    ),
    [savings, setSavings] = useState(null),
    [memoryQuery, setMemoryQuery] = useState(""),
    [memoryResults, setMemoryResults] = useState([]),
    [memorySearching, setMemorySearching] = useState(!1),
    [feedFilters, setFeedFilters] = useState({
      since: "1h",
      kind: "all",
      agent: "",
      unread: !1,
    }),
    [selectedOperator, setSelectedOperator] = useState(() => {
      if (typeof window > "u") return "";
      try {
        return window.localStorage.getItem(CORTEX_OPERATOR_STORAGE_KEY) || "";
      } catch {
        return "";
      }
    }),
    [messageTarget, setMessageTarget] = useState(""),
    [messageDraft, setMessageDraft] = useState(""),
    [taskCompletionDrafts, setTaskCompletionDrafts] = useState({}),
    [completionTaskId, setCompletionTaskId] = useState(""),
    [busyActionKey, setBusyActionKey] = useState(""),
    [activitySince, setActivitySince] = useState("1h"),
    [feedbackMessage, setFeedbackMessage] = useState("Checking daemon..."),
    [daemonTimeoutStaleSummary, setDaemonTimeoutStaleSummary] = useState(""),
    [conflictPairs, setConflictPairs] = useState([]),
    [resolveDrafts, setResolveDrafts] = useState({}),
    [conflictLoading, setConflictLoading] = useState(!1),
    [permissionGrants, setPermissionGrants] = useState([]),
    [permissionLoading, setPermissionLoading] = useState(!1),
    [permissionAccessDenied, setPermissionAccessDenied] = useState(!1),
    [permissionsEndpointAvailable, setPermissionsEndpointAvailable] = useState(!0),
    [permissionDraft, setPermissionDraft] = useState({
      client: "",
      permission: "read",
      scope: "*",
    }),
    [editorSetup, setEditorSetup] = useState(null),
    [editorDetections, setEditorDetections] = useState([]),
    [selectedEditorIds, setSelectedEditorIds] = useState([]),
    [cortexBase, setCortexBase] = useState(() => browserBootstrap.cortexBase || DEFAULT_CORTEX_BASE),
    [showConnectionDialog, setShowConnectionDialog] = useState(!1),
    [showEditorSetupWizard, setShowEditorSetupWizard] = useState(!1),
    [availableUpdate, setAvailableUpdate] = useState(null),
    [updateInstalling, setUpdateInstalling] = useState(!1),
    [restartingDaemon, setRestartingDaemon] = useState(!1),
    [restartError, setRestartError] = useState(""),
    [showMissionMetricLegend, setShowMissionMetricLegend] = useState(!1),
    [showMissionCompactUnits, setShowMissionCompactUnits] = useState(!0),
    [hasVisitedAnalytics, setHasVisitedAnalytics] = useState(() => browserBootstrap.panel === "analytics"),
    [analyticsReady, setAnalyticsReady] = useState(() => browserBootstrap.panel === "analytics"),
    [startupCoreReadyState, setStartupCoreReadyState] = useState(!1),
    [isSettingUpEditors, setIsSettingUpEditors] = useState(!1),
    [controlSettings, setControlSettings] = useState(() => readControlCenterSettings()),
    [budgetConfigStatus, setBudgetConfigStatus] = useState(null),
    [budgetDraft, setBudgetDraft] = useState(() => createBudgetDraftFromStatus(null)),
    [budgetDraftDirty, setBudgetDraftDirty] = useState(!1),
    [budgetConfigBusy, setBudgetConfigBusy] = useState(!1),
    [budgetConfigMessage, setBudgetConfigMessage] = useState(""),
    [ipcAvailable, setIpcAvailable] = useState(!1),
    [osReducedMotion, setOsReducedMotion] = useState(() => getOsReducedMotionPreference()),
    [currency, setCurrency] = useState(() => normalizeCurrencyCode(readLocalStorageValue("cortex_currency", "USD"))),
    [analyticsMode, setAnalyticsMode] = useState(() =>
      readLocalStorageValue("cortex_analytics_mode", "aggregate") === "operations" ? "operations" : "aggregate",
    ),
    effectiveReducedMotion = useMemo(
      () => resolveEffectiveReducedMotion(controlSettings.reducedMotion, osReducedMotion),
      [controlSettings.reducedMotion, osReducedMotion],
    ),
    invokeRef = useRef(null),
    tokenRef = useRef(browserBootstrap.authToken || ""),
    refreshAllRef = useRef(async () => {}),
    refreshAllInFlightRef = useRef(null),
    refreshAllQueuedRef = useRef(!1),
    daemonTransitionRef = useRef(!1),
    recoveryRetryTimerRef = useRef(null),
    startupRetryStateRef = useRef({ startedAtMs: 0, attempts: 0 }),
    startupCoreReadyRef = useRef(!1),
    lastCoreRefreshAtRef = useRef(0),
    lastSecondaryRefreshAtRef = useRef(0),
    startupSecondaryRefreshInFlightRef = useRef(!1),
    skipInitialFeedRefreshRef = useRef(!0),
    skipInitialMessagesRefreshRef = useRef(!0),
    skipInitialActivityRefreshRef = useRef(!0),
    connectionDialogRef = useRef(null),
    connectionDialogTriggerRef = useRef(null),
    editorSetupDialogRef = useRef(null),
    editorSetupTriggerRef = useRef(null),
    topbarRef = useRef(null),
    analyticsPanelRef = useRef(null),
    brainPanelRef = useRef(null),
    analyticsTabRefs = useRef({}),
    sessionsRef = useRef([]),
    daemonStateRef = useRef(EMPTY_DAEMON),
    streamConnectedAtRef = useRef(0),
    streamDisconnectedAtRef = useRef(0),
    streamSessionEventCountRef = useRef(0),
    devVerificationStartedRef = useRef(!1),
    permissionsEndpointAvailableRef = useRef(!0),
    browserHealthProbeRef = useRef(null),
    connectionDialogAutoPromptSuppressedRef = useRef(!1),
    budgetConfigLoadAttemptedRef = useRef(!1),
    restoreFocusToTrigger = useCallback((triggerRef) => {
      typeof window > "u" ||
        window.requestAnimationFrame(() => {
          const target = triggerRef.current;
          ((triggerRef.current = null),
            target && typeof target.focus == "function" && document.contains(target) && target.focus());
        });
    }, []),
    openConnectionDialog = useCallback((event) => {
      ((connectionDialogTriggerRef.current = event?.currentTarget || document.activeElement),
        (connectionDialogAutoPromptSuppressedRef.current = !1),
        setShowConnectionDialog(!0));
    }, []),
    dismissConnectionDialog = useCallback(() => {
      ((connectionDialogAutoPromptSuppressedRef.current = !0),
        setShowConnectionDialog(!1),
        restoreFocusToTrigger(connectionDialogTriggerRef));
    }, [restoreFocusToTrigger]),
    closeConnectionDialog = useCallback(() => {
      ((connectionDialogAutoPromptSuppressedRef.current = !1),
        setShowConnectionDialog(!1),
        restoreFocusToTrigger(connectionDialogTriggerRef));
    }, [restoreFocusToTrigger]),
    closeEditorSetupWizard = useCallback(() => {
      (setShowEditorSetupWizard(!1), restoreFocusToTrigger(editorSetupTriggerRef));
    }, [restoreFocusToTrigger]),
    updateControlSetting = useCallback((key, value) => {
      setControlSettings((current) => ({ ...current, [key]: value }));
    }, []),
    changePanel = useCallback(
      (nextPanel) => {
        if (!PANEL_SEQUENCE_KEYS.has(nextPanel) || nextPanel === panel) return;
        nextPanel === "brain" && setBrainPanelMounted(!0);
        const currentIndex = panelIndex(panel),
          nextIndex = panelIndex(nextPanel);
        (setPanelMotionDirection(
          currentIndex >= 0 && nextIndex >= 0 && nextIndex < currentIndex ? "backward" : "forward",
        ),
          setPanel(nextPanel));
      },
      [panel],
    ),
    normalizedSessions = useMemo(() => {
      if (!Array.isArray(sessions)) return [];
      const sorted = sessions
          .map((session, index) => normalizeSession(session, index))
          .sort((a, b) => b.lastHeartbeatMs - a.lastHeartbeatMs),
        deduped = new Map();
      for (const session of sorted) {
        const agentRaw = String(session?.agent || "").trim();
        if (!agentRaw) {
          deduped.set(session.sessionId || `session-${deduped.size}`, session);
          continue;
        }
        const key =
            agentRaw
              .replace(/\s*\([^)]*\)\s*$/, "")
              .trim()
              .toLowerCase() === "droid"
              ? "droid"
              : agentRaw.toLowerCase(),
          existing = deduped.get(key);
        if (!existing) {
          deduped.set(key, session);
          continue;
        }
        const existingHasModel = /\([^)]+\)/.test(String(existing.agent || ""));
        /\([^)]+\)/.test(agentRaw) && !existingHasModel && deduped.set(key, session);
      }
      return Array.from(deduped.values()).filter((session) => !isTransportSession(session));
    }, [sessions]);
  (useEffect(() => {
    sessionsRef.current = normalizedSessions;
  }, [normalizedSessions]),
    useEffect(() => {
      daemonStateRef.current = daemonState;
    }, [daemonState]));
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
    }, [feedEntries, locks, messageEntries, messageTarget, normalizedSessions, selectedOperator, tasks]),
    editorSetupSummary = useMemo(() => {
      const results = Array.isArray(editorSetup) ? editorSetup : [];
      return {
        results,
        detected: results.filter((entry) => entry.detected).length,
        registered: results.filter((entry) => entry.registered).length,
        failed: results.filter((entry) => entry.detected && !entry.registered).length,
      };
    }, [editorSetup]),
    editorDetectionSummary = useMemo(() => {
      const results = Array.isArray(editorDetections) ? editorDetections : [];
      return {
        results,
        detected: results.filter((entry) => entry.detected).length,
        registered: results.filter((entry) => entry.registered).length,
      };
    }, [editorDetections]),
    setupCommandPath = useMemo(() => {
      const current = editorDetectionSummary.results.find((entry) => entry.commandPath)?.commandPath,
        previous = editorSetupSummary.results.find((entry) => entry.commandPath)?.commandPath;
      return current || previous || "C:\\Users\\<you>\\.cortex\\bin\\cortex.exe";
    }, [editorDetectionSummary.results, editorSetupSummary.results]),
    manualMcpSnippet = useMemo(
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
    ),
    selectedOperatorName = useMemo(
      () => resolveAgentName(selectedOperator, knownAgents),
      [knownAgents, selectedOperator],
    ),
    messageTargetName = useMemo(() => resolveAgentName(messageTarget, knownAgents), [knownAgents, messageTarget]),
    safeCurrency = normalizeCurrencyCode(currency),
    currencyRate = USD_TO_CURRENCY_RATE[safeCurrency] ?? USD_TO_CURRENCY_RATE.USD,
    activeBudgetStatus = budgetConfigStatus || healthMeta.budgets,
    budgetSummary = useMemo(() => summarizeBudgetStatus(activeBudgetStatus), [activeBudgetStatus]),
    budgetDraftError = useMemo(() => validateBudgetDraft(budgetDraft), [budgetDraft]),
    budgetDraftEndpoints = budgetDraft?.endpoints || createBudgetDraftFromStatus(null).endpoints,
    memoryLoad = useMemo(
      () =>
        (typeof stats.memories == "number" ? stats.memories : 0) +
        (typeof stats.decisions == "number" ? stats.decisions : 0),
      [stats],
    ),
    currencyFormatter = useMemo(() => {
      try {
        return new Intl.NumberFormat(void 0, {
          style: "currency",
          currency: safeCurrency,
          maximumFractionDigits: safeCurrency === "JPY" || safeCurrency === "KRW" ? 0 : 2,
        });
      } catch {
        return new Intl.NumberFormat(void 0, {
          style: "currency",
          currency: "USD",
          maximumFractionDigits: 2,
        });
      }
    }, [safeCurrency]),
    formatCurrency = useCallback(
      (usdAmount) => currencyFormatter.format((Number(usdAmount) || 0) * currencyRate),
      [currencyFormatter, currencyRate],
    ),
    savingsEstimateLegend = useMemo(() => {
      const base = `Assumption: $${SAVINGS_USD_PER_MILLION} USD per 1M tokens saved`;
      return safeCurrency === "USD" ? base : `${base}, converted to ${safeCurrency}`;
    }, [safeCurrency]),
    formatMissionTokenValue = useCallback(
      (value, { signed = !1, perDay = !1 } = {}) => {
        const numeric = Number(value || 0);
        if (!Number.isFinite(numeric)) return perDay ? "0 tokens/day" : "0 tokens";
        if (showMissionCompactUnits)
          return `${signed ? formatSignedCompactNumber(numeric) : formatCompactNumber(numeric)}t${perDay ? "/day" : ""}`;
        const absRounded = Math.round(Math.abs(numeric)).toLocaleString(),
          valueWithSign = `${signed && numeric > 0 ? "+" : numeric < 0 ? "-" : ""}${absRounded}`;
        return perDay ? `${valueWithSign} tokens/day` : `${valueWithSign} tokens`;
      },
      [showMissionCompactUnits],
    ),
    clearTransientFeedback = useCallback((fallback = "Connected to daemon.") => {
      setFeedbackMessage((current) => (isTransientDaemonFeedback(current) ? fallback : current));
    }, []),
    setSecondaryAvailabilityFeedback = useCallback((errors) => {
      const summary = summarizeDashboardErrors(errors),
        message = summary
          ? `Connected (core ready). Secondary panels may be stale: ${summary}`
          : "Connected (core ready). Secondary panels may be stale.";
      setFeedbackMessage((current) => {
        const text = String(current || "");
        return !text ||
          text === "Checking daemon..." ||
          text.startsWith("Connected") ||
          text.startsWith("Waiting for daemon auth token")
          ? message
          : current;
      });
    }, []),
    clearRecoveryRetry = useCallback(() => {
      typeof window > "u" ||
        !recoveryRetryTimerRef.current ||
        (window.clearTimeout(recoveryRetryTimerRef.current), (recoveryRetryTimerRef.current = null));
    }, []),
    scheduleRecoveryRetry = useCallback((delay = 1e3) => {
      typeof window > "u" ||
        recoveryRetryTimerRef.current ||
        (recoveryRetryTimerRef.current = window.setTimeout(() => {
          ((recoveryRetryTimerRef.current = null), refreshAllRef.current());
        }, delay));
    }, []),
    resetStartupRetryState = useCallback(() => {
      startupRetryStateRef.current = { startedAtMs: 0, attempts: 0 };
    }, []),
    scheduleStartupRecoveryRetry = useCallback(
      (message) => {
        const step = computeStartupRetryStep(startupRetryStateRef.current);
        if (
          ((startupRetryStateRef.current = {
            startedAtMs: step.startedAtMs,
            attempts: step.attempts,
          }),
          step.exhausted)
        ) {
          clearRecoveryRetry();
          const elapsedSeconds = Math.max(1, Math.ceil(step.elapsedMs / 1e3));
          return (
            setFeedbackMessage(
              `Daemon startup timed out after ${elapsedSeconds}s. Check Cortex logs, then restart from Control Center.`,
            ),
            !1
          );
        }
        return (setFeedbackMessage(message), scheduleRecoveryRetry(step.nextDelayMs), !0);
      },
      [clearRecoveryRetry, scheduleRecoveryRetry],
    ),
    clearDisconnectedData = useCallback(() => {
      (setSessions([]),
        setLocks([]),
        setTasks([]),
        setFeedEntries([]),
        setMessageEntries([]),
        setActivityEntries([]),
        setConflictPairs([]),
        setResolveDrafts({}),
        setPermissionGrants([]),
        setPermissionAccessDenied(!1),
        setPermissionsEndpointAvailable(!0),
        (permissionsEndpointAvailableRef.current = !0),
        setSavings(null),
        setStats({ memories: "--", decisions: "--", events: "--" }));
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
export { useDashboardState };
