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

export function useSseStream(ctx) {
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
      const token = tokenRef.current;
      if (!token) return;
      const streamUrl = `${cortexBase}/events/stream?token=${encodeURIComponent(token)}`;
      const nextStream = new EventSource(streamUrl);
      stream = nextStream;

      nextStream.onopen = () => {
        reconnectAttempt = 0;
        streamConnectedAtRef.current = Date.now();
        scheduleRefresh(true);
      };

      nextStream.onmessage = handleRealtimeEvent;
      nextStream.addEventListener("connected", handleRealtimeEvent);
      nextStream.addEventListener("task", handleRealtimeEvent);
      nextStream.addEventListener("session", () => {
        streamSessionEventCountRef.current += 1;
        handleRealtimeEvent();
      });
      nextStream.addEventListener("lock", handleRealtimeEvent);
      nextStream.addEventListener("feed", handleRealtimeEvent);
      nextStream.addEventListener("message", handleRealtimeEvent);
      nextStream.addEventListener("activity", handleRealtimeEvent);

      nextStream.onerror = () => {
        if (disposed || stream !== nextStream) return;
        streamDisconnectedAtRef.current = Date.now();
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
  }, [cortexBase, daemonState.authTokenReady]);
  return ctx;
}
