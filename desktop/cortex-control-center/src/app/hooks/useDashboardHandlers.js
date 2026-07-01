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

export function useDashboardHandlers(ctx) {
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
    handleTaskClaim,
    handleTaskAbandon,
    handleTaskComplete,
    handleTaskDelete,
    handleUnlock,
    handleSendMessage,
    handleFeedAck,
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
    operationRows,
    operationMaxSaved,
    topSavingsByAgent,
  } = ctx;

  async function handleMemorySearch(e) {
    e?.preventDefault();
    if (!memoryQuery.trim()) return;
    setMemorySearching(true);
    try {
      // `/peek` is protected on real daemons; require auth in both IPC and HTTP fallback paths.
      const peekResult = await api(`/peek?q=${encodeURIComponent(memoryQuery.trim())}&k=15`, true);
      setMemoryResults(peekResult?.matches || []);
    } catch {
      setMemoryResults([]);
    }
    setMemorySearching(false);
  }

  async function handleMemoryExpand(source) {
    try {
      // `/recall` may be protected depending on daemon policy; keep expand-on-click auth-aware.
      const recallResult = await api(`/recall?q=${encodeURIComponent(source)}&k=3`, true);
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
    resetStartupRetryState();
    daemonTransitionRef.current = true;
    try {
      const result = await call("start_daemon");
      setFeedbackMessage(result.message || "Daemon start requested.");
      const reachable = await waitForDaemonReachable({ shortCircuitIfStarting: true });
      if (!reachable) {
        scheduleStartupRecoveryRetry("Daemon is still starting. Reconnect will continue automatically.");
      }
      daemonTransitionRef.current = false;
      await readAuthToken({ suppressFeedback: true });
      await runRefreshAll();
    } catch (error) {
      setFeedbackMessage(`Start failed: ${error.message || error}`);
    } finally {
      daemonTransitionRef.current = false;
    }
  }

  async function handleStopDaemon() {
    if (!invokeRef.current) return;
    resetStartupRetryState();
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
        await runRefreshAll();
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

    try {
      await runRestartDaemonSequence();
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

  useEffect(() => {
    if (!DEV_RESTART_VERIFY_ENABLED || devVerificationStartedRef.current) {
      return undefined;
    }
    devVerificationStartedRef.current = true;

    let cancelled = false;
    let completed = false;
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    const waitForCondition = async (label, check, timeoutMs = DEV_RESTART_VERIFY_TIMEOUT_MS, intervalMs = 200) => {
      const started = Date.now();
      while (!cancelled && Date.now() - started < timeoutMs) {
        const value = check();
        if (value) {
          return value;
        }
        await sleep(intervalMs);
      }
      if (cancelled) {
        throw new Error("Dev verification cancelled.");
      }
      throw new Error(`Timed out waiting for ${label}.`);
    };
        const findSessionByAgent = (agent) => sessionsRef.current.find(
          (session) => sessionMatchesAgent(session, agent)
        ) || null;
    const sessionSnapshot = (session) => {
      if (!session) return null;
      return {
        agent: String(session.agent || ""),
        description: String(session.description || ""),
        lastHeartbeat: String(session.lastHeartbeat || session.last_heartbeat || ""),
        expiresAt: String(session.expiresAt || session.expires_at || ""),
      };
    };

    const runVerification = async () => {
      const report = {
        mode: "app-dev-restart-reconnect",
        startedAt: new Date().toISOString(),
        controlCenterVersion: CONTROL_CENTER_VERSION,
        cortexBase,
        success: false,
        steps: [],
      };
      const recordStep = (name, details = {}) => {
        report.steps.push({
          name,
          at: new Date().toISOString(),
          ...details,
        });
      };

      try {
        invokeRef.current = await readTauriInvoke();
        if (!invokeRef.current) {
          throw new Error("Tauri IPC is not available for dev verification.");
        }

        setFeedbackMessage("Running dev restart/reconnect verification...");
        await runRefreshAll();
        let streamAvailable = streamConnectedAtRef.current > 0;
        if (!streamAvailable) {
          try {
            await waitForCondition(
              "the initial event stream connection",
              () => streamConnectedAtRef.current > 0,
              25000
            );
            streamAvailable = true;
          } catch {
            streamAvailable = false;
            recordStep("stream", {
              mode: "polling-fallback",
              warning: "Event stream did not connect during startup window; continuing with polling checks.",
            });
          }
        }
        if (streamAvailable) {
          recordStep("stream", {
            mode: "event-stream",
            connectedAt: new Date(streamConnectedAtRef.current).toISOString(),
          });
        }

        if (!daemonStateRef.current?.reachable) {
          const startResult = await call("start_daemon");
          recordStep("start", { message: startResult?.message || "Daemon start requested." });
          const reachable = await waitForDaemonReachable();
          if (!reachable) {
            throw new Error("Daemon did not become reachable during verification startup.");
          }
          await readAuthToken({ suppressFeedback: true });
          await runRefreshAll();
        } else {
          recordStep("start", { message: "Daemon already reachable before verification." });
        }

        const authToken = await readAuthToken({ suppressFeedback: true });
        if (!authToken) {
          throw new Error("Daemon auth token did not become available.");
        }

        const verificationAgent = `cortex-dev-verify-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
        report.agent = verificationAgent;

        const sessionEventCountBeforeBoot = streamSessionEventCountRef.current;
        const bootResult = await callMcpTool("cortex_boot", {
          agent: verificationAgent,
          model: "desktop-dev-verify",
          budget: 120,
        });
        if (streamAvailable) {
          await waitForCondition(
            "the boot session event",
            () => streamSessionEventCountRef.current > sessionEventCountBeforeBoot
          );
        }
        const bootSession = await waitForCondition(
          "the boot session in the Agents surface",
          () => findSessionByAgent(verificationAgent)
        );
        const bootSnapshot = sessionSnapshot(bootSession);
        recordStep("boot", {
          tokenEstimate: Number(bootResult?.tokenEstimate || 0),
          session: bootSnapshot,
        });

        const connectedBeforeRestart = streamConnectedAtRef.current;
        const disconnectedBeforeRestart = streamDisconnectedAtRef.current;
        const sessionEventCountBeforeReconnect = streamSessionEventCountRef.current;

        const restartResult = await runRestartDaemonSequence();
        if (restartResult?.restartSkippedExternal) {
          recordStep("restart", {
            skipped: true,
            reason: restartResult?.message || "Daemon remained online (externally managed).",
          });
        } else {
          if (streamAvailable) {
            await waitForCondition(
              "the event stream disconnect during restart",
              () => streamDisconnectedAtRef.current > disconnectedBeforeRestart
            );
            await waitForCondition(
              "the event stream reconnect after restart",
              () => streamConnectedAtRef.current > connectedBeforeRestart
            );
            recordStep("restart", {
              disconnectedAt: new Date(streamDisconnectedAtRef.current).toISOString(),
              reconnectedAt: new Date(streamConnectedAtRef.current).toISOString(),
            });
          } else {
            recordStep("restart", {
              mode: "polling-fallback",
              skippedStreamChecks: true,
              message: "Restart completed without stream lifecycle checks; polling verification continued.",
            });
          }
        }

        const reconnectResult = await callMcpTool("cortex_reconnect", {
          agent: verificationAgent,
          model: "desktop-dev-verify",
        });
        if (streamAvailable) {
          await waitForCondition(
            "the reconnect session event",
            () => streamSessionEventCountRef.current > sessionEventCountBeforeReconnect
          );
        }
        const reconnectSession = await waitForCondition(
          "the reconnected session in the Agents surface",
          () => findSessionByAgent(verificationAgent)
        );
        const reconnectSnapshot = sessionSnapshot(reconnectSession);
        if (bootSnapshot?.description && reconnectSnapshot?.description !== bootSnapshot.description) {
          throw new Error("Reconnect changed the session description shown in the Agents surface.");
        }
        recordStep("reconnect", {
          expiresAt: reconnectResult?.expiresAt || "",
          session: reconnectSnapshot,
        });

        const recallResult = await callMcpTool("cortex_recall", {
          agent: verificationAgent,
          model: "desktop-dev-verify",
          query: "restart reconnect verification",
          budget: 200,
        });
        const recallSessionsPayload = await api("/sessions", true);
        const recallSession = (Array.isArray(recallSessionsPayload?.sessions) ? recallSessionsPayload.sessions : [])
          .map((session, index) => normalizeSession(session, index))
          .find((session) => sessionMatchesAgent(session, verificationAgent)) || null;
        const recallSnapshot = sessionSnapshot(recallSession);
        if (!recallSnapshot) {
          throw new Error("Session disappeared after read-path recall refresh.");
        }
        if (bootSnapshot?.description && recallSnapshot.description !== bootSnapshot.description) {
          throw new Error("Read-path recall refresh downgraded the session description.");
        }
        recordStep("read-path-refresh", {
          resultCount: Array.isArray(recallResult?.results) ? recallResult.results.length : 0,
          session: recallSnapshot,
        });

        report.success = true;
        setFeedbackMessage("Dev restart/reconnect verification passed.");
      } catch (error) {
        const message = error?.message || String(error);
        report.error = message;
        setFeedbackMessage(`Dev verification failed: ${message}`);
      } finally {
        if (cancelled && !completed) {
          return;
        }
        report.completedAt = new Date().toISOString();
        report.finalDaemonState = {
          running: Boolean(daemonStateRef.current?.running),
          reachable: Boolean(daemonStateRef.current?.reachable),
          managed: Boolean(daemonStateRef.current?.managed),
          authTokenReady: Boolean(daemonStateRef.current?.authTokenReady),
          message: String(daemonStateRef.current?.message || ""),
        };
        try {
          report.reportPath = await writeDevVerificationReport(report);
        } catch (writeError) {
          report.reportWriteError = writeError?.message || String(writeError);
        }
        completed = true;
        await sleep(500);
        if (invokeRef.current) {
          try {
            await call("quit_app");
          } catch {
            // App is already exiting.
          }
        }
      }
    };

    runVerification();
    return () => {
      cancelled = true;
      if (!completed) {
        devVerificationStartedRef.current = false;
      }
    };
  }, [api, call, callMcpTool, cortexBase, readAuthToken, runRefreshAll, runRestartDaemonSequence, waitForDaemonReachable, writeDevVerificationReport]);

  useEffect(() => {
    if (!showConnectionDialog && !showEditorSetupWizard) {
      return undefined;
    }

    function handleDialogKey(event) {
      if (event.key === "Tab") {
        const dialog = showEditorSetupWizard ? editorSetupDialogRef.current : connectionDialogRef.current;
        trapFocusInContainer(event, dialog);
        return;
      }

      if (event.key !== "Escape") {
        return;
      }

      if (showEditorSetupWizard && !isSettingUpEditors) {
        event.preventDefault();
        closeEditorSetupWizard();
        return;
      }

      if (showConnectionDialog) {
        event.preventDefault();
        dismissConnectionDialog();
      }
    }

    window.addEventListener("keydown", handleDialogKey);
    return () => window.removeEventListener("keydown", handleDialogKey);
  }, [closeEditorSetupWizard, dismissConnectionDialog, isSettingUpEditors, showConnectionDialog, showEditorSetupWizard]);

  useEffect(() => {
    if (!showConnectionDialog) return undefined;
    const frame = window.requestAnimationFrame(() => {
      connectionDialogRef.current?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [showConnectionDialog]);

  useEffect(() => {
    if (!showEditorSetupWizard) return undefined;
    const frame = window.requestAnimationFrame(() => {
      editorSetupDialogRef.current?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [showEditorSetupWizard]);

  useEffect(() => {
    setElementInert(topbarRef.current, panel === "overview");
    setElementInert(analyticsPanelRef.current, panel !== "analytics");
    setElementInert(brainPanelRef.current, panel !== "brain");
  }, [panel]);

  // Keyboard nav
  useEffect(() => {
    function handleKey(e) {
      if (shouldIgnoreGlobalShortcut(e, showConnectionDialog || showEditorSetupWizard)) return;
      const idx = panelIndex(panel);
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
  }, [changePanel, panel, showConnectionDialog, showEditorSetupWizard]);

  const effectiveSidebarCollapsed = sidebarCollapsed || isNarrowViewport;
  const canStartDaemon = Boolean(invokeRef.current && !restartingDaemon && !daemonState.running);
  const canStopDaemon = Boolean(invokeRef.current && !restartingDaemon && (daemonState.reachable || daemonState.running));
  const canSetupEditors = Boolean(invokeRef.current && !isSettingUpEditors);
  const firstRunReadiness = useMemo(
    () => buildFirstRunReadiness({
      daemonState,
      stats,
      sessions: normalizedSessions,
      editorSetupSummary,
      healthMeta,
      canStartDaemon,
      canSetupEditors,
      isSettingUpEditors,
    }),
    [
      canSetupEditors,
      canStartDaemon,
      daemonState.reachable,
      daemonState.running,
      editorSetupSummary.registered,
      healthMeta.dbCorrupted,
      healthMeta.degraded,
      isSettingUpEditors,
      normalizedSessions.length,
      stats.decisions,
      stats.memories,
    ]
  );
  function handleFirstRunAction() {
    if (firstRunReadiness.action.disabled) return;
    switch (firstRunReadiness.action.kind) {
      case "start_daemon":
        handleStartDaemon();
        break;
      case "restart_daemon":
        handleRestartDaemon();
        break;
      case "setup_mcp":
        openEditorSetupWizard();
        break;
      case "open_memory":
        changePanel("memory");
        break;
      case "refresh":
      default:
        runRefreshAll();
        break;
    }
  }
  const activePanelLabel = PANEL_SEQUENCE_LABEL.get(panel) || "Overview";
  const connectionEndpoint = useMemo(() => {
    const fallback = {
      host: "127.0.0.1",
      port: "7437",
      hostLabel: cortexBase === DEFAULT_CORTEX_BASE ? "LOCAL" : "?",
    };
    try {
      const url = new URL(cortexBase);
      return {
        host: url.hostname || fallback.host,
        port: url.port || fallback.port,
        hostLabel: cortexBase === DEFAULT_CORTEX_BASE ? "LOCAL" : url.hostname || "?",
      };
    } catch {
      return fallback;
    }
  }, [cortexBase]);
  const hostLabel = connectionEndpoint.hostLabel;
  const handleAnalyticsTabKey = useCallback((event) => {
    const order = ["aggregate", "operations"];
    const currentIndex = Math.max(0, order.indexOf(analyticsMode));
    let nextIndex = null;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      nextIndex = (currentIndex + 1) % order.length;
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      nextIndex = (currentIndex - 1 + order.length) % order.length;
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = order.length - 1;
    }
    if (nextIndex === null) return;
    event.preventDefault();
    const nextMode = order[nextIndex];
    setAnalyticsMode(nextMode);
    window.requestAnimationFrame(() => {
      analyticsTabRefs.current[nextMode]?.focus();
    });
  }, [analyticsMode]);
  return {
    ...ctx,
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
  };
}
