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

export function useDaemonConnection(ctx) {
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
    toggleEditorSelection,
    openEditorSetupWizard,
    applyEditorSetup,
    reloadBudgetConfigDraft,
    saveBudgetConfigDraft,
    updateBudgetDraftRoot,
    updateBudgetEndpointDraft,
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
    handleResolveConflict,
    handleResolveDraftChange,
    handleGrantPermission,
    handleRevokePermission,
    refreshMessages,
    refreshActivity,
    refreshFeed,
    refreshConflicts,
    refreshPermissions,
    refreshSavings,
    reportSurfaceError,
    readAuthToken,
    api,
    postApi,
    call,
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

  const waitForDaemonReachable = useCallback(async (options = {}) => {
    const shortCircuitIfStarting = options?.shortCircuitIfStarting === true;
    const started = Date.now();
    while (Date.now() - started < DAEMON_START_WAIT_TIMEOUT_MS) {
      try {
        if (invokeRef.current) {
          const state = { ...EMPTY_DAEMON, ...(await call("daemon_status")) };
          setDaemonState(state);
          if (state?.reachable) return true;
          if (
            shortCircuitIfStarting
            && state?.running
            && !state?.reachable
            && Date.now() - started >= DAEMON_START_STILL_STARTING_GRACE_MS
          ) {
            return false;
          }
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

  const runRestartDaemonSequence = useCallback(async () => {
    daemonTransitionRef.current = true;
    resetStartupRetryState();

    const statusBefore = await call("daemon_status").catch(() => null);
    const shouldStop = Boolean(statusBefore?.running || statusBefore?.reachable);
    const managedBefore = Boolean(statusBefore?.managed);
    let restartSkippedExternal = false;
    let startResult = null;

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
      const stopState = stopResult?.ok ? stopResult.result : null;
      const unmanagedStillReachable = Boolean(stopState?.reachable && !stopState?.managed);
      const stopped = unmanagedStillReachable ? false : await waitForDaemonOffline();
      if (!stopped) {
        if (unmanagedStillReachable && !managedBefore) {
          restartSkippedExternal = true;
          setFeedbackMessage("Daemon is externally managed and remained online. Continuing without forced shutdown.");
        } else {
          throw new Error(stopFailure || "Existing daemon did not stop cleanly.");
        }
      }
      if (!restartSkippedExternal) {
        tokenRef.current = "";
        persistBrowserAuthToken("");
        clearDisconnectedData();
        setDaemonState({
          running: false,
          reachable: false,
          managed: false,
          authTokenReady: false,
          pid: null,
          message: `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`,
        });
      }
    } else {
      setFeedbackMessage("Daemon already stopped. Starting...");
    }

    if (!restartSkippedExternal) {
      setFeedbackMessage("Restarting daemon: starting...");
      startResult = await call("start_daemon");
      if (startResult?.message) {
        setFeedbackMessage(startResult.message);
      }

      const reachable = await waitForDaemonReachable({ shortCircuitIfStarting: true });
      if (!reachable) {
        if (startResult?.running && !startResult?.reachable) {
          scheduleStartupRecoveryRetry("Daemon is still starting. Reconnect will continue automatically.");
        } else {
          throw new Error("Daemon did not become reachable after restart.");
        }
      }
    } else {
      startResult = await call("daemon_status").catch(() => ({
        running: true,
        reachable: true,
        managed: false,
        authTokenReady: Boolean(tokenRef.current),
        pid: null,
        message: "Daemon remained online (externally managed).",
      }));
    }

    daemonTransitionRef.current = false;
    await readAuthToken({ suppressFeedback: true });
    await runRefreshAll();
    return { ...startResult, restartSkippedExternal };
  }, [call, clearDisconnectedData, cortexBase, readAuthToken, resetStartupRetryState, runRefreshAll, scheduleStartupRecoveryRetry, waitForDaemonOffline, waitForDaemonReachable]);
  return ctx;
}
