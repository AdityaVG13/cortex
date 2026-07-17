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

export function useRefreshAll(ctx) {
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

  const refreshAll = useCallback(async () => {
    try {
      invokeRef.current = await readTauriInvoke();
    } catch {
      invokeRef.current = null;
    }
    setIpcAvailable(Boolean(invokeRef.current));

    const nextDaemonState = await refreshDaemonState();
    let healthReady = await refreshHealth();
    let readinessReady = false;
    if (
      invokeRef.current
      && nextDaemonState?.managed
      && !nextDaemonState?.reachable
      && !healthReady
    ) {
      readinessReady = await probeReadiness();
      if (readinessReady) {
        healthReady = true;
      }
    }
    const reachableViaHealthFallback =
      Boolean(invokeRef.current)
      && Boolean(healthReady)
      && !Boolean(nextDaemonState?.reachable);
    const reachableViaReadinessFallback =
      Boolean(invokeRef.current)
      && Boolean(readinessReady)
      && !Boolean(nextDaemonState?.reachable);
    const daemonReachable =
      Boolean(nextDaemonState?.reachable) || reachableViaHealthFallback || reachableViaReadinessFallback;

    if (daemonTransitionRef.current) {
      return;
    }

    if (reachableViaHealthFallback || reachableViaReadinessFallback) {
      setDaemonState((current) => ({
        ...current,
        running: true,
        reachable: true,
        managed: Boolean(nextDaemonState?.managed),
        authTokenReady: Boolean(tokenRef.current),
        message: `Connected to daemon on ${formatDaemonEndpoint(cortexBase)} (${reachableViaReadinessFallback ? "readiness fallback active" : "IPC fallback active"}).`,
      }));
    }

    const keepStartupRecovery =
      shouldContinueStartupRecovery({
        invokeAvailable: Boolean(invokeRef.current),
        daemonReachable,
        currentDaemonState: nextDaemonState,
        previousDaemonState: daemonStateRef.current,
      });

    if (keepStartupRecovery) {
      setDaemonTimeoutStaleSummary("");
      clearStartupCoreReady();
      if (!scheduleStartupRecoveryRetry("Daemon is still starting. Reconnect will continue automatically.")) {
        let timeoutMessage = `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`;
        try {
          const stopResult = await call("stop_daemon");
          if (stopResult?.message) {
            timeoutMessage = `${timeoutMessage}. ${stopResult.message}`;
          }
        } catch (error) {
          const detail = error?.message || String(error || "");
          if (detail) {
            timeoutMessage = `${timeoutMessage}. Startup recovery cleanup failed: ${detail}`;
          }
        }
        tokenRef.current = "";
        persistBrowserAuthToken("");
        clearDisconnectedData();
        setDaemonState({
          running: false,
          reachable: false,
          managed: false,
          authTokenReady: false,
          pid: null,
          message: timeoutMessage,
        });
      }
      return;
    }

    if (!daemonReachable) {
      resetStartupRetryState();
      setDaemonTimeoutStaleSummary("");
      clearStartupCoreReady();
      clearRecoveryRetry();
      if (invokeRef.current) {
        tokenRef.current = "";
        persistBrowserAuthToken("");
      }
      clearDisconnectedData();
      clearTransientFeedback(nextDaemonState?.message || `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`);
      return;
    }

    if (invokeRef.current && !healthReady) {
      setDaemonTimeoutStaleSummary("");
      clearStartupCoreReady();
      scheduleStartupRecoveryRetry("Daemon is reachable but still warming up. Retrying shortly...");
      return;
    }

    const authToken = await readAuthToken({ suppressFeedback: true });
    if (invokeRef.current && !authToken) {
      setDaemonTimeoutStaleSummary("");
      clearStartupCoreReady();
      scheduleStartupRecoveryRetry("Waiting for daemon auth token to finish rotating...");
      return;
    }

    resetStartupRetryState();
    let {
      coreErrors,
      secondaryErrors,
      coreSuccessCount,
      coreTotalCount,
    } = await refreshProtectedDataForStartup();
    if (invokeRef.current && coreErrors.length && coreErrors.every((error) => isAuthFailure(error))) {
      const refreshedToken = await readAuthToken({ suppressFeedback: true });
      if (refreshedToken) {
        ({
          coreErrors,
          secondaryErrors,
          coreSuccessCount,
          coreTotalCount,
        } = await refreshProtectedDataForStartup());
      }
    }
    const browserCoreAuthFailuresOnly =
      !invokeRef.current
      && coreErrors.length > 0
      && coreErrors.every((error) => isAuthFailure(error));
    if (browserCoreAuthFailuresOnly) {
      tokenRef.current = "";
      persistBrowserAuthToken("");
    }

    if (coreErrors.length) {
      const unique = [...new Set(coreErrors)];
      const timeoutErrors = unique.filter((error) => isDaemonTimeoutErrorMessage(error));
      const warmupErrorsOnly = unique.every(
        (error) => isDaemonTimeoutErrorMessage(error) || isAuthFailure(error)
      );
      const partialCoreReady =
        daemonReachable
        && coreSuccessCount > 0
        && warmupErrorsOnly;
      if (partialCoreReady) {
        startupCoreReadyRef.current = true;
        setStartupCoreReadyState(true);
        refreshSecondaryDataInBackground();
        const timeoutSummary = timeoutErrors.length
          ? summarizeDashboardErrors(timeoutErrors) || "IPC request timeouts detected."
          : "";
        if (timeoutSummary) {
          setDaemonTimeoutStaleSummary(timeoutSummary);
        } else {
          setDaemonTimeoutStaleSummary("");
        }
        const partialSummary = summarizeDashboardErrors(unique) || "Protected endpoints are still warming up.";
        setFeedbackMessage(
          `Connected (core ${coreSuccessCount}/${coreTotalCount || 3} ready). ${partialSummary}`
        );
        scheduleRecoveryRetry(1000);
      } else if (daemonReachable && unique.every((error) => isDaemonTimeoutErrorMessage(error))) {
        clearStartupCoreReady();
        clearRecoveryRetry();
        const summary = summarizeDashboardErrors(unique) || "IPC request timeouts detected.";
        setDaemonTimeoutStaleSummary(summary);
        setFeedbackMessage(
          summary
            ? `Connected (core stale). IPC requests timed out: ${summary}`
            : "Connected (core stale). IPC requests timed out; retrying."
        );
        scheduleRecoveryRetry(1000);
      } else if (unique.every((error) => isDaemonOfflineErrorMessage(error))) {
        clearStartupCoreReady();
        setDaemonTimeoutStaleSummary("");
        clearDisconnectedData();
        clearTransientFeedback(nextDaemonState?.message || `Cannot reach daemon on ${formatDaemonEndpoint(cortexBase)}`);
        scheduleRecoveryRetry(1000);
      } else if (invokeRef.current && unique.every((error) => isAuthFailure(error))) {
        clearStartupCoreReady();
        setDaemonTimeoutStaleSummary("");
        setFeedbackMessage("Waiting for daemon auth token to finish rotating...");
        scheduleRecoveryRetry(1000);
      } else {
        clearStartupCoreReady();
        setDaemonTimeoutStaleSummary("");
        clearRecoveryRetry();
        setFeedbackMessage(summarizeDashboardErrors(unique));
        if (
          !invokeRef.current
          && unique.every((error) => isAuthFailure(error))
          && !connectionDialogAutoPromptSuppressedRef.current
        ) {
          setShowConnectionDialog(true);
        }
      }
    } else {
      connectionDialogAutoPromptSuppressedRef.current = false;
      clearRecoveryRetry();
      const uniqueSecondary = [...new Set(secondaryErrors)];
      if (uniqueSecondary.length) {
        const timeoutErrors = uniqueSecondary.filter((error) => isDaemonTimeoutErrorMessage(error));
        if (timeoutErrors.length) {
          setDaemonTimeoutStaleSummary(summarizeDashboardErrors(timeoutErrors) || "IPC request timeouts detected.");
        } else {
          setDaemonTimeoutStaleSummary("");
        }
        setSecondaryAvailabilityFeedback(uniqueSecondary);
      } else {
        setDaemonTimeoutStaleSummary("");
        clearTransientFeedback();
      }
    }
  }, [
    call,
    clearStartupCoreReady,
    clearRecoveryRetry,
    clearTransientFeedback,
    readAuthToken,
    refreshDaemonState,
    refreshHealth,
    probeReadiness,
    refreshProtectedDataForStartup,
    clearDisconnectedData,
    cortexBase,
    resetStartupRetryState,
    scheduleRecoveryRetry,
    scheduleStartupRecoveryRetry,
    refreshSecondaryDataInBackground,
    setSecondaryAvailabilityFeedback,
  ]);

  const runRefreshAll = useCallback(() => {
    if (refreshAllInFlightRef.current) {
      refreshAllQueuedRef.current = true;
      return refreshAllInFlightRef.current;
    }

    let pendingRefresh = null;
    pendingRefresh = (async () => {
      do {
        refreshAllQueuedRef.current = false;
        await refreshAll();
      } while (refreshAllQueuedRef.current);
    })().finally(() => {
      if (refreshAllInFlightRef.current === pendingRefresh) {
        refreshAllInFlightRef.current = null;
      }
    });

    refreshAllInFlightRef.current = pendingRefresh;
    return pendingRefresh;
  }, [refreshAll]);

  return { ...ctx };
}
