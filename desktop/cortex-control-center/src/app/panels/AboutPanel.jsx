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

export function AboutPanel(p) {
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
        {panel === "about" ? (
          <section className="panel active">
            <div className="panel-header">
              <div>
                <h1>About</h1>
                <p className="panel-subtitle">Shipping surface, runtime contract, and contributor credits for Cortex Control Center.</p>
              </div>
            </div>
            <div className="card full">
              <div className="about-content">
                <div className="about-brand">
                  <img
                    src={`${import.meta.env.BASE_URL}icons/icon.png`}
                    alt="Cortex"
                    className="about-logo"
                    onError={(event) => { event.currentTarget.style.display = "none"; event.currentTarget.nextSibling.style.display = "flex"; }}
                  />
                  <div className="about-logo about-logo-fallback">CC</div>
                  <div className="about-heading">
                    <h2 className="about-title">Cortex Control Center</h2>
                    <p className="about-version">Built by the Cortex maintainer team -- Version {CONTROL_CENTER_VERSION}</p>
                  </div>
                </div>

                <p className="about-description">
                  A desktop command surface for Cortex built around one app-managed daemon instance:
                  auth-aware startup, owned lifecycle control, live telemetry, and a brain view that can double as a showpiece.
                </p>

                <div className="about-stats-grid">
                  {[
                    ["Daemon", "Rust + Axum"],
                    ["Desktop shell", "Tauri + React"],
                    ["Embeddings", "ONNX (all-MiniLM-L6-v2)"],
                    ["Storage", "SQLite (WAL)"],
                    ["Transport", "HTTP + MCP stdio"],
                    ["Port", "7437"],
                  ].map(([label, value]) => (
                    <div key={label} className="about-stat-card">
                      <span className="about-stat-label">{label}</span>
                      <div className="about-stat-value">{value}</div>
                    </div>
                  ))}
                </div>

                <div className="about-section">
                  <h3 className="about-section-title">App Lifecycle</h3>
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
                        <td>Launches the app-managed Cortex daemon and waits for a healthy API before reloading data.</td>
                      </tr>
                      <tr>
                        <td>Stop</td>
                        <td>Sends a graceful shutdown request to the app-managed daemon, then clears owned process handles.</td>
                      </tr>
                      <tr>
                        <td>Restart</td>
                        <td>Runs Stop then Start with timeout handling so the UI can recover from stale daemon state without creating a second instance.</td>
                      </tr>
                      <tr>
                        <td>Close Window</td>
                        <td>Minimizes to tray by default so the app-managed daemon can keep serving local clients in the background.</td>
                      </tr>
                      <tr>
                        <td>Exit</td>
                        <td>Fully quits the app and requests daemon shutdown when this app instance owns it.</td>
                      </tr>
                    </tbody>
                  </table>
                </div>

                <div className="about-section">
                  <h3 className="about-section-title">Contributors</h3>
                  <div className="about-contributors">
                    {[
                      { handle: "Cortex-Team", role: "Creator & maintainer" },
                      { handle: "Claude Code", role: "Core architecture & retrieval pipeline" },
                      { handle: "Factory Droid", role: "Desktop app, reconnection & telemetry" },
                      { handle: "Codex", role: "Desktop rewrite, auth hardening, analytics and brain UX" },
                    ].map(({ handle, role }) => (
                      <div key={handle} className="about-contributor">
                        <span className="agent-indicator" style={{ background: "var(--cyan)", boxShadow: "0 0 8px var(--cyan)" }} />
                        <span className="about-contributor-handle">@{handle}</span>
                        <span className="about-contributor-role">{role}</span>
                      </div>
                    ))}
                  </div>
                </div>
              </div>
            </div>
          </section>
        ) : null}
    </>
  );
}
