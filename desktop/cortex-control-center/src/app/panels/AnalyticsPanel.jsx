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

export function AnalyticsPanel(p) {
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
        {panel === "analytics" || hasVisitedAnalytics ? (
          <section
            ref={analyticsPanelRef}
            className={`panel analytics-panel ${panel === "analytics" ? "active" : "panel-hidden"}`}
            aria-hidden={panel === "analytics" ? undefined : true}
          >
            <div className="analytics-panel-header">
              <div className="analytics-header-copy">
                <span className="analytics-kicker">Cortex / Analytics</span>
                <h1>Compounding Memory Economics</h1>
                <p>
                  Track how Cortex turns raw recall pressure into a smaller boot prompt, compounding token savings over time instead of replaying the whole brain on every boot.
                </p>
              </div>
              <div className="analytics-toolbar">
                <span className="panel-subtitle">Token savings and brain health</span>
                <label className="analytics-inline-control">
                  <span>Currency</span>
                  <select value={safeCurrency} onChange={(event) => setCurrency(normalizeCurrencyCode(event.target.value))}>
                    {CURRENCY_OPTIONS.map((code) => (
                      <option key={code} value={code}>{code}</option>
                    ))}
                  </select>
                </label>
                <div className="analytics-view-toggle" role="tablist" aria-label="Analytics view mode">
                  <button
                    id="analytics-tab-aggregate"
                    ref={(element) => { analyticsTabRefs.current.aggregate = element; }}
                    type="button"
                    role="tab"
                    aria-selected={analyticsMode === "aggregate"}
                    tabIndex={analyticsMode === "aggregate" ? 0 : -1}
                    className={`btn-sm ${analyticsMode === "aggregate" ? "btn-primary" : ""}`}
                    onClick={() => setAnalyticsMode("aggregate")}
                    onKeyDown={handleAnalyticsTabKey}
                  >
                    Aggregate
                  </button>
                  <button
                    id="analytics-tab-operations"
                    ref={(element) => { analyticsTabRefs.current.operations = element; }}
                    type="button"
                    role="tab"
                    aria-selected={analyticsMode === "operations"}
                    tabIndex={analyticsMode === "operations" ? 0 : -1}
                    className={`btn-sm ${analyticsMode === "operations" ? "btn-primary" : ""}`}
                    onClick={() => setAnalyticsMode("operations")}
                    onKeyDown={handleAnalyticsTabKey}
                  >
                    By Operation
                  </button>
                </div>
                <button type="button" className="btn-sm" onClick={() => refreshSavings().catch(reportSurfaceError)}>
                  Refresh
                </button>
              </div>
            </div>
            {!analyticsReady ? (
              <div className="card full analytics-loading-card">
                <EmptyItem text="Preparing analytics surface..." />
              </div>
            ) : savings ? (
              <>
                <div className="analytics-metric-legend" role="group" aria-label="Analytics metric legend">
                  {ANALYTICS_METRIC_LEGEND.map((entry) => (
                    <div key={entry.label} className="analytics-metric-legend-item">
                      <span className="analytics-metric-legend-label">{entry.label}</span>
                      <span className="analytics-metric-legend-value">{entry.meaning}</span>
                    </div>
                  ))}
                </div>
                <div className="analytics-assumption-note">{savingsEstimateLegend}</div>
                <div className="analytics-metrics-grid">
                  <div className="metric metric-featured" data-accent="cyan">
                    <span className="metric-kicker">Compounding return</span>
                    <span className="metric-value"><AnimatedNumber value={savings.summary?.totalSaved || 0} duration={MOTION_MS.numberSlow} reducedMotion={effectiveReducedMotion} /></span>
                    <span className="metric-label">Boot Tokens Saved (30d total)</span>
                    <span className="metric-footnote">
                      {bootSavingsMomentum === null
                        ? "Rolling 30-day total tokens saved across boot compilations. Momentum appears after at least 8 daily samples."
                        : `Rolling 30-day total tokens saved across boot compilations, momentum ${bootSavingsMomentum >= 0 ? "+" : ""}${bootSavingsMomentum}% vs prior 4-day window.`}
                    </span>
                    <span className="metric-icon"><AppIcon name="savings" /></span>
                  </div>
                  <div className="metric" data-accent="green">
                    <span className="metric-kicker">Efficiency</span>
                    <span className="metric-value"><AnimatedNumber value={savings.summary?.avgPercent || 0} reducedMotion={effectiveReducedMotion} />%</span>
                    <span className="metric-label">30d Avg Compression</span>
                    <span className="metric-footnote">
                      Average tokens saved per boot: {formatCompactNumber(Number(savings.summary?.avgSavedPerBoot || 0))} over the same 30-day window
                    </span>
                    <span className="metric-icon"><AppIcon name="efficiency" /></span>
                  </div>
                  <div className="metric" data-accent="blue">
                    <span className="metric-kicker">Throughput</span>
                    <span className="metric-value"><AnimatedNumber value={throughputBoots7d} reducedMotion={effectiveReducedMotion} /></span>
                    <span className="metric-label">Boot Compilations (last 7 calendar days)</span>
                    <span className="metric-footnote">
                      {throughputSummary.daysRepresented === 0
                        ? "No boot compilations recorded in the last 7 calendar days."
                        : throughputSummary.isPartialHistory
                          ? `Since first recorded boot (${throughputSummary.daysRepresented} day${throughputSummary.daysRepresented === 1 ? "" : "s"}, ~${throughputAvgPerDay7d}/day).`
                          : `Last 7 calendar days (~${throughputAvgPerDay7d}/day).`}
                    </span>
                    <span className="metric-icon"><AppIcon name="refresh" /></span>
                  </div>
                  <div className="metric" data-accent="purple">
                    <span className="metric-kicker">Compiled context</span>
                    <span className="metric-value"><AnimatedNumber value={savings.summary?.totalServed || 0} duration={MOTION_MS.numberSlow} reducedMotion={effectiveReducedMotion} /></span>
                    <span className="metric-label">Boot Prompt Tokens Served (30d total)</span>
                    <span className="metric-footnote">
                      30-day cumulative prompt tokens served at boot; average {formatCompactNumber(Number(savings.summary?.avgServedPerBoot || 0))} per boot
                    </span>
                    <span className="metric-icon"><AppIcon name="outbound" /></span>
                  </div>
                  <div className="metric" data-accent="green">
                    <span className="metric-kicker">Economic value</span>
                    <span className="metric-value">{formatCurrency(((savings.summary?.totalSaved || 0) * SAVINGS_USD_PER_MILLION) / 1000000)}</span>
                    <span className="metric-label">Est. {safeCurrency} Saved</span>
                    <span className="metric-footnote">
                      Derived from {formatCompactNumber(Number(savings.summary?.totalSaved || 0))} total tokens saved across boot compilations in the last 30 days
                    </span>
                    <span className="metric-icon">$</span>
                  </div>
                </div>

                {analyticsMode === "aggregate" ? (
                  <div
                    id="analytics-tabpanel-aggregate"
                    className="analytics-mode-panel"
                    role="tabpanel"
                    aria-labelledby="analytics-tab-aggregate"
                    tabIndex={0}
                  >
                    <div className="analytics-explainer analytics-explainer-rich">
                      <div className="analytics-explainer-title">How to read this</div>
                      <p>
                        Cortex compiles a budgeted boot prompt instead of replaying raw memory. <code>baseline</code> is estimated raw context load, <code>served</code> is the compiled prompt, and <code>saved</code> is the difference. Aggregate mode shows the compounding system view. By Operation isolates where those savings come from.
                      </p>
                      <div className="analytics-stat-strip">
                        <div className="analytics-stat-chip">
                          <span className="analytics-stat-chip-label">Avg raw per boot</span>
                          <strong>{formatCompactNumber(Number(savings.summary?.avgBaselinePerBoot || 0))}t</strong>
                        </div>
                        <div className="analytics-stat-chip">
                          <span className="analytics-stat-chip-label">Avg served per boot</span>
                          <strong>{formatCompactNumber(Number(savings.summary?.avgServedPerBoot || 0))}t</strong>
                        </div>
                        <div className="analytics-stat-chip">
                          <span className="analytics-stat-chip-label">Median 30d gain</span>
                          <strong>
                            {monteCarloProjection
                              ? `${formatSignedCompactNumber(Number(monteCarloProjection.summary?.p50Gain || 0))}t`
                              : "Pending"}
                          </strong>
                        </div>
                      </div>
                    </div>

                    <div className="analytics-stage-grid">
                      <div className="card analytics-hero-card analytics-card-span-2">
                        <div className="analytics-card-header-tight">
                          <div>
                            <span className="analytics-card-kicker">Projection</span>
                            <h2>Monte Carlo Savings Horizon</h2>
                          </div>
                          <span className="badge">
                            {monteCarloProjection ? `${monteCarloProjection.simulationCount} sims / 30 days` : "Waiting for more history"}
                          </span>
                        </div>
                        <p className="chart-summary">
                          A deterministic Monte Carlo projection built from recent daily savings. It estimates the likely additional savings band over the next 30 days so the trajectory reads as future lift, not replayed lifetime totals.
                        </p>
                        <MonteCarloProjectionChart projection={monteCarloProjection} />
                        {monteCarloProjection ? (
                          <div className="analytics-stat-strip analytics-stat-strip-tight">
                            <div className="analytics-stat-chip">
                              <span className="analytics-stat-chip-label">p10</span>
                              <strong>{formatSignedCompactNumber(Number(monteCarloProjection.summary?.p10Gain || 0))}t</strong>
                            </div>
                            <div className="analytics-stat-chip">
                              <span className="analytics-stat-chip-label">p50</span>
                              <strong>{formatSignedCompactNumber(Number(monteCarloProjection.summary?.p50Gain || 0))}t</strong>
                            </div>
                            <div className="analytics-stat-chip">
                              <span className="analytics-stat-chip-label">p90</span>
                              <strong>{formatSignedCompactNumber(Number(monteCarloProjection.summary?.p90Gain || 0))}t</strong>
                            </div>
                            <div className="analytics-stat-chip">
                              <span className="analytics-stat-chip-label">Current run-rate</span>
                              <strong>{formatCompactNumber(Number(monteCarloProjection.summary?.avgDaily || 0))}t/day</strong>
                            </div>
                          </div>
                        ) : null}
                      </div>

                      <div className="card analytics-chart-card analytics-health-card">
                        <div className="analytics-card-header-tight">
                          <div>
                            <span className="analytics-card-kicker">Live health</span>
                            <h2>Recall Quality</h2>
                          </div>
                          <span className="badge">{latestRecallHitRate || 0}%</span>
                        </div>
                        <p className="chart-summary">
                          Recall quality is tracked as a health box because the current signal is usually flat. What matters here is whether it is stable, drifting, or falling behind token savings.
                        </p>
                        <div className="analytics-stat-strip analytics-stat-strip-tight">
                          <div className="analytics-stat-chip">
                            <span className="analytics-stat-chip-label">Headline</span>
                            <strong>{latestRecallHitRate || 0}%</strong>
                          </div>
                          <div className="analytics-stat-chip">
                            <span className="analytics-stat-chip-label">7-day avg</span>
                            <strong>{recallWindowAverage || 0}%</strong>
                          </div>
                          <div className="analytics-stat-chip">
                            <span className="analytics-stat-chip-label">Spread</span>
                            <strong>{recallWindowSpread || 0} pts</strong>
                          </div>
                          <div className="analytics-stat-chip">
                            <span className="analytics-stat-chip-label">Assessment</span>
                            <strong>{recallWindowSpread <= 2 ? "Stable" : latestRecallHitRate >= 90 ? "Strong" : "Watch"}</strong>
                          </div>
                        </div>
                        {recallHeadlineUsesFallback ? (
                          <p className="analytics-inline-note">
                            Headline is pinned to the last full sample day until live recall reaches {RECALL_HEADLINE_MIN_QUERIES} queries.
                            Today&apos;s live sample is {Math.round(Number(latestRecallPoint?.hitRatePct || 0))}% on {latestRecallSampleSize} queries.
                          </p>
                        ) : null}
                        <div className="chart-legend analytics-quality-strip">
                          {recentRecallWindow.length ? recentRecallWindow.map((point) => (
                            <span key={point.date} className="chart-day">
                              <span className="chart-day-label">{(point.date || "").slice(5)}</span>
                              <span className="chart-day-value">{Math.round(Number(point.hitRatePct || 0))}%</span>
                            </span>
                          )) : (
                            <span className="sparkline-empty">Recall metrics will appear after recent boots.</span>
                          )}
                        </div>
                      </div>
                    </div>

                    <div className="overview-grid analytics-secondary-grid">
                      <div className="card analytics-chart-card">
                        <div className="analytics-card-header-tight">
                          <div>
                            <span className="analytics-card-kicker">Short-term movement</span>
                            <h2>Daily Boot Token Savings</h2>
                          </div>
                          <span className="badge">{dailySeries.length} days</span>
                        </div>
                        <Sparkline
                          data={(savings.daily || []).map(d => d.saved)}
                          width={520}
                          height={120}
                          className="sparkline-tall"
                        />
                        <div className="chart-legend">
                          {(savings.daily || []).slice(-7).map(d => (
                            <span key={d.date} className="chart-day">
                              <span className="chart-day-label">{d.date.slice(5)}</span>
                              <span className="chart-day-value">{formatCompactNumber(Number(d.saved || 0))}</span>
                            </span>
                          ))}
                        </div>
                      </div>

                      <div className="card analytics-chart-card">
                        <div className="analytics-card-header-tight">
                          <div>
                            <span className="analytics-card-kicker">System load</span>
                            <h2>Daily Boot Compilations</h2>
                          </div>
                          <span className="badge">{formatCompactNumber(throughputBoots30d)} last 30d</span>
                        </div>
                        <Sparkline
                          data={(savings.daily || []).map(d => d.boots)}
                          width={520}
                          height={120}
                          color="var(--agent-claude)"
                          className="sparkline-tall"
                        />
                        <div className="chart-legend">
                          {(savings.daily || []).slice(-7).map(d => (
                            <span key={d.date} className="chart-day">
                              <span className="chart-day-label">{d.date.slice(5)}</span>
                              <span className="chart-day-value">{d.boots}</span>
                            </span>
                          ))}
                        </div>
                      </div>
                      <div className="card analytics-chart-card">
                        <div className="analytics-card-header-tight">
                          <div>
                            <span className="analytics-card-kicker">Long-term impact</span>
                            <h2>Cumulative Savings (30d)</h2>
                          </div>
                          <span className="badge">{formatCompactNumber(cumulativeLatestTotal)}t</span>
                        </div>
                        <Sparkline
                          data={cumulativeSeries.map((point) => Number(point.savedTotal || 0))}
                          width={520}
                          height={120}
                          color="var(--green)"
                          className="sparkline-tall"
                        />
                        <div className="chart-legend">
                          {cumulativeSeries.slice(-7).map((point) => (
                            <span key={point.date || point.timestamp} className="chart-day">
                              <span className="chart-day-label">{(point.date || "").slice(5)}</span>
                              <span className="chart-day-value">{formatCompactNumber(Number(point.savedTotal || 0))}</span>
                            </span>
                          ))}
                        </div>
                      </div>
                    </div>

                    {activityHeatmap.length > 0 && (
                      <div className="card analytics-heatmap-card">
                        <div className="analytics-card-header-tight">
                          <div>
                            <span className="analytics-card-kicker">Behavioral map</span>
                            <h2>Agent Activity Heatmap</h2>
                          </div>
                          <div className="heatmap-legend-scale" aria-hidden="true">
                            <span>Low</span>
                            <span className="heatmap-legend-bar" />
                            <span>High</span>
                          </div>
                        </div>
                        <div className="activity-heatmap">
                          {["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].map((day) => (
                            <div key={day} className="activity-heatmap-row">
                              <span className="activity-heatmap-day">{day}</span>
                              <div className="activity-heatmap-cells">
                                {Array.from({ length: 24 }).map((_, hour) => {
                                  const count = activityHeatmapLookup.get(`${day}:${hour}`) || 0;
                                  const alpha = count > 0 ? clampNumber(count / activityHeatmapMax, 0.12, 1) : 0.04;
                                  return (
                                    <span
                                      key={`${day}-${hour}`}
                                      className="activity-heatmap-cell"
                                      title={`${day} ${hour.toString().padStart(2, "0")}:00 - ${count} events`}
                                      style={{ background: `linear-gradient(180deg, rgba(67, 234, 255, ${alpha}), rgba(58, 109, 255, ${alpha * 0.72}))` }}
                                    />
                                  );
                                })}
                              </div>
                            </div>
                          ))}
                        </div>
                      </div>
                    )}

                    <div className="analytics-lists-grid">
                      <div className="card analytics-list-card">
                        <div className="analytics-card-header-tight">
                          <div>
                            <span className="analytics-card-kicker">Who is creating lift (30d)</span>
                            <h2>Boot Savings by Agent (30d)</h2>
                          </div>
                          <span className="badge">{savings.byAgent?.length || 0}</span>
                        </div>
                        <ul className="item-list analytics-list">
                          {topSavingsByAgent.length ? topSavingsByAgent
                            .map((row, i) => (
                              <li key={`${row.agent}-${i}`}>
                                <div className="item-meta">
                                  <span className="item-name" style={{ color: agentColor(row.agent) }}>{row.agent}</span>
                                  <span className="memory-method">{Number(row.percent || 0)}% saved</span>
                                  <span className="muted-inline">{Number(row.boots || 0)} boots</span>
                                </div>
                                <div className="item-detail">
                                  {`${Number(row.saved || 0).toLocaleString()}t saved - ${Number(row.served || 0).toLocaleString()}t served`}
                                </div>
                              </li>
                            )) : <EmptyItem text="No per-agent savings data yet" />}
                        </ul>
                      </div>

                      <div className="card analytics-list-card">
                        <div className="analytics-card-header-tight">
                          <div>
                            <span className="analytics-card-kicker">Latest savings events</span>
                            <h2>Recent Boot Savings</h2>
                          </div>
                          <span className="badge">{savings.recent?.length || 0}</span>
                        </div>
                        <ul className="item-list analytics-list">
                          {savings.recent?.length ? savings.recent.slice(-10).reverse().map((s, i) => (
                            <li key={`${s.timestamp}-${i}`}>
                              <div className="item-meta">
                                <span className="item-name" style={{ color: agentColor(s.agent) }}>{s.agent}</span>
                                <span className="memory-method">{s.percent}% saved</span>
                                <span className="muted-inline">{timeAgo(s.timestamp)}</span>
                              </div>
                              <div className="item-detail">
                                {`boot prompt ${Number(s.served || 0).toLocaleString()}t from est. raw ${Number(s.baseline || 0).toLocaleString()}t (${Number(s.saved || 0).toLocaleString()}t saved)`}
                                {(Number(s.admitted || 0) > 0 || Number(s.rejected || 0) > 0)
                                  ? ` - capsules ${Number(s.admitted || 0)} in / ${Number(s.rejected || 0)} out`
                                  : ""}
                              </div>
                            </li>
                          )) : <EmptyItem text="No recent boot savings events yet" />}
                        </ul>
                      </div>
                    </div>
                  </div>
                ) : (
                  <div
                    id="analytics-tabpanel-operations"
                    className="analytics-mode-panel"
                    role="tabpanel"
                    aria-labelledby="analytics-tab-operations"
                    tabIndex={0}
                  >
                    <div className="analytics-explainer analytics-explainer-rich">
                      <div className="analytics-explainer-title">Operation view</div>
                      <p>Operation view breaks savings into recall, store, boot compression, and tool-call categories using local events. Use it to see where the system is earning margin, not just how much it saved overall.</p>
                    </div>
                    <div className="card analytics-operations-card">
                      <div className="analytics-card-header-tight">
                        <div>
                          <span className="analytics-card-kicker">Attribution</span>
                          <h2>Savings by Operation (30d)</h2>
                        </div>
                        <span className="badge">{operationRows.length} categories</span>
                      </div>
                      <div className="operation-bars">
                        {operationRows.length ? operationRows.map((row) => {
                          const saved = Number(row.saved || 0);
                          const served = Number(row.served || 0);
                          const baseline = Number(row.baseline || 0);
                          const width = Math.max(4, Math.round((saved / operationMaxSaved) * 100));
                          const label = SAVINGS_OPERATION_LABELS[row.operation] || row.operation;
                          return (
                            <div className="operation-bar-row" key={row.operation}>
                              <div className="operation-bar-header">
                                <span className="item-name">{label}</span>
                                <span className="muted-inline">{saved.toLocaleString()} tokens - {formatCurrency((saved * SAVINGS_USD_PER_MILLION) / 1000000)}</span>
                              </div>
                              <div className="operation-bar-track" title={`Raw ${baseline.toLocaleString()} - Compressed ${served.toLocaleString()}`}>
                                <span className="operation-bar-fill" style={{ width: `${width}%` }} />
                              </div>
                              <div className="item-detail">{`${Number(row.events || 0)} events - raw ${baseline.toLocaleString()} - compressed ${served.toLocaleString()}`}</div>
                            </div>
                          );
                        }) : <EmptyItem text="No operation breakdown data yet" />}
                      </div>
                    </div>
                  </div>
                )}
              </>
            ) : (
              <div className="card full">
                <EmptyItem text="Loading savings data..." />
              </div>
            )}
          </section>
        ) : null}
    </>
  );
}
