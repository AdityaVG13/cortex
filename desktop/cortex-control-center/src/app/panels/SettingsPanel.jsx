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

export function SettingsPanel(p) {
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
          <section
            className={`panel settings-panel ${panel === "settings" ? "active" : "panel-hidden"}`}
            aria-hidden={panel === "settings" ? undefined : true}
          >
              <div className="panel-header">
                <div>
                  <span className="panel-kicker">Control Center</span>
                  <h1>Settings</h1>
                  <p className="panel-subtitle">Accessibility, motion, connection, keyboard, and local budget state.</p>
                </div>
                <button type="button" className="btn-sm" onClick={runRefreshAll}>Refresh</button>
              </div>

              <div className="settings-grid">
                <section className="settings-section" aria-labelledby="settings-accessibility">
                  <div className="settings-section-head">
                    <AppIcon name="settings" size={18} />
                    <h2 id="settings-accessibility">Accessibility</h2>
                  </div>
                  <label className="settings-row">
                    <span>
                      <strong>High contrast</strong>
                      <small>Increase text and border contrast.</small>
                    </span>
                    <input
                      type="checkbox"
                      checked={controlSettings.highContrast}
                      onChange={(event) => updateControlSetting("highContrast", event.target.checked)}
                    />
                  </label>
                  <label className="settings-row">
                    <span>
                      <strong>Keyboard hints</strong>
                      <small>Show shortcut labels.</small>
                    </span>
                    <input
                      type="checkbox"
                      checked={controlSettings.keyboardHints}
                      onChange={(event) => updateControlSetting("keyboardHints", event.target.checked)}
                    />
                  </label>
                </section>

                <section className="settings-section" aria-labelledby="settings-motion">
                  <div className="settings-section-head">
                    <AppIcon name="analytics" size={18} />
                    <h2 id="settings-motion">Appearance & Motion</h2>
                  </div>
                  <label className="settings-row">
                    <span>
                      <strong>Motion</strong>
                      <small>Runtime transition profile.</small>
                    </span>
                    <select
                      value={controlSettings.reducedMotion}
                      onChange={(event) => updateControlSetting("reducedMotion", event.target.value)}
                    >
                      <option value="system">System</option>
                      <option value="reduce">Reduced</option>
                      <option value="full">Full</option>
                    </select>
                  </label>
                  <label className="settings-row">
                    <span>
                      <strong>Currency</strong>
                      <small>Token-savings estimates.</small>
                    </span>
                    <select
                      value={safeCurrency}
                      onChange={(event) => setCurrency(normalizeCurrencyCode(event.target.value))}
                    >
                      {CURRENCY_OPTIONS.map((code) => <option key={code} value={code}>{code}</option>)}
                    </select>
                  </label>
                </section>

                <section className="settings-section settings-section-wide" aria-labelledby="settings-connection">
                  <div className="settings-section-head">
                    <AppIcon name="outbound" size={18} />
                    <h2 id="settings-connection">Connection</h2>
                  </div>
                  <div className="settings-status-grid">
                    <div>
                      <span className="settings-label">Host</span>
                      <strong>{formatDaemonEndpoint(cortexBase)}</strong>
                    </div>
                    <div>
                      <span className="settings-label">Mode</span>
                      <strong>{isTauriRuntime ? "Desktop managed" : "Browser attach"}</strong>
                    </div>
                    <div>
                      <span className="settings-label">Daemon</span>
                      <strong>{daemonStatusBadge.label}</strong>
                    </div>
                  </div>
                  <button type="button" className="btn-sm" onClick={openConnectionDialog}>Connection Settings</button>
                </section>

                <section className="settings-section settings-section-wide" aria-labelledby="settings-budgets">
                  <div className="settings-section-head">
                    <AppIcon name="token" size={18} />
                    <h2 id="settings-budgets">Budgets</h2>
                    <span className={`settings-budget-pill ${budgetSummary.statusLabel.toLowerCase()}`}>
                      {budgetSummary.statusLabel}
                    </span>
                  </div>
                  <div className="settings-status-grid">
                    <div>
                      <span className="settings-label">Config</span>
                      <strong>{budgetSummary.configLoaded ? "Loaded" : "Not loaded"}</strong>
                    </div>
                    <div>
                      <span className="settings-label">Enforcement</span>
                      <strong>{budgetSummary.enabled ? "Enabled" : "Off"}</strong>
                    </div>
                    <div>
                      <span className="settings-label">Source</span>
                      <strong>{budgetSummary.source || "Default unlimited"}</strong>
                    </div>
                    <div>
                      <span className="settings-label">Recent Denials</span>
                      <strong>{budgetSummary.recentDenialsTotal}</strong>
                    </div>
                  </div>
                  {budgetSummary.error ? <p className="settings-error" role="alert">{budgetSummary.error}</p> : null}
                  <div className="settings-budget-table-wrap">
                    <table className="settings-budget-table">
                      <caption className="sr-only">Configured budget endpoints</caption>
                      <thead>
                        <tr>
                          <th scope="col">Endpoint</th>
                          <th scope="col">Limit</th>
                          <th scope="col">Window</th>
                          <th scope="col">Recent Denials</th>
                        </tr>
                      </thead>
                      <tbody>
                    {(budgetSummary.endpointRows.length ? budgetSummary.endpointRows : [{ endpoint: "none", limit: null, windowSeconds: null }]).map((row) => {
                      const denial = budgetSummary.denialRows.find((entry) => entry.endpoint === row.endpoint)?.count;
                      return (
                        <tr key={row.endpoint}>
                          <th scope="row" data-label="Endpoint">{row.endpoint}</th>
                          <td data-label="Limit">{row.limit ?? "--"}</td>
                          <td data-label="Window">{row.windowSeconds ? `${row.windowSeconds}s` : "--"}</td>
                          <td data-label="Recent Denials">{denial ?? (budgetSummary.denialRows.length ? 0 : "--")}</td>
                        </tr>
                      );
                    })}
                      </tbody>
                    </table>
                  </div>
                  <form className="settings-budget-editor" onSubmit={saveBudgetConfigDraft}>
                    <div className="settings-budget-editor-head">
                      <label className="settings-row settings-budget-defaults">
                        <span>
                          <strong>Enforce budgets</strong>
                          <small>Writes the local operator budget config.</small>
                        </span>
                        <input
                          type="checkbox"
                          checked={budgetDraft.enabled}
                          disabled={!ipcAvailable || budgetConfigBusy}
                          onChange={(event) => updateBudgetDraftRoot({ enabled: event.target.checked })}
                        />
                      </label>
                      <div className="settings-budget-actions">
                        <button
                          type="button"
                          className="btn-sm"
                          disabled={!ipcAvailable || budgetConfigBusy}
                          onClick={() => reloadBudgetConfigDraft()}
                        >
                          Reload
                        </button>
                        <button
                          type="submit"
                          className="btn-sm btn-primary"
                          disabled={!ipcAvailable || budgetConfigBusy || Boolean(budgetDraftError)}
                        >
                          {budgetConfigBusy ? "Saving..." : "Save"}
                        </button>
                      </div>
                    </div>
                    <div className="settings-budget-edit-grid" role="group" aria-label="Budget endpoint editor">
                      {BUDGET_ENDPOINT_DEFINITIONS.map((definition) => {
                        const draft = budgetDraftEndpoints[definition.key];
                        const endpointEnabled = Boolean(draft?.enabled);
                        return (
                          <fieldset
                            key={definition.key}
                            className="settings-budget-edit-row"
                            disabled={!ipcAvailable || budgetConfigBusy}
                          >
                            <legend>{definition.label}</legend>
                            <label className="settings-budget-enable">
                              <input
                                type="checkbox"
                                checked={endpointEnabled}
                                onChange={(event) => updateBudgetEndpointDraft(definition.key, { enabled: event.target.checked })}
                              />
                              <span>Limited</span>
                            </label>
                            <label className="settings-budget-input">
                              <span>Calls</span>
                              <input
                                type="number"
                                min="1"
                                step="1"
                                inputMode="numeric"
                                value={draft?.limit ?? ""}
                                disabled={!endpointEnabled}
                                onChange={(event) => updateBudgetEndpointDraft(definition.key, { limit: event.target.value })}
                              />
                            </label>
                            <label className="settings-budget-input">
                              <span>Window</span>
                              <input
                                type="number"
                                min="1"
                                step="1"
                                inputMode="numeric"
                                value={draft?.windowSeconds ?? ""}
                                disabled={!endpointEnabled}
                                onChange={(event) => updateBudgetEndpointDraft(definition.key, { windowSeconds: event.target.value })}
                              />
                            </label>
                          </fieldset>
                        );
                      })}
                    </div>
                    {!ipcAvailable ? (
                      <p className="settings-budget-note">Budget edits require the desktop app.</p>
                    ) : null}
                    {budgetDraftError ? <p className="settings-error" role="alert">{budgetDraftError}</p> : null}
                    {budgetConfigMessage ? <p className="settings-budget-note" role="status">{budgetConfigMessage}</p> : null}
                  </form>
                </section>

                <section className="settings-section" aria-labelledby="settings-keyboard">
                  <div className="settings-section-head">
                    <AppIcon name="work" size={18} />
                    <h2 id="settings-keyboard">Keyboard & Navigation</h2>
                  </div>
                  <label className="settings-row">
                    <span>
                      <strong>Compact navigation</strong>
                      <small>Denser sidebar controls.</small>
                    </span>
                    <input
                      type="checkbox"
                      checked={controlSettings.compactNavigation}
                      onChange={(event) => updateControlSetting("compactNavigation", event.target.checked)}
                    />
                  </label>
                  {controlSettings.keyboardHints ? (
                    <div className="settings-shortcut-grid" role="group" aria-label="Keyboard shortcut summary">
                      {PANEL_SEQUENCE.map((item, idx) => (
                        <span key={item.key}><kbd>{idx + 1}</kbd>{item.label}</span>
                      ))}
                    </div>
                  ) : null}
                </section>
              </div>
            </section>
    </>
  );
}
