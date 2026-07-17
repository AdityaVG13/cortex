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

export function useRefreshOrchestration(ctx) {
  const {
    panel,
    setPanel,
    brainPanelMounted,
    panelMotionDirection,
    daemonState,
    healthMeta,
    stats,
    sessions,
    tasks,
    locks,
    feedEntries,
    messageEntries,
    activityEntries,
    sidebarCollapsed,
    setSidebarCollapsed,
    isNarrowViewport,
    savings,
    memoryQuery,
    setMemoryQuery,
    memoryResults,
    memorySearching,
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
    activitySince,
    setActivitySince,
    feedbackMessage,
    daemonTimeoutStaleSummary,
    conflictPairs,
    resolveDrafts,
    conflictLoading,
    permissionGrants,
    permissionLoading,
    permissionAccessDenied,
    permissionsEndpointAvailable,
    permissionDraft,
    setPermissionDraft,
    editorSetup,
    editorDetections,
    selectedEditorIds,
    cortexBase,
    setCortexBase,
    showConnectionDialog,
    showEditorSetupWizard,
    availableUpdate,
    updateInstalling,
    setUpdateInstalling,
    setFeedbackMessage,
    restartingDaemon,
    restartError,
    showMissionMetricLegend,
    setShowMissionMetricLegend,
    showMissionCompactUnits,
    setShowMissionCompactUnits,
    hasVisitedAnalytics,
    analyticsReady,
    isSettingUpEditors,
    controlSettings,
    budgetConfigStatus,
    budgetDraft,
    budgetDraftDirty,
    budgetConfigBusy,
    budgetConfigMessage,
    ipcAvailable,
    currency,
    setCurrency,
    analyticsMode,
    setAnalyticsMode,
    effectiveReducedMotion,
    invokeRef,
    refreshAllRef,
    tokenRef,
    connectionDialogRef,
    connectionDialogTriggerRef,
    editorSetupDialogRef,
    editorSetupTriggerRef,
    topbarRef,
    analyticsPanelRef,
    brainPanelRef,
    analyticsTabRefs,
    isTauriRuntime,
    changePanel,
    normalizedSessions,
    knownAgents,
    editorSetupSummary,
    editorDetectionSummary,
    manualMcpSnippet,
    selectedOperatorName,
    messageTargetName,
    safeCurrency,
    budgetSummary,
    budgetDraftError,
    budgetDraftEndpoints,
    memoryLoad,
    formatCurrency,
    savingsEstimateLegend,
    formatMissionTokenValue,
    runRefreshAll,
    openConnectionDialog,
    dismissConnectionDialog,
    closeConnectionDialog,
    closeEditorSetupWizard,
    updateControlSetting,
    restoreFocusToTrigger,
    handleStartDaemon,
    handleStopDaemon,
    handleRestartDaemon,
    handleTaskClaim,
    handleTaskAbandon,
    handleTaskComplete,
    handleTaskDelete,
    handleUnlock,
    handleSendMessage,
    handleFeedAck,
    handleMemorySearch,
    handleMemoryExpand,
    reportSurfaceError,
    pill,
    utilityPill,
    sidebarUtilityStats,
    daemonRecoveryHint,
    daemonStatusBadge,
    daemonSysStatus,
    pendingTasks,
    claimedTasks,
    completedTasks,
    monteCarloProjection,
    bootSavingsMomentum,
    latestRecallHitRate,
    recallWindowAverage,
    recallWindowSpread,
    topActivityEntries,
    topFeedEntries,
    recentOverviewTasks,
    firstRunReadiness,
    handleFirstRunAction,
    activePanelLabel,
    connectionEndpoint,
    hostLabel,
    handleAnalyticsTabKey,
    effectiveSidebarCollapsed,
    canStartDaemon,
    canStopDaemon,
    canSetupEditors,
    operationRows,
    operationMaxSaved,
    topSavingsByAgent,
  } = ctx;

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

  const callMcpTool = useCallback(async (name, args = {}) => {
    const payload = await postApi("/mcp-rpc", {
      jsonrpc: "2.0",
      id: `control-center-${Date.now()}-${Math.random().toString(16).slice(2)}`,
      method: "tools/call",
      params: {
        name,
        arguments: args,
      },
    });
    const error = extractMcpToolError(payload);
    if (error) {
      throw new Error(`MCP ${name} failed: ${error}`);
    }
    return parseMcpToolResult(payload?.result) ?? payload?.result ?? null;
  }, [postApi]);

  const writeDevVerificationReport = useCallback(async (report) => {
    if (!DEV_RESTART_VERIFY_ENABLED) {
      return "";
    }
    return call("write_dev_verification_report", {
      content: JSON.stringify(report, null, 2),
    });
  }, [call]);

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
        if (!suppressFeedback && (!daemonTransitionRef.current || !isDaemonSuppressibleErrorMessage(message))) {
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
        browserHealthProbeRef.current = null;
        setDaemonState(state);
        return state;
      } catch {
        // fallback to HTTP health
      }
    }

    let health;
    try {
      health = await api("/health");
      browserHealthProbeRef.current = health || null;
    } catch {
      // daemon unreachable is an expected state, not an error
      browserHealthProbeRef.current = null;
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

  const probeReadiness = useCallback(async () => {
    try {
      const readiness = await api("/readiness");
      return isReadyReadinessPayload(readiness);
    } catch {
      return false;
    }
  }, [api]);

  const refreshHealth = useCallback(async () => {
    let health = browserHealthProbeRef.current;
    browserHealthProbeRef.current = null;
    if (!health) {
      try {
        health = await api("/health");
      } catch {
        // daemon unreachable -- show dashes
      }
    }
    if (!health) {
      const readinessReady = await probeReadiness();
      setHealthMeta(EMPTY_HEALTH_META);
      setStats({
        memories: "--",
        decisions: "--",
        events: "--",
      });
      return readinessReady;
    }

    const status = String(health?.status || "unknown").toLowerCase();
    const runtimeVersion = String(health?.runtime?.version || "");
    setHealthMeta({
      status,
      degraded: Boolean(health?.degraded),
      dbCorrupted: Boolean(health?.db_corrupted),
      runtimeVersion,
      budgets: health?.budgets || null,
    });

    if (!health?.stats) {
      setStats({
        memories: "--",
        decisions: "--",
        events: "--",
      });
      return isReachableHealthPayload(health);
    }

    const next = health.stats;
    setStats({
      memories: next.memories ?? 0,
      decisions: next.decisions ?? 0,
      events: next.events ?? 0,
    });
    return isReachableHealthPayload(health);
  }, [api, probeReadiness]);

  const refreshCoreData = useCallback(async (options = {}) => {
    const throwOnError = options?.throwOnError !== false;
    const jobs = [
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
    ];

    const results = await Promise.allSettled(jobs.map((job) => job.fn()));
    const errors = [];
    let successCount = 0;
    results.forEach((result, index) => {
      if (result.status === "fulfilled") {
        jobs[index].apply(result.value);
        successCount += 1;
        return;
      }
      errors.push(result.reason?.message || String(result.reason));
    });

    if (successCount > 0) {
      clearTransientFeedback();
    }

    const summary = {
      errors: [...new Set(errors)],
      successCount,
      totalCount: jobs.length,
    };
    if (throwOnError && summary.errors.length) {
      throw new Error(summary.errors.join("; "));
    }
    return summary;
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
    const normalizedPairs = normalizeConflictPairsPayload(result);
    setConflictPairs(normalizedPairs);
    setResolveDrafts((current) => {
      if (!current || typeof current !== "object") return {};
      const next = {};
      const validKeys = new Set(normalizedPairs.map((pair) => pair.key));
      for (const [key, value] of Object.entries(current)) {
        if (validKeys.has(key)) {
          next[key] = value;
        }
      }
      return next;
    });
    clearTransientFeedback();
  }, [api, clearTransientFeedback]);

  const refreshPermissions = useCallback(async (options = {}) => {
    const force = options?.force === true;
    if (!force && !permissionsEndpointAvailableRef.current) {
      return;
    }
    try {
      const result = await api("/permissions", true);
      permissionsEndpointAvailableRef.current = true;
      setPermissionsEndpointAvailable(true);
      setPermissionGrants(normalizePermissionPayload(result));
      setPermissionAccessDenied(false);
      clearTransientFeedback();
    } catch (error) {
      if (String(error?.message || error || "").includes("HTTP 403")) {
        permissionsEndpointAvailableRef.current = true;
        setPermissionsEndpointAvailable(true);
        setPermissionAccessDenied(true);
        setPermissionGrants([]);
        return;
      }
      if (isRouteMissingError(error)) {
        permissionsEndpointAvailableRef.current = false;
        setPermissionsEndpointAvailable(false);
        setPermissionAccessDenied(false);
        setPermissionGrants([]);
        clearTransientFeedback();
        return;
      }
      throw error;
    }
  }, [api, clearTransientFeedback]);

  const refreshSecondaryData = useCallback(async (options = {}) => {
    const force = options?.force === true;
    const wantsWorkStreams = panel === "work" || panel === "overview";
    const wantsMemoryAdmin = panel === "memory";
    const jobs = [];
    if (wantsWorkStreams) {
      jobs.push(refreshFeed, refreshMessages, refreshActivity);
    }
    if (wantsWorkStreams || wantsMemoryAdmin) {
      jobs.push(refreshConflicts);
    }
    if (wantsMemoryAdmin) {
      jobs.push(() => refreshPermissions({ force }));
    }
    if (!jobs.length) {
      return [];
    }
    return settledCollectErrors(jobs);
  }, [
    panel,
    refreshFeed,
    refreshMessages,
    refreshActivity,
    refreshConflicts,
    refreshPermissions,
  ]);

  const refreshProtectedData = useCallback(async (options = {}) => {
    const includeSecondary = options?.includeSecondary !== false;
    const forceCore = options?.forceCore === true;
    const forceSecondary = options?.forceSecondary === true;
    const now = Date.now();
    const shouldRefreshCore = forceCore || now - lastCoreRefreshAtRef.current >= CORE_REFRESH_MIN_INTERVAL_MS;
    let coreErrors = [];
    let coreSuccessCount = 0;
    let coreTotalCount = 0;
    if (shouldRefreshCore) {
      const coreRefresh = await refreshCoreData({ throwOnError: false });
      coreErrors = coreRefresh.errors;
      coreSuccessCount = coreRefresh.successCount;
      coreTotalCount = coreRefresh.totalCount;
      if (!coreErrors.length) {
        lastCoreRefreshAtRef.current = Date.now();
      }
    }
    if (coreErrors.length || !includeSecondary) {
      return { coreErrors, secondaryErrors: [], coreSuccessCount, coreTotalCount };
    }
    const shouldRefreshSecondary =
      forceSecondary || now - lastSecondaryRefreshAtRef.current >= SECONDARY_REFRESH_MIN_INTERVAL_MS;
    if (!shouldRefreshSecondary) {
      return { coreErrors: [], secondaryErrors: [], coreSuccessCount, coreTotalCount };
    }
    const secondaryErrors = await refreshSecondaryData({ force: forceSecondary });
    if (!secondaryErrors.length) {
      lastSecondaryRefreshAtRef.current = Date.now();
    }
    return { coreErrors: [], secondaryErrors, coreSuccessCount, coreTotalCount };
  }, [
    refreshCoreData,
    refreshSecondaryData,
  ]);

  const refreshSecondaryDataInBackground = useCallback(() => {
    if (typeof window === "undefined" || startupSecondaryRefreshInFlightRef.current) {
      return;
    }
    startupSecondaryRefreshInFlightRef.current = true;
    window.setTimeout(() => {
      void (async () => {
        if (!daemonStateRef.current?.reachable) {
          return;
        }
        const secondaryErrors = await refreshSecondaryData({ force: true });
        if (!secondaryErrors.length) {
          setDaemonTimeoutStaleSummary("");
          lastSecondaryRefreshAtRef.current = Date.now();
        }
        if (!secondaryErrors.length || !daemonStateRef.current?.reachable) {
          return;
        }
        const timeoutErrors = secondaryErrors.filter((error) => isDaemonTimeoutErrorMessage(error));
        if (timeoutErrors.length) {
          setDaemonTimeoutStaleSummary(summarizeDashboardErrors(timeoutErrors) || "IPC request timeouts detected.");
        } else {
          setDaemonTimeoutStaleSummary("");
        }
        setSecondaryAvailabilityFeedback(secondaryErrors);
      })().finally(() => {
        startupSecondaryRefreshInFlightRef.current = false;
      });
    }, 0);
  }, [refreshSecondaryData, setSecondaryAvailabilityFeedback]);

  const refreshProtectedDataForStartup = useCallback(async () => {
    // First successful pass only waits on core data; secondary calls hydrate in background.
    const includeSecondary = startupCoreReadyRef.current;
    let result = await refreshProtectedData({ includeSecondary, forceCore: true });
    if (!includeSecondary && !result.coreErrors.length) {
      startupCoreReadyRef.current = true;
      setStartupCoreReadyState(true);
      refreshSecondaryDataInBackground();
      result = { ...result, secondaryErrors: [] };
    }
    return result;
  }, [refreshProtectedData, refreshSecondaryDataInBackground]);

  const clearStartupCoreReady = useCallback(() => {
    startupCoreReadyRef.current = false;
    setStartupCoreReadyState(false);
    lastCoreRefreshAtRef.current = 0;
    lastSecondaryRefreshAtRef.current = 0;
  }, []);

  const handleResolveConflict = useCallback(async (keepId, action, supersededId, pair = null) => {
    const resolver = selectedOperatorName ? `user:${selectedOperatorName}` : "user:control-center";
    const resolutionBody = {
      keepId,
      action,
      supersededId,
      conflictId: pair?.conflictId || null,
      winnerId: action === "keep" ? keepId : null,
      loserId: action === "keep" ? supersededId : null,
      resolution: action,
      resolvedBy: resolver,
    };
    setConflictLoading(true);
    try {
      try {
        await postApi("/conflicts/resolve", resolutionBody);
      } catch (primaryError) {
        if (!isRouteMissingError(primaryError)) {
          throw primaryError;
        }
        await postApi("/resolve", resolutionBody);
      }
      await refreshConflicts();
    } catch (err) {
      setFeedbackMessage(`Resolve failed: ${err.message || err}`);
    } finally {
      setConflictLoading(false);
    }
  }, [postApi, refreshConflicts, selectedOperatorName]);

  const handleResolveDraftChange = useCallback((pairKey, updates) => {
    setResolveDrafts((current) => {
      const draft = current[pairKey] || { action: "keep", winner: "left" };
      return {
        ...current,
        [pairKey]: {
          ...draft,
          ...updates,
        },
      };
    });
  }, []);

  const handleGrantPermission = useCallback(async () => {
    if (!permissionsEndpointAvailable) {
      setFeedbackMessage("Permission endpoint unavailable on this daemon build.");
      return;
    }
    const client = String(permissionDraft.client || "").trim();
    if (!client) {
      setFeedbackMessage("Permission grant failed: client is required.");
      return;
    }

    setPermissionLoading(true);
    try {
      await postApi("/permissions/grant", {
        client,
        permission: permissionDraft.permission || "read",
        scope: String(permissionDraft.scope || "*").trim() || "*",
        grantedBy: selectedOperatorName
          ? `user:${selectedOperatorName}`
          : "user:control-center",
      });
      setPermissionDraft((current) => ({
        ...current,
        client: "",
      }));
      await refreshPermissions({ force: true });
    } catch (err) {
      setFeedbackMessage(`Permission grant failed: ${err.message || err}`);
    } finally {
      setPermissionLoading(false);
    }
  }, [permissionDraft, permissionsEndpointAvailable, postApi, refreshPermissions, selectedOperatorName]);

  const handleRevokePermission = useCallback(
    async (grant) => {
      if (!permissionsEndpointAvailable) {
        setFeedbackMessage("Permission endpoint unavailable on this daemon build.");
        return;
      }
      if (!grant?.client || !grant?.permission) return;
      setPermissionLoading(true);
      try {
        await postApi("/permissions/revoke", {
          client: grant.client,
          permission: grant.permission,
          scope: grant.scope || "*",
        });
        await refreshPermissions({ force: true });
      } catch (err) {
        setFeedbackMessage(`Permission revoke failed: ${err.message || err}`);
      } finally {
        setPermissionLoading(false);
      }
    },
    [permissionsEndpointAvailable, postApi, refreshPermissions]
  );

  const openEditorSetupWizard = useCallback(async (event) => {
    editorSetupTriggerRef.current = event?.currentTarget || document.activeElement;
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
      closeEditorSetupWizard();
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
  }, [call, closeEditorSetupWizard, selectedEditorIds]);

  const updateBudgetDraftRoot = useCallback((patch) => {
    setBudgetDraftDirty(true);
    setBudgetConfigMessage("");
    setBudgetDraft((current) => ({
      ...(current?.endpoints ? current : createBudgetDraftFromStatus(null)),
      ...patch,
    }));
  }, []);

  const updateBudgetEndpointDraft = useCallback((endpoint, patch) => {
    setBudgetDraftDirty(true);
    setBudgetConfigMessage("");
    setBudgetDraft((current) => {
      const base = current?.endpoints ? current : createBudgetDraftFromStatus(null);
      return {
        ...base,
        endpoints: {
          ...base.endpoints,
          [endpoint]: {
            ...base.endpoints[endpoint],
            ...patch,
          },
        },
      };
    });
  }, []);

  const reloadBudgetConfigDraft = useCallback(async ({ silent = false } = {}) => {
    if (!invokeRef.current) {
      if (!silent) setBudgetConfigMessage("Budget editing requires the desktop app.");
      return;
    }

    budgetConfigLoadAttemptedRef.current = true;
    setBudgetConfigBusy(true);
    try {
      const status = await call("read_budget_config");
      setBudgetConfigStatus(status);
      setHealthMeta((current) => ({ ...current, budgets: status }));
      setBudgetDraft(createBudgetDraftFromStatus(status));
      setBudgetDraftDirty(false);
      if (!silent) {
        setBudgetConfigMessage(status?.source ? `Loaded ${status.source}` : "Loaded budget config.");
      }
    } catch (err) {
      setBudgetConfigMessage(`Budget load failed: ${err?.message || String(err)}`);
    } finally {
      setBudgetConfigBusy(false);
    }
  }, [call]);

  const saveBudgetConfigDraft = useCallback(async (event) => {
    event.preventDefault();
    if (!invokeRef.current) {
      setBudgetConfigMessage("Budget editing requires the desktop app.");
      return;
    }

    const validationError = validateBudgetDraft(budgetDraft);
    if (validationError) {
      setBudgetConfigMessage(validationError);
      return;
    }

    setBudgetConfigBusy(true);
    try {
      const status = await call("save_budget_config", {
        draft: serializeBudgetDraftForSave(budgetDraft),
      });
      setBudgetConfigStatus(status);
      setHealthMeta((current) => ({ ...current, budgets: status }));
      setBudgetDraft(createBudgetDraftFromStatus(status));
      setBudgetDraftDirty(false);
      setBudgetConfigMessage("Saved budgets.toml. Restart daemon to apply enforcement.");
      setFeedbackMessage("Budget config saved.");
    } catch (err) {
      setBudgetConfigMessage(`Budget save failed: ${err?.message || String(err)}`);
    } finally {
      setBudgetConfigBusy(false);
    }
  }, [budgetDraft, call]);

  return { ...ctx };
}
