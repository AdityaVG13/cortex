import React from "react";
import { AppIcon } from "../../ui-icons.jsx";
import {
  SAVINGS_USD_PER_MILLION,
  SAVINGS_HISTORY_DAYS,
  MISSION_METRIC_LEGEND,
} from "../constants.js";
import { AnimatedNumber } from "../components/AnimatedNumber.jsx";
import { EmptyItem } from "../components/common.jsx";
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
  return React.createElement(
    React.Fragment,
    null,
    panel === "overview"
      ? React.createElement(
          "section",
          { className: "panel active" },
          React.createElement(
            "div",
            { className: "panel-header overview-panel-header" },
            React.createElement(
              "div",
              null,
              React.createElement("h1", null, "Overview"),
              React.createElement(
                "p",
                { className: "panel-subtitle" },
                "Command center for analytics, live agent traffic, and memory quality.",
              ),
            ),
            React.createElement(
              "div",
              { className: "surface-actions" },
              React.createElement(
                "button",
                { type: "button", className: "btn-sm", onClick: runRefreshAll },
                "Refresh",
              ),
              React.createElement(
                "button",
                {
                  type: "button",
                  className: "btn-sm btn-primary",
                  onClick: openEditorSetupWizard,
                  disabled: !canSetupEditors,
                  title: invokeRef.current
                    ? "Preview and register Cortex MCP in supported clients"
                    : "Setup MCP requires the desktop app IPC bridge",
                },
                isSettingUpEditors
                  ? "Setting Up..."
                  : invokeRef.current
                    ? "Setup MCP"
                    : "Setup MCP (Desktop)",
              ),
            ),
          ),
          React.createElement(
            "div",
            { className: "metrics overview-metrics" },
            React.createElement(
              "div",
              { className: "metric", "data-accent": "cyan" },
              React.createElement(
                "span",
                { className: "metric-value" },
                React.createElement(AnimatedNumber, {
                  value: typeof stats.memories == "number" ? stats.memories : 0,
                  reducedMotion: effectiveReducedMotion,
                }),
              ),
              React.createElement(
                "span",
                { className: "metric-label" },
                "Memories",
              ),
              React.createElement(
                "span",
                { className: "metric-icon" },
                React.createElement(AppIcon, { name: "memory" }),
              ),
            ),
            React.createElement(
              "div",
              { className: "metric", "data-accent": "blue" },
              React.createElement(
                "span",
                { className: "metric-value" },
                React.createElement(AnimatedNumber, {
                  value:
                    typeof stats.decisions == "number" ? stats.decisions : 0,
                  reducedMotion: effectiveReducedMotion,
                }),
              ),
              React.createElement(
                "span",
                { className: "metric-label" },
                "Decisions",
              ),
              React.createElement(
                "span",
                { className: "metric-icon" },
                React.createElement(AppIcon, { name: "decision" }),
              ),
            ),
            React.createElement(
              "div",
              { className: "metric", "data-accent": "purple" },
              React.createElement(
                "span",
                { className: "metric-value" },
                React.createElement(AnimatedNumber, {
                  value: typeof stats.events == "number" ? stats.events : 0,
                  reducedMotion: effectiveReducedMotion,
                }),
              ),
              React.createElement(
                "span",
                { className: "metric-label" },
                "Events",
              ),
              React.createElement(
                "span",
                { className: "metric-icon" },
                React.createElement(AppIcon, { name: "event" }),
              ),
            ),
            React.createElement(
              "div",
              { className: "metric", "data-accent": "green" },
              React.createElement(
                "span",
                { className: "metric-value" },
                React.createElement(AnimatedNumber, {
                  value: normalizedSessions.length,
                  reducedMotion: effectiveReducedMotion,
                }),
              ),
              React.createElement(
                "span",
                { className: "metric-label" },
                "Active Agents",
              ),
              React.createElement(
                "span",
                { className: "metric-icon" },
                React.createElement(AppIcon, { name: "agents" }),
              ),
            ),
            React.createElement(
              "div",
              { className: "metric", "data-accent": "blue" },
              React.createElement(
                "span",
                { className: "metric-value" },
                formatCompactNumber(Number(savings?.summary?.totalSaved || 0)),
              ),
              React.createElement(
                "span",
                { className: "metric-label" },
                "Saved Tokens (30d)",
              ),
              React.createElement(
                "span",
                { className: "metric-icon" },
                React.createElement(AppIcon, { name: "token" }),
              ),
            ),
          ),
          React.createElement(
            "div",
            { className: "system-strip" },
            React.createElement(
              "div",
              { className: "sys-item" },
              React.createElement("span", { className: "sys-label" }, "DAEMON"),
              React.createElement(
                "span",
                { className: `sys-value ${daemonSysStatus.toneClass}` },
                daemonSysStatus.daemonLabel,
              ),
            ),
            React.createElement(
              "div",
              { className: "sys-item" },
              React.createElement(
                "span",
                { className: "sys-label" },
                "EMBEDDINGS",
              ),
              React.createElement(
                "span",
                { className: `sys-value ${daemonSysStatus.toneClass}` },
                daemonSysStatus.embeddingsLabel,
              ),
            ),
            React.createElement(
              "div",
              { className: "sys-item" },
              React.createElement("span", { className: "sys-label" }, "HOST"),
              React.createElement(
                "span",
                { className: "sys-value" },
                hostLabel,
              ),
            ),
            React.createElement(
              "div",
              { className: "sys-item" },
              React.createElement("span", { className: "sys-label" }, "LOCKS"),
              React.createElement(
                "span",
                { className: "sys-value" },
                locks.length,
                " ACTIVE",
              ),
            ),
            React.createElement(
              "div",
              { className: "sys-item" },
              React.createElement("span", { className: "sys-label" }, "TASKS"),
              React.createElement(
                "span",
                { className: "sys-value" },
                pendingTasks.length,
                " PENDING",
              ),
            ),
            React.createElement(
              "button",
              {
                type: "button",
                className: `sys-item sys-item-action ${canSetupEditors ? "" : "sys-item-disabled"}`,
                onClick: openEditorSetupWizard,
                title: invokeRef.current
                  ? "Preview and register Cortex MCP in supported clients"
                  : "Setup MCP requires the desktop app IPC bridge",
                disabled: !canSetupEditors,
              },
              React.createElement("span", { className: "sys-label" }, "MCP"),
              React.createElement(
                "span",
                { className: "sys-value" },
                isSettingUpEditors
                  ? "WORKING"
                  : editorSetup
                    ? `${editorSetupSummary.registered} EDITORS`
                    : invokeRef.current
                      ? "SETUP"
                      : "DESKTOP",
              ),
            ),
            React.createElement(
              "button",
              {
                type: "button",
                className: "sys-item sys-item-action",
                onClick: () => changePanel("memory"),
                title: "Open memory health and conflict resolution",
              },
              React.createElement("span", { className: "sys-label" }, "RECALL"),
              React.createElement(
                "span",
                {
                  className: `sys-value ${latestRecallHitRate >= 85 ? "sys-ok" : ""}`,
                },
                latestRecallHitRate || 0,
                "%",
              ),
            ),
          ),
          editorSetupSummary.results.length
            ? React.createElement(
                "div",
                { className: "editor-setup-panel" },
                React.createElement(
                  "div",
                  { className: "editor-setup-header" },
                  React.createElement(
                    "div",
                    null,
                    React.createElement(
                      "span",
                      { className: "editor-setup-kicker" },
                      "MCP Registration",
                    ),
                    React.createElement("h2", null, "Editor setup results"),
                  ),
                  React.createElement(
                    "span",
                    { className: "badge" },
                    editorSetupSummary.registered,
                    "/",
                    editorSetupSummary.detected ||
                      editorSetupSummary.results.length,
                  ),
                ),
                React.createElement(
                  "div",
                  { className: "editor-setup-grid" },
                  editorSetupSummary.results.map((entry) => {
                    const tone = entry.detected
                        ? entry.registered
                          ? "ok"
                          : "warn"
                        : "idle",
                      stateLabel = entry.detected
                        ? entry.registered
                          ? "Configured"
                          : "Needs attention"
                        : "Not detected";
                    return React.createElement(
                      "div",
                      {
                        key: entry.name,
                        className: `editor-setup-item ${tone}`,
                      },
                      React.createElement(
                        "div",
                        { className: "editor-setup-item-head" },
                        React.createElement(
                          "span",
                          { className: "editor-setup-name" },
                          entry.name,
                        ),
                        React.createElement(
                          "span",
                          { className: "editor-setup-state" },
                          stateLabel,
                        ),
                      ),
                      React.createElement(
                        "p",
                        null,
                        entry.message || "No detail provided.",
                      ),
                    );
                  }),
                ),
              )
            : null,
          React.createElement(
            "div",
            { className: "overview-dashboard-grid" },
            React.createElement(
              "div",
              { className: "card overview-hero-card overview-span-2" },
              React.createElement(
                "div",
                { className: "card-header" },
                React.createElement("h2", null, "Mission Control"),
                React.createElement(
                  "span",
                  {
                    className: "badge",
                    title: `Estimated value over the last ${SAVINGS_HISTORY_DAYS} days`,
                  },
                  formatCurrency(
                    ((savings?.summary?.totalSaved || 0) *
                      SAVINGS_USD_PER_MILLION) /
                      1e6,
                  ),
                  " (30d est)",
                ),
              ),
              React.createElement(
                "div",
                { className: "overview-hero-meta" },
                React.createElement(
                  "button",
                  {
                    type: "button",
                    className: `btn-sm ${showMissionCompactUnits ? "" : "btn-primary"}`,
                    onClick: () =>
                      setShowMissionCompactUnits((current) => !current),
                    title: showMissionCompactUnits
                      ? "Switch to full token counts"
                      : "Switch to compact token units",
                  },
                  showMissionCompactUnits
                    ? "Show Full Numbers"
                    : "Show Compact Units",
                ),
                React.createElement(
                  "button",
                  {
                    type: "button",
                    className: `btn-sm ${showMissionMetricLegend ? "btn-primary" : ""}`,
                    "aria-expanded": showMissionMetricLegend,
                    onClick: () =>
                      setShowMissionMetricLegend((current) => !current),
                    title: "Explain Mission Control metrics and unit labels",
                  },
                  "Metric Legend",
                ),
              ),
              React.createElement(
                "div",
                {
                  className: "overview-unit-strip",
                  role: "group",
                  "aria-label": "Mission Control unit key",
                },
                React.createElement(
                  "span",
                  { className: "overview-unit-strip-title" },
                  "Unit key",
                ),
                MISSION_METRIC_LEGEND.map((entry) =>
                  React.createElement(
                    "span",
                    {
                      key: entry.abbreviation,
                      className: "overview-unit-chip",
                    },
                    React.createElement("code", null, entry.abbreviation),
                    React.createElement("span", null, entry.meaning),
                  ),
                ),
                React.createElement(
                  "span",
                  { className: "overview-unit-chip" },
                  React.createElement("code", null, "t/day"),
                  React.createElement("span", null, "tokens per day"),
                ),
              ),
              showMissionMetricLegend
                ? React.createElement(
                    "div",
                    {
                      id: "mission-metric-legend",
                      className: "overview-metric-legend",
                      role: "dialog",
                      "aria-modal": "false",
                      "aria-labelledby": "mission-metric-legend-title",
                    },
                    React.createElement(
                      "div",
                      {
                        className: "overview-metric-legend-title",
                        id: "mission-metric-legend-title",
                      },
                      "Metric Legend",
                    ),
                    React.createElement(
                      "p",
                      null,
                      React.createElement("strong", null, "30d median gain"),
                      " is the projected p50 token savings over the next 30 days.",
                      React.createElement("strong", null, " Current run-rate"),
                      " is the projected daily token savings pace.",
                    ),
                    React.createElement(
                      "p",
                      null,
                      "Suffixes are case-sensitive: lowercase ",
                      React.createElement("code", null, "t"),
                      " means tokens, uppercase ",
                      React.createElement("code", null, "T"),
                      " means trillions.",
                    ),
                    React.createElement(
                      "ul",
                      null,
                      MISSION_METRIC_LEGEND.map((entry) =>
                        React.createElement(
                          "li",
                          { key: entry.abbreviation },
                          React.createElement("code", null, entry.abbreviation),
                          " = ",
                          entry.meaning,
                        ),
                      ),
                      React.createElement(
                        "li",
                        null,
                        React.createElement("code", null, "t/day"),
                        " = tokens per day (for example, ",
                        React.createElement("code", null, "11.8Mt/day"),
                        " means 11.8 million tokens/day)",
                      ),
                    ),
                  )
                : null,
              React.createElement(
                "p",
                { className: "chart-summary" },
                "Overview now behaves like a command deck instead of a spacer page: analytics, work, and memory quality are visible immediately.",
              ),
              React.createElement(
                "div",
                { className: "overview-summary-grid" },
                React.createElement(
                  "div",
                  { className: "overview-summary-card" },
                  React.createElement(
                    "span",
                    { className: "overview-summary-label" },
                    "30d median gain",
                  ),
                  React.createElement(
                    "strong",
                    null,
                    formatMissionTokenValue(
                      Number(monteCarloProjection?.summary?.p50Gain || 0),
                      { signed: !0 },
                    ),
                  ),
                  React.createElement(
                    "span",
                    null,
                    monteCarloProjection
                      ? `${monteCarloProjection.simulationCount} deterministic sims`
                      : "Waiting for more history",
                  ),
                ),
                React.createElement(
                  "div",
                  { className: "overview-summary-card" },
                  React.createElement(
                    "span",
                    { className: "overview-summary-label" },
                    "Boot Savings Run-Rate",
                  ),
                  React.createElement(
                    "strong",
                    null,
                    formatMissionTokenValue(
                      Number(monteCarloProjection?.summary?.avgDaily || 0),
                      { perDay: !0 },
                    ),
                  ),
                  React.createElement(
                    "span",
                    null,
                    bootSavingsMomentum === null
                      ? "Momentum pending"
                      : `${bootSavingsMomentum >= 0 ? "+" : ""}${bootSavingsMomentum}% vs prior window`,
                  ),
                ),
                React.createElement(
                  "div",
                  { className: "overview-summary-card" },
                  React.createElement(
                    "span",
                    { className: "overview-summary-label" },
                    "Work in flight",
                  ),
                  React.createElement(
                    "strong",
                    null,
                    claimedTasks.length + pendingTasks.length,
                  ),
                  React.createElement(
                    "span",
                    null,
                    claimedTasks.length,
                    " claimed / ",
                    pendingTasks.length,
                    " pending",
                  ),
                ),
                React.createElement(
                  "div",
                  { className: "overview-summary-card" },
                  React.createElement(
                    "span",
                    { className: "overview-summary-label" },
                    "Knowledge Entries",
                  ),
                  React.createElement("strong", null, memoryLoad),
                  React.createElement(
                    "span",
                    null,
                    stats.memories,
                    " memories / ",
                    stats.decisions,
                    " decisions",
                  ),
                ),
              ),
              React.createElement(
                "div",
                { className: "overview-hero-actions" },
                React.createElement(
                  "button",
                  {
                    type: "button",
                    className: "btn-sm btn-primary",
                    onClick: () => changePanel("analytics"),
                  },
                  "Open Analytics",
                ),
                React.createElement(
                  "button",
                  {
                    type: "button",
                    className: "btn-sm",
                    onClick: () => changePanel("brain"),
                  },
                  "Open Brain",
                ),
                React.createElement(
                  "button",
                  {
                    type: "button",
                    className: "btn-sm",
                    onClick: () => changePanel("work"),
                  },
                  "Open Work",
                ),
              ),
            ),
            React.createElement(
              "div",
              {
                className: `card onboarding-readiness-card ${firstRunReadiness.tone}`,
              },
              React.createElement(
                "div",
                { className: "card-header" },
                React.createElement("h2", null, "First Run"),
                React.createElement(
                  "span",
                  { className: `readiness-badge ${firstRunReadiness.tone}` },
                  firstRunReadiness.statusLabel,
                ),
              ),
              React.createElement(
                "div",
                { className: "onboarding-next-action" },
                React.createElement("span", null, "Next action"),
                React.createElement(
                  "strong",
                  null,
                  firstRunReadiness.nextAction,
                ),
              ),
              React.createElement(
                "div",
                {
                  className: "overview-status-list",
                  "aria-label": "First-run readiness checklist",
                },
                firstRunReadiness.steps.map((step) =>
                  React.createElement(
                    "div",
                    {
                      key: step.key,
                      className: "overview-status-row onboarding-step-row",
                    },
                    React.createElement(
                      "span",
                      { title: step.detail },
                      step.label,
                    ),
                    React.createElement(
                      "strong",
                      { className: `readiness-step ${step.tone}` },
                      step.state,
                    ),
                  ),
                ),
              ),
              React.createElement(
                "button",
                {
                  type: "button",
                  className: `btn-sm ${firstRunReadiness.tone === "ok" ? "" : "btn-primary"}`,
                  onClick: handleFirstRunAction,
                  disabled: firstRunReadiness.action.disabled,
                },
                firstRunReadiness.action.label,
              ),
            ),
            React.createElement(
              "div",
              { className: "card overview-status-card" },
              React.createElement(
                "div",
                { className: "card-header" },
                React.createElement("h2", null, "Memory Health"),
                React.createElement(
                  "span",
                  { className: "badge" },
                  latestRecallHitRate || 0,
                  "%",
                ),
              ),
              React.createElement(
                "div",
                { className: "overview-status-list" },
                React.createElement(
                  "div",
                  { className: "overview-status-row" },
                  React.createElement("span", null, "Headline recall hit rate"),
                  React.createElement(
                    "strong",
                    null,
                    latestRecallHitRate || 0,
                    "%",
                  ),
                ),
                React.createElement(
                  "div",
                  { className: "overview-status-row" },
                  React.createElement("span", null, "7-day average"),
                  React.createElement(
                    "strong",
                    null,
                    recallWindowAverage || 0,
                    "%",
                  ),
                ),
                React.createElement(
                  "div",
                  { className: "overview-status-row" },
                  React.createElement("span", null, "Spread"),
                  React.createElement(
                    "strong",
                    null,
                    recallWindowSpread || 0,
                    " pts",
                  ),
                ),
                React.createElement(
                  "div",
                  { className: "overview-status-row" },
                  React.createElement("span", null, "Conflict pairs"),
                  React.createElement("strong", null, conflictPairs.length),
                ),
              ),
              React.createElement(
                "button",
                {
                  type: "button",
                  className: "btn-sm",
                  onClick: () => changePanel("memory"),
                },
                "Open Memory Surface",
              ),
            ),
            React.createElement(
              "div",
              { className: "card" },
              React.createElement(
                "div",
                { className: "card-header" },
                React.createElement("h2", null, "Active Agents"),
                React.createElement(
                  "span",
                  { className: "badge" },
                  normalizedSessions.length,
                ),
              ),
              React.createElement(
                "ul",
                { className: "item-list" },
                normalizedSessions.length
                  ? normalizedSessions.map((session) =>
                      React.createElement(AgentItem, {
                        key: session.sessionId || session.agent,
                        session,
                      }),
                    )
                  : React.createElement(EmptyItem, {
                      text: "No agents online",
                    }),
              ),
            ),
            React.createElement(
              "div",
              { className: "card" },
              React.createElement(
                "div",
                { className: "card-header" },
                React.createElement("h2", null, "Recent Activity"),
                React.createElement(
                  "span",
                  { className: "badge" },
                  topActivityEntries.length,
                ),
              ),
              React.createElement(
                "ul",
                { className: "item-list" },
                topActivityEntries.length
                  ? topActivityEntries.map((entry) =>
                      React.createElement(ActivityItem, {
                        key: entry.id,
                        entry,
                      }),
                    )
                  : React.createElement(EmptyItem, {
                      text: "No recent activity",
                    }),
              ),
            ),
            React.createElement(
              "div",
              { className: "card" },
              React.createElement(
                "div",
                { className: "card-header" },
                React.createElement("h2", null, "Recent Feed"),
                React.createElement(
                  "span",
                  { className: "badge" },
                  topFeedEntries.length,
                ),
              ),
              React.createElement(
                "ul",
                { className: "item-list" },
                topFeedEntries.length
                  ? topFeedEntries.map((entry) =>
                      React.createElement(FeedItem, { key: entry.id, entry }),
                    )
                  : React.createElement(EmptyItem, { text: "No feed entries" }),
              ),
            ),
            React.createElement(
              "div",
              { className: "card" },
              React.createElement(
                "div",
                { className: "card-header" },
                React.createElement("h2", null, "Queue & Locks"),
                React.createElement(
                  "span",
                  { className: "badge" },
                  pendingTasks.length + locks.length,
                ),
              ),
              React.createElement(
                "div",
                { className: "overview-dual-stack" },
                React.createElement(
                  "div",
                  null,
                  React.createElement(
                    "div",
                    { className: "overview-stack-title" },
                    "Work Queue",
                  ),
                  React.createElement(
                    "ul",
                    { className: "item-list compact-list" },
                    recentOverviewTasks.length
                      ? recentOverviewTasks.map((task) =>
                          React.createElement(TaskItem, {
                            key: task.taskId,
                            task,
                          }),
                        )
                      : React.createElement(EmptyItem, {
                          text: "No active tasks",
                        }),
                  ),
                ),
                React.createElement(
                  "div",
                  null,
                  React.createElement(
                    "div",
                    { className: "overview-stack-title" },
                    "File Locks",
                  ),
                  React.createElement(
                    "ul",
                    { className: "item-list compact-list" },
                    locks.length
                      ? locks
                          .slice(0, 4)
                          .map((lock) =>
                            React.createElement(LockItem, {
                              key: lock.id || `${lock.path}:${lock.agent}`,
                              lock,
                            }),
                          )
                      : React.createElement(EmptyItem, {
                          text: "No active locks",
                        }),
                  ),
                ),
              ),
            ),
          ),
        )
      : null,
  );
}
export { OverviewPanel };
