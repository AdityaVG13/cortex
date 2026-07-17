import { useCallback, useEffect, useMemo } from "react";
import { checkForUpdates } from "../../updater.js";
import { MOTION_MS } from "../../design/motion.js";
import { summarizeDashboardErrors } from "../../api-client.js";
import { nextFeedAckId, sameAgent } from "../../live-surface.js";
import { daemonStatusPill, daemonSystemStatus, daemonUtilityPill, isDaemonStartingState } from "../../daemon-startup.js";
import { buildMonteCarloProjection } from "../../analytics-projection.js";
import { summarizeBootThroughput } from "../../analytics-metrics.js";
import { createBudgetDraftFromStatus, writeControlCenterSettings } from "../../settings/settings-state.js";
import { ANALYTICS_REFRESH_MS, CONTROL_CENTER_VERSION, CORTEX_BASE_STORAGE_KEY, CORTEX_OPERATOR_STORAGE_KEY, CORTEX_PANEL_STORAGE_KEY, DEFAULT_CORTEX_BASE, FALLBACK_REFRESH_MS, RECALL_HEADLINE_MIN_QUERIES, SIDEBAR_COLLAPSE_BREAKPOINT_PX } from "../constants.js";
import { persistBrowserAuthToken } from "../browser-bootstrap.js";
import { formatDaemonEndpoint, priorityRank } from "../utils/format.js";
import { isDaemonSuppressibleErrorMessage } from "../utils/daemon.js";

