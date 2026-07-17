import React from "react";
import { AppIcon } from "../../ui-icons.jsx";
import { formatCompactNumber } from "../../number-format.js";
import { SAVINGS_USD_PER_MILLION, SAVINGS_HISTORY_DAYS, MISSION_METRIC_LEGEND } from "../constants.js";
import { AnimatedNumber } from "../components/AnimatedNumber.jsx";
import { EmptyItem, ListCard, StatusRows } from "../components/common.jsx";
import { AgentItem } from "../components/AgentItem.jsx";
import { TaskItem } from "../components/TaskItem.jsx";
import { LockItem } from "../components/LockItem.jsx";
import { FeedItem } from "../components/FeedItem.jsx";
import { ActivityItem } from "../components/ActivityItem.jsx";
function OverviewPanel(p) {
  const {
    panel,
    stats,
    tasks,
    locks,
    savings,
    conflictPairs,
    editorSetup,
    showMissionMetricLegend,
    setShowMissionMetricLegend,
    showMissionCompactUnits,
    setShowMissionCompactUnits,
    isSettingUpEditors,
    effectiveReducedMotion,
    invokeRef,
    changePanel,
    normalizedSessions,
    editorSetupSummary,
    memoryLoad,
    formatCurrency,
    formatMissionTokenValue,
    runRefreshAll,
    openEditorSetupWizard,
    daemonSysStatus,
    pendingTasks,
    claimedTasks,
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
    hostLabel,
    canSetupEditors,
  } = p;
  const setupTitle = invokeRef.current
      ? "Preview and register Cortex MCP in supported clients"
      : "Setup MCP requires the desktop app IPC bridge",
    metricCards = [
      ["cyan", "memory", "Memories", typeof stats.memories == "number" ? stats.memories : 0],
      ["blue", "decision", "Decisions", typeof stats.decisions == "number" ? stats.decisions : 0],
      ["purple", "event", "Events", typeof stats.events == "number" ? stats.events : 0],
      ["green", "agents", "Active Agents", normalizedSessions.length],
    ],
    summaryCards = [
      [
        "30d median gain",
        formatMissionTokenValue(Number(monteCarloProjection?.summary?.p50Gain || 0), { signed: !0 }),
        monteCarloProjection
          ? `${monteCarloProjection.simulationCount} deterministic sims`
          : "Waiting for more history",
      ],
      [
        "Boot Savings Run-Rate",
        formatMissionTokenValue(Number(monteCarloProjection?.summary?.avgDaily || 0), { perDay: !0 }),
        bootSavingsMomentum === null
          ? "Momentum pending"
          : `${bootSavingsMomentum >= 0 ? "+" : ""}${bootSavingsMomentum}% vs prior window`,
      ],
      [
        "Work in flight",
        claimedTasks.length + pendingTasks.length,
        `${claimedTasks.length} claimed / ${pendingTasks.length} pending`,
      ],
      ["Knowledge Entries", memoryLoad, `${stats.memories} memories / ${stats.decisions} decisions`],
    ],
    memoryRows = [
      { label: "Headline recall hit rate", value: `${latestRecallHitRate || 0}%` },
      { label: "7-day average", value: `${recallWindowAverage || 0}%` },
      { label: "Spread", value: `${recallWindowSpread || 0} pts` },
      { label: "Conflict pairs", value: conflictPairs.length },
    ];
  return (
    <React.Fragment>
      {panel === "overview" ? (
        <section className="panel active">
          <div className="panel-header overview-panel-header">
            <div>
              <h1>Overview</h1>
              <p className="panel-subtitle">Command center for analytics, live agent traffic, and memory quality.</p>
            </div>
            <div className="surface-actions">
              <button type="button" className="btn-sm" onClick={runRefreshAll}>
                Refresh
              </button>
              <button
                type="button"
                className="btn-sm btn-primary"
                onClick={openEditorSetupWizard}
                disabled={!canSetupEditors}
                title={setupTitle}
              >
                {isSettingUpEditors ? "Setting Up..." : invokeRef.current ? "Setup MCP" : "Setup MCP (Desktop)"}
              </button>
            </div>
          </div>
          <div className="metrics overview-metrics">
            {metricCards.map(([accent, icon, label, value]) => (
              <div key={label} className="metric" data-accent={accent}>
                <span className="metric-value">
                  <AnimatedNumber value={value} reducedMotion={effectiveReducedMotion} />
                </span>
                <span className="metric-label">{label}</span>
                <span className="metric-icon">
                  <AppIcon name={icon} />
                </span>
              </div>
            ))}
            <div className="metric" data-accent="blue">
              <span className="metric-value">{formatCompactNumber(Number(savings?.summary?.totalSaved || 0))}</span>
              <span className="metric-label">Saved Tokens (30d)</span>
              <span className="metric-icon">
                <AppIcon name="token" />
              </span>
            </div>
          </div>
          <div className="system-strip">
            <div className="sys-item">
              <span className="sys-label">DAEMON</span>
              <span className={`sys-value ${daemonSysStatus.toneClass}`}>{daemonSysStatus.daemonLabel}</span>
            </div>
            <div className="sys-item">
              <span className="sys-label">EMBEDDINGS</span>
              <span className={`sys-value ${daemonSysStatus.toneClass}`}>{daemonSysStatus.embeddingsLabel}</span>
            </div>
            <div className="sys-item">
              <span className="sys-label">HOST</span>
              <span className="sys-value">{hostLabel}</span>
            </div>
            <div className="sys-item">
              <span className="sys-label">LOCKS</span>
              <span className="sys-value">
                {locks.length}
                {" ACTIVE"}
              </span>
            </div>
            <div className="sys-item">
              <span className="sys-label">TASKS</span>
              <span className="sys-value">
                {pendingTasks.length}
                {" PENDING"}
              </span>
            </div>
            <button
              type="button"
              className={`sys-item sys-item-action ${canSetupEditors ? "" : "sys-item-disabled"}`}
              onClick={openEditorSetupWizard}
              title={setupTitle}
              disabled={!canSetupEditors}
            >
              <span className="sys-label">MCP</span>
              <span className="sys-value">
                {isSettingUpEditors
                  ? "WORKING"
                  : editorSetup
                    ? `${editorSetupSummary.registered} EDITORS`
                    : invokeRef.current
                      ? "SETUP"
                      : "DESKTOP"}
              </span>
            </button>
            <button
              type="button"
              className="sys-item sys-item-action"
              onClick={() => changePanel("memory")}
              title="Open memory health and conflict resolution"
            >
              <span className="sys-label">RECALL</span>
              <span className={`sys-value ${latestRecallHitRate >= 85 ? "sys-ok" : ""}`}>
                {latestRecallHitRate || 0}%
              </span>
            </button>
          </div>
          {editorSetupSummary.results.length ? (
            <div className="editor-setup-panel">
              <div className="editor-setup-header">
                <div>
                  <span className="editor-setup-kicker">MCP Registration</span>
                  <h2>Editor setup results</h2>
                </div>
                <span className="badge">
                  {editorSetupSummary.registered}/{editorSetupSummary.detected || editorSetupSummary.results.length}
                </span>
              </div>
              <div className="editor-setup-grid">
                {editorSetupSummary.results.map((entry) => {
                  const tone = entry.detected ? (entry.registered ? "ok" : "warn") : "idle",
                    stateLabel = entry.detected
                      ? entry.registered
                        ? "Configured"
                        : "Needs attention"
                      : "Not detected";
                  return (
                    <div key={entry.name} className={`editor-setup-item ${tone}`}>
                      <div className="editor-setup-item-head">
                        <span className="editor-setup-name">{entry.name}</span>
                        <span className="editor-setup-state">{stateLabel}</span>
                      </div>
                      <p>{entry.message || "No detail provided."}</p>
                    </div>
                  );
                })}
              </div>
            </div>
          ) : null}
          <div className="overview-dashboard-grid">
            <div className="card overview-hero-card overview-span-2">
              <div className="card-header">
                <h2>Mission Control</h2>
                <span className="badge" title={`Estimated value over the last ${SAVINGS_HISTORY_DAYS} days`}>
                  {formatCurrency(((savings?.summary?.totalSaved || 0) * SAVINGS_USD_PER_MILLION) / 1e6)}
                  {" (30d est)"}
                </span>
              </div>
              <div className="overview-hero-meta">
                <button
                  type="button"
                  className={`btn-sm ${showMissionCompactUnits ? "" : "btn-primary"}`}
                  onClick={() => setShowMissionCompactUnits((current) => !current)}
                  title={showMissionCompactUnits ? "Switch to full token counts" : "Switch to compact token units"}
                >
                  {showMissionCompactUnits ? "Show Full Numbers" : "Show Compact Units"}
                </button>
                <button
                  type="button"
                  className={`btn-sm ${showMissionMetricLegend ? "btn-primary" : ""}`}
                  aria-expanded={showMissionMetricLegend}
                  onClick={() => setShowMissionMetricLegend((current) => !current)}
                  title="Explain Mission Control metrics and unit labels"
                >
                  Metric Legend
                </button>
              </div>
              <div className="overview-unit-strip" role="group" aria-label="Mission Control unit key">
                <span className="overview-unit-strip-title">Unit key</span>
                {MISSION_METRIC_LEGEND.map((entry) => (
                  <span key={entry.abbreviation} className="overview-unit-chip">
                    <code>{entry.abbreviation}</code>
                    <span>{entry.meaning}</span>
                  </span>
                ))}
                <span className="overview-unit-chip">
                  <code>t/day</code>
                  <span>tokens per day</span>
                </span>
              </div>
              {showMissionMetricLegend ? (
                <div
                  id="mission-metric-legend"
                  className="overview-metric-legend"
                  role="dialog"
                  aria-modal="false"
                  aria-labelledby="mission-metric-legend-title"
                >
                  <div className="overview-metric-legend-title" id="mission-metric-legend-title">
                    Metric Legend
                  </div>
                  <p>
                    <strong>30d median gain</strong>
                    {" is the projected p50 token savings over the next 30 days."}
                    <strong>{" Current run-rate"}</strong>
                    {" is the projected daily token savings pace."}
                  </p>
                  <p>
                    {"Suffixes are case-sensitive: lowercase "}
                    <code>t</code>
                    {" means tokens, uppercase "}
                    <code>T</code>
                    {" means trillions."}
                  </p>
                  <ul>
                    {MISSION_METRIC_LEGEND.map((entry) => (
                      <li key={entry.abbreviation}>
                        <code>{entry.abbreviation}</code>
                        {" = "}
                        {entry.meaning}
                      </li>
                    ))}
                    <li>
                      <code>t/day</code>
                      {" = tokens per day (for example, "}
                      <code>11.8Mt/day</code>
                      {" means 11.8 million tokens/day)"}
                    </li>
                  </ul>
                </div>
              ) : null}
              <p className="chart-summary">
                Overview now behaves like a command deck instead of a spacer page: analytics, work, and memory quality
                are visible immediately.
              </p>
              <div className="overview-summary-grid">
                {summaryCards.map(([label, value, detail]) => (
                  <div key={label} className="overview-summary-card">
                    <span className="overview-summary-label">{label}</span>
                    <strong>{value}</strong>
                    <span>{detail}</span>
                  </div>
                ))}
              </div>
              <div className="overview-hero-actions">
                <button type="button" className="btn-sm btn-primary" onClick={() => changePanel("analytics")}>
                  Open Analytics
                </button>
                <button type="button" className="btn-sm" onClick={() => changePanel("brain")}>
                  Open Brain
                </button>
                <button type="button" className="btn-sm" onClick={() => changePanel("work")}>
                  Open Work
                </button>
              </div>
            </div>
            <div className={`card onboarding-readiness-card ${firstRunReadiness.tone}`}>
              <div className="card-header">
                <h2>First Run</h2>
                <span className={`readiness-badge ${firstRunReadiness.tone}`}>{firstRunReadiness.statusLabel}</span>
              </div>
              <div className="onboarding-next-action">
                <span>Next action</span>
                <strong>{firstRunReadiness.nextAction}</strong>
              </div>
              <div className="overview-status-list" aria-label="First-run readiness checklist">
                {firstRunReadiness.steps.map((step) => (
                  <div key={step.key} className="overview-status-row onboarding-step-row">
                    <span title={step.detail}>{step.label}</span>
                    <strong className={`readiness-step ${step.tone}`}>{step.state}</strong>
                  </div>
                ))}
              </div>
              <button
                type="button"
                className={`btn-sm ${firstRunReadiness.tone === "ok" ? "" : "btn-primary"}`}
                onClick={handleFirstRunAction}
                disabled={firstRunReadiness.action.disabled}
              >
                {firstRunReadiness.action.label}
              </button>
            </div>
            <div className="card overview-status-card">
              <div className="card-header">
                <h2>Memory Health</h2>
                <span className="badge">{latestRecallHitRate || 0}%</span>
              </div>
              <StatusRows rows={memoryRows} />
              <button type="button" className="btn-sm" onClick={() => changePanel("memory")}>
                Open Memory Surface
              </button>
            </div>
            <ListCard
              title="Active Agents"
              items={normalizedSessions}
              emptyText="No agents online"
              renderItem={(session) => <AgentItem key={session.sessionId || session.agent} session={session} />}
            />
            <ListCard
              title="Recent Activity"
              items={topActivityEntries}
              emptyText="No recent activity"
              renderItem={(entry) => <ActivityItem key={entry.id} entry={entry} />}
            />
            <ListCard
              title="Recent Feed"
              items={topFeedEntries}
              emptyText="No feed entries"
              renderItem={(entry) => <FeedItem key={entry.id} entry={entry} />}
            />
            <div className="card">
              <div className="card-header">
                <h2>Queue & Locks</h2>
                <span className="badge">{pendingTasks.length + locks.length}</span>
              </div>
              <div className="overview-dual-stack">
                <div>
                  <div className="overview-stack-title">Work Queue</div>
                  <ul className="item-list compact-list">
                    {recentOverviewTasks.length ? (
                      recentOverviewTasks.map((task) => <TaskItem key={task.taskId} task={task} />)
                    ) : (
                      <EmptyItem text="No active tasks" />
                    )}
                  </ul>
                </div>
                <div>
                  <div className="overview-stack-title">File Locks</div>
                  <ul className="item-list compact-list">
                    {locks.length ? (
                      locks
                        .slice(0, 4)
                        .map((lock) => <LockItem key={lock.id || `${lock.path}:${lock.agent}`} lock={lock} />)
                    ) : (
                      <EmptyItem text="No active locks" />
                    )}
                  </ul>
                </div>
              </div>
            </div>
          </div>
        </section>
      ) : null}
    </React.Fragment>
  );
}
export { OverviewPanel };
