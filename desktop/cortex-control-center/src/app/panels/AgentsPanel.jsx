import { AppIcon } from "../../ui-icons.jsx";
import { CURRENCY_OPTIONS, SAVINGS_OPERATION_LABELS, timeAgo } from "../../constants.js";
import { SAVINGS_USD_PER_MILLION, SAVINGS_HISTORY_DAYS, MISSION_METRIC_LEGEND, CONTROL_CENTER_VERSION, ANALYTICS_METRIC_LEGEND } from "../constants.js";
import { BUDGET_ENDPOINT_DEFINITIONS } from "../../settings/settings-state.js";
import { handleKeyboardActivation } from "../../keyboard-access.js";
import { sameAgent } from "../../live-surface.js";
import { normalizeCurrencyCode, formatDaemonEndpoint } from "../utils/format.js";
import { conflictBadgeClass } from "../normalize/conflicts.js";
import { agentColor } from "../utils/agent-color.js";
import { AnimatedNumber } from "../components/AnimatedNumber.jsx";
import { Sparkline } from "../components/Sparkline.jsx";
import { MonteCarloProjectionChart } from "../components/MonteCarloProjectionChart.jsx";
import { EmptyItem } from "../components/common.jsx";
import { AgentItem } from "../components/AgentItem.jsx";
import { OperatorSelector } from "../components/OperatorSelector.jsx";
import { TaskItem } from "../components/TaskItem.jsx";
import { LockItem } from "../components/LockItem.jsx";
import { FeedItem } from "../components/FeedItem.jsx";
import { MessageItem } from "../components/MessageItem.jsx";
import { ActivityItem } from "../components/ActivityItem.jsx";
import { ConflictPairCard } from "../components/ConflictPairCard.jsx";
import { PANEL_SEQUENCE } from "../constants.js";

export function AgentsPanel(p) {
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
  } = p;

  return (
    <>
        {panel === "agents" ? (
          <section className="panel active">
            <div className="panel-header">
              <div>
                <h1>Agents</h1>
                <p className="panel-subtitle">Sessions, messages, and recent activity in one place.</p>
              </div>
              <div className="surface-actions">
                <button type="button" className="btn-sm" onClick={runRefreshAll}>Refresh</button>
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
    </>
  );
}