export function useDashboardEffects(ctx) {
  const {
    panel,
    daemonState,
    healthMeta,
    tasks,
    locks,
    feedEntries,
    activityEntries,
    savings,
    selectedOperator,
    setSelectedOperator,
    messageTarget,
    setMessageTarget,
    messageDraft,
    setMessageDraft,
    setTaskCompletionDrafts,
    setCompletionTaskId,
    daemonTimeoutStaleSummary,
    cortexBase,
    setCortexBase,
    setFeedbackMessage,
    hasVisitedAnalytics,
    analyticsReady,
    controlSettings,
    budgetConfigStatus,
    budgetDraftDirty,
    budgetConfigBusy,
    ipcAvailable,
    analyticsMode,
    effectiveReducedMotion,
    refreshAllRef,
    tokenRef,
    isTauriRuntime,
    normalizedSessions,
    knownAgents,
    selectedOperatorName,
    messageTargetName,
    safeCurrency,
    runRefreshAll,
    reloadBudgetConfigDraft,
    refreshMessages,
    refreshActivity,
    refreshFeed,
    refreshSavings,
    postApi,
    activeBudgetStatus,
    budgetConfigLoadAttemptedRef,
    setOsReducedMotion,
    setIsNarrowViewport,
    setHasVisitedAnalytics,
    setAnalyticsReady,
    clearRecoveryRetry,
    setAvailableUpdate,
    skipInitialFeedRefreshRef,
    skipInitialMessagesRefreshRef,
    skipInitialActivityRefreshRef,
    startupCoreReadyState,
    setBusyActionKey,
    refreshCoreData,
  } = ctx;

  useEffect(() => {
    localStorage.setItem(CORTEX_BASE_STORAGE_KEY, cortexBase);
    refreshAllRef.current();
  }, [cortexBase]);

  useEffect(() => {
    if (!isTauriRuntime) {
      return;
    }
    if (cortexBase !== DEFAULT_CORTEX_BASE) {
      setCortexBase(DEFAULT_CORTEX_BASE);
    }
    if (tokenRef.current) {
      tokenRef.current = "";
      persistBrowserAuthToken("");
    }
  }, [cortexBase, isTauriRuntime]);

  useEffect(() => {
    localStorage.setItem("cortex_currency", safeCurrency);
  }, [safeCurrency]);

  useEffect(() => {
    if (budgetDraftDirty) return;
    setBudgetDraft(createBudgetDraftFromStatus(activeBudgetStatus));
  }, [activeBudgetStatus, budgetDraftDirty]);

  useEffect(() => {
    if (
      panel !== "settings"
      || !ipcAvailable
      || budgetConfigStatus
      || budgetConfigBusy
      || budgetConfigLoadAttemptedRef.current
    ) {
      return;
    }
    const budgetReloadTimer = window.setTimeout(() => {
      reloadBudgetConfigDraft({ silent: true });
    }, effectiveReducedMotion ? 0 : MOTION_MS.panel);

    return () => {
      window.clearTimeout(budgetReloadTimer);
    };
  }, [
    budgetConfigBusy,
    budgetConfigStatus,
    effectiveReducedMotion,
    ipcAvailable,
    panel,
    reloadBudgetConfigDraft,
  ]);

  useEffect(() => {
    writeControlCenterSettings(controlSettings);
    if (typeof document === "undefined") return;
    document.documentElement.dataset.cortexReducedMotion = controlSettings.reducedMotion;
    document.documentElement.dataset.cortexEffectiveReducedMotion = effectiveReducedMotion ? "reduce" : "full";
    document.documentElement.dataset.cortexContrast = controlSettings.highContrast ? "high" : "standard";
    document.documentElement.dataset.cortexKeyboardHints = controlSettings.keyboardHints ? "on" : "off";
    document.documentElement.dataset.cortexCompactNavigation = controlSettings.compactNavigation ? "on" : "off";
  }, [controlSettings, effectiveReducedMotion]);

  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
      return undefined;
    }
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    const syncReducedMotion = () => setOsReducedMotion(Boolean(query.matches));
    syncReducedMotion();
    if (typeof query.addEventListener === "function") {
      query.addEventListener("change", syncReducedMotion);
      return () => query.removeEventListener("change", syncReducedMotion);
    }
    query.addListener?.(syncReducedMotion);
    return () => query.removeListener?.(syncReducedMotion);
  }, []);

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
    refreshAllRef.current = runRefreshAll;
  }, [runRefreshAll]);

  useEffect(() => () => {
    clearRecoveryRetry();
  }, [clearRecoveryRetry]);

  useEffect(() => {
    // Call refreshAll directly on mount -- refreshAllRef.current isn't assigned
    // yet when this effect fires (ref-assignment effect hasn't run).
    runRefreshAll();
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
    if (skipInitialFeedRefreshRef.current) {
      skipInitialFeedRefreshRef.current = false;
      return;
    }
    refreshFeed().catch((error) => {
      const message = error?.message || String(error);
      if (!message || isDaemonSuppressibleErrorMessage(message)) return;
      setFeedbackMessage(summarizeDashboardErrors([message]) || message);
    });
  }, [refreshFeed]);

  useEffect(() => {
    if (skipInitialMessagesRefreshRef.current) {
      skipInitialMessagesRefreshRef.current = false;
      return;
    }
    refreshMessages().catch((error) => {
      const message = error?.message || String(error);
      if (!message || isDaemonSuppressibleErrorMessage(message)) return;
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
      if (!message || isDaemonSuppressibleErrorMessage(message)) return;
      setFeedbackMessage(summarizeDashboardErrors([message]) || message);
    });
  }, [refreshActivity]);

  useEffect(() => {
    if (
      panel !== "analytics"
      || !analyticsReady
      || !daemonState.reachable
      || !daemonState.authTokenReady
      || !startupCoreReadyState
    ) return;
    refreshSavings().catch((error) => {
      const message = error?.message || String(error);
      if (!message || isDaemonSuppressibleErrorMessage(message)) return;
      setFeedbackMessage(summarizeDashboardErrors([message]) || message);
    });
    const timer = setInterval(() => {
      refreshSavings().catch((error) => {
        const message = error?.message || String(error);
        if (!message || isDaemonSuppressibleErrorMessage(message)) return;
        setFeedbackMessage(summarizeDashboardErrors([message]) || message);
      });
    }, ANALYTICS_REFRESH_MS);
    return () => clearInterval(timer);
  }, [analyticsReady, daemonState.authTokenReady, daemonState.reachable, panel, refreshSavings, startupCoreReadyState]);

  const pendingTasks = useMemo(
    () => tasks.filter((task) => task.status === "pending").sort((a, b) => priorityRank(b.priority) - priorityRank(a.priority)),
    [tasks]
  );
  const claimedTasks = useMemo(() => tasks.filter((task) => task.status === "claimed"), [tasks]);
  const completedTasks = useMemo(() => tasks.filter((task) => task.status === "completed"), [tasks]);
  const recentOverviewTasks = useMemo(() => [...claimedTasks, ...pendingTasks].slice(0, 5), [claimedTasks, pendingTasks]);
  const pill = daemonStatusPill(daemonState);
  const utilityPill = useMemo(
    () => daemonUtilityPill(daemonState),
    [daemonState.reachable, daemonState.running]
  );
  const daemonSysStatus = useMemo(
    () => daemonSystemStatus(daemonState),
    [daemonState.reachable, daemonState.running]
  );

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

  const cumulativeLatestTotal = useMemo(
    () => Number(cumulativeSeries.at(-1)?.savedTotal || 0),
    [cumulativeSeries]
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

  const throughputSummary = useMemo(
    () => summarizeBootThroughput(dailySeries, 7),
    [dailySeries]
  );

  const throughputBoots7d = throughputSummary.boots;
  const throughputAvgPerDay7d = throughputSummary.avgPerDay;

  const throughputBoots30d = useMemo(
    () => Number(savings?.summary?.totalBoots || 0),
    [savings]
  );

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

  const topSavingsByAgent = useMemo(() => {
    const rows = Array.isArray(savings?.byAgent) ? savings.byAgent : [];
    return [...rows]
      .sort((a, b) => Number(b.saved || 0) - Number(a.saved || 0))
      .slice(0, 8);
  }, [savings?.byAgent]);

  const sidebarUtilityStats = useMemo(
    () => [
      { label: "Queue", value: pendingTasks.length, tone: pendingTasks.length ? "warning" : "calm" },
      { label: "Locks", value: locks.length, tone: locks.length ? "cyan" : "calm" },
      { label: "Recall", value: `${latestRecallHitRate || 0}%`, tone: latestRecallHitRate >= 85 ? "green" : "warning" },
      { label: "Agents", value: normalizedSessions.length, tone: normalizedSessions.length ? "cyan" : "calm" },
    ],
    [pendingTasks.length, locks.length, latestRecallHitRate, normalizedSessions.length]
  );

  const runtimeVersionMismatch = useMemo(
    () => Boolean(healthMeta.runtimeVersion) && healthMeta.runtimeVersion !== CONTROL_CENTER_VERSION,
    [healthMeta.runtimeVersion]
  );

  const daemonStarting = useMemo(
    () => isDaemonStartingState(daemonState),
    [daemonState.reachable, daemonState.running]
  );

  const daemonStatusBadge = useMemo(() => {
    if (daemonStarting) {
      return {
        className: "warning",
        label: "◌ STARTING",
        title: daemonState.message || "Cortex daemon process is running but not reachable yet.",
      };
    }
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
    if (daemonTimeoutStaleSummary) {
      return {
        className: "warning",
        label: "▲ STALE",
        title: `Daemon reachable, but recent IPC requests timed out. ${daemonTimeoutStaleSummary}`,
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
  }, [cortexBase, daemonStarting, daemonState.message, daemonState.reachable, daemonTimeoutStaleSummary, healthMeta.dbCorrupted, healthMeta.degraded]);

  const daemonRecoveryHint = useMemo(() => {
    if (daemonStarting) {
      return "Daemon process is up but still initializing. Control Center will keep retrying with bounded backoff.";
    }
    if (!daemonState.reachable) {
      return "";
    }
    if (healthMeta.dbCorrupted) {
      return "Database integrity checks are failing. Restart Cortex to trigger repair and inspect the daemon if it stays degraded.";
    }
    if (daemonTimeoutStaleSummary) {
      return "Daemon is reachable, but recent IPC requests timed out. Core and panel data may be temporarily stale.";
    }
    if (runtimeVersionMismatch) {
      return `Connected to daemon v${healthMeta.runtimeVersion}. Restart from Control Center to switch to v${CONTROL_CENTER_VERSION}.`;
    }
    if (healthMeta.degraded) {
      return "Semantic search is using keyword fallback right now. Restart Cortex if this state does not clear.";
    }
    return "";
  }, [daemonStarting, daemonState.reachable, daemonTimeoutStaleSummary, healthMeta.dbCorrupted, healthMeta.degraded, healthMeta.runtimeVersion, runtimeVersionMismatch]);

  const reportSurfaceError = useCallback((error) => {
    const message = error?.message || String(error);
    if (!message || isDaemonSuppressibleErrorMessage(message)) return;
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

  return {
    ...ctx,
    pendingTasks,
    claimedTasks,
    completedTasks,
    recentOverviewTasks,
    utilityPill,
    daemonSysStatus,
    operationRows,
    operationMaxSaved,
    dailySeries,
    cumulativeSeries,
    cumulativeLatestTotal,
    recallTrendSeries,
    activityHeatmap,
    activityHeatmapLookup,
    activityHeatmapMax,
    bootSavingsMomentum,
    throughputSummary,
    throughputBoots30d,
    recentRecallWindow,
    latestRecallPoint,
    stableRecallHeadlinePoint,
    latestRecallHitRate,
    latestRecallSampleSize,
    recallHeadlineUsesFallback,
    recallWindowAverage,
    recallWindowSpread,
    monteCarloProjection,
    topFeedEntries,
    topActivityEntries,
    topSavingsByAgent,
    sidebarUtilityStats,
    runtimeVersionMismatch,
    daemonStarting,
    daemonStatusBadge,
    daemonRecoveryHint,
    reportSurfaceError,
    handleTaskClaim,
    handleTaskAbandon,
    handleTaskComplete,
    handleTaskDelete,
    handleUnlock,
    handleSendMessage,
    handleFeedAck,
  };
}
